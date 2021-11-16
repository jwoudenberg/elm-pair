use core::ops::Range;
use sized_stack::SizedStack;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tree_sitter::{InputEdit, Node, Tree, TreeCursor};

const MAX_COMPILATION_CANDIDATES: usize = 10;

fn main() {
    let fifo_path = "/tmp/elm-pair";
    nix::unistd::mkfifo(fifo_path, nix::sys::stat::Mode::S_IRWXU).unwrap();
    let fifo = std::fs::File::open(fifo_path).unwrap();
    let compilation_thread_state = Arc::new(CompilationThreadState {
        last_compilation_success: Mutex::new(None),
        candidates: Mutex::new(SizedStack::with_capacity(MAX_COMPILATION_CANDIDATES)),
    });
    let compilation_thread_state_clone = Arc::clone(&compilation_thread_state);
    // TODO: figure out a way to keep tabs on thread health.
    std::thread::spawn(move || {
        run_compilation_thread(compilation_thread_state_clone);
    });
    handle_events(
        compilation_thread_state,
        &mut std::io::BufReader::new(fifo).lines(),
    );
}

struct CompilationThreadState {
    last_compilation_success: Mutex<Option<SourceFileSnapshot>>,
    candidates: Mutex<SizedStack<SourceFileSnapshot>>,
}

struct SourceFileSnapshot {
    // Absolute path to this source file.
    _path: PathBuf,
    // Root of the Elm project containing this source file.
    project_root: PathBuf,
    // Absolute path to the `elm` compiler.
    elm_path: PathBuf,
    // The code in the file.
    code: SourceCode,
}

#[derive(Clone)]
struct SourceCode {
    // Vec offers a '.splice()' operation we need to replace bits of the vector
    // in response to updates made in the editor. This is probably not super
    // efficient though, so we should look for a better datastructure here.
    //
    // TODO: look into better data structures for splice-heavy workloads.
    bytes: Vec<u8>,
    // The tree at the time of the last 'checkpoint'.
    tree: Tree,
}

struct SourceFileState {
    // Absolute path to this source file.
    path: PathBuf,
    // Root of the Elm project containing this source file.
    project_root: PathBuf,
    // Absolute path to the `elm` compiler.
    elm_path: PathBuf,
    // The code at the time of the last 'checkpoint' (when the code compiled).
    checkpointed_code: SourceCode,
    // The code with latest edits applied.
    latest_code: SourceCode,
}

fn parse_event(serialized_event: &str) -> (PathBuf, Vec<u8>, InputEdit) {
    let (
        path,
        changed_code,
        start_byte,
        old_end_byte,
        new_end_byte,
        start_row,
        start_col,
        old_end_row,
        old_end_col,
        new_end_row,
        new_end_col,
    ): (
        PathBuf,
        String,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
    ) = serde_json::from_str(serialized_event).unwrap();
    let start_position = tree_sitter::Point {
        row: start_row,
        column: start_col,
    };
    let old_end_position = tree_sitter::Point {
        row: old_end_row,
        column: old_end_col,
    };
    let new_end_position = tree_sitter::Point {
        row: new_end_row,
        column: new_end_col,
    };
    let input_edit = tree_sitter::InputEdit {
        start_byte,
        old_end_byte,
        new_end_byte,
        start_position,
        old_end_position,
        new_end_position,
    };
    (path, changed_code.into_bytes(), input_edit)
}

fn handle_events<I>(compilation_thread_state: Arc<CompilationThreadState>, lines: &mut I)
where
    I: Iterator<Item = Result<String, std::io::Error>>,
{
    // First event returns the initial state.
    let first_line = match lines.next() {
        None => return, // We receive no events at all :(. Exit early.
        Some(line) => line,
    };
    let (path, bytes, _) = parse_event(&first_line.unwrap());
    let tree = parse(None, &bytes).unwrap();
    let code = SourceCode { tree, bytes };
    let mut state = SourceFileState {
        elm_path: find_executable("elm").unwrap(),
        project_root: find_project_root(&path).unwrap().to_path_buf(),
        path,
        checkpointed_code: code.clone(),
        latest_code: code,
    };
    print_latest_tree(&state);

    // Subsequent parses of a file.
    for line in lines {
        let (_, changed_lines, edit) = parse_event(&line.unwrap());
        {
            let mut last_compilation_success = compilation_thread_state
                .last_compilation_success
                .lock()
                .unwrap();

            if let Some(snapshot) = std::mem::replace(&mut *last_compilation_success, None) {
                state = new_checkpoint(state, snapshot);
            }
        }
        handle_event(&mut state, changed_lines, edit);
        // TODO: add logic to only add some candidates for compilation
        let snapshot = SourceFileSnapshot {
            // TODO: try to avoid cloning path, elm_path, and project_root.
            _path: state.path.clone(),
            elm_path: state.elm_path.clone(),
            project_root: state.project_root.clone(),
            // We clone here to create a snahshot of the code in this state, to
            // check for compilation success asynchronously while we continue
            // making edits on top of the original.
            code: state.latest_code.clone(),
        };
        {
            let mut candidates = compilation_thread_state.candidates.lock().unwrap();
            candidates.push(snapshot)
        }
    }
}

fn new_checkpoint(state: SourceFileState, snapshot: SourceFileSnapshot) -> SourceFileState {
    SourceFileState {
        checkpointed_code: snapshot.code,
        ..state
    }
}

fn run_compilation_thread(compilation_thread_state: Arc<CompilationThreadState>) {
    loop {
        if let Some(candidate) = pop_latest_candidate(&compilation_thread_state.candidates) {
            if does_latest_compile(&candidate) {
                let mut last_compilation_success = compilation_thread_state
                    .last_compilation_success
                    .lock()
                    .unwrap();
                *last_compilation_success = Some(candidate);
            }
        }
    }
}

fn pop_latest_candidate(
    candidates: &Mutex<SizedStack<SourceFileSnapshot>>,
) -> Option<SourceFileSnapshot> {
    candidates.lock().unwrap().pop()
}

fn does_latest_compile(snapshot: &SourceFileSnapshot) -> bool {
    // Write lates code to temporary file. We don't compile the original source
    // file, because the version stored on disk is likely ahead or behind the
    // version in the editor.
    let mut temp_path = snapshot.project_root.join("elm_stuff/elm-pair");
    std::fs::create_dir_all(&temp_path).unwrap();
    temp_path.push("Temp.elm");
    std::fs::write(&temp_path, &snapshot.code.bytes).unwrap();

    // Run Elm compiler against temporary file.
    let output = std::process::Command::new(&snapshot.elm_path)
        .args(["make", "--report=json", temp_path.to_str().unwrap()])
        .current_dir(&snapshot.project_root)
        .output()
        .unwrap();

    output.status.success()
}

fn find_executable(name: &str) -> Option<PathBuf> {
    let cwd = std::env::current_dir().unwrap();
    let path = std::env::var_os("PATH").unwrap();
    let dirs = std::env::split_paths(&path);
    for dir in dirs {
        let mut bin_path = cwd.join(dir);
        bin_path.push(name);
        if bin_path.is_file() {
            return Some(bin_path);
        };
    }
    None
}

fn find_project_root(source_file: &Path) -> Option<&Path> {
    let mut maybe_root = source_file;
    loop {
        match maybe_root.parent() {
            None => {
                return None;
            }
            Some(parent) => {
                if parent.join("elm.json").exists() {
                    return Some(parent);
                } else {
                    maybe_root = parent;
                }
            }
        }
    }
}

fn handle_event(state: &mut SourceFileState, changed_bytes: Vec<u8>, edit: InputEdit) {
    println!("edit: {:?}", edit);
    state.latest_code.tree.edit(&edit);
    let range = edit.start_byte..edit.old_end_byte;
    state.latest_code.bytes.splice(range, changed_bytes);

    let parse_result = parse(Some(&state.latest_code.tree), &state.latest_code.bytes);
    if let Some(new_tree) = parse_result {
        state.latest_code.tree = new_tree;
        print_latest_tree(state);
        let mut old_cursor = state.checkpointed_code.tree.walk();
        let mut new_cursor = state.latest_code.tree.walk();
        let changes = diff_trees(state, &mut old_cursor, &mut new_cursor);
        let elm_change = interpret_change(state, &changes);
        println!("CHANGE: {:?}", elm_change);
    }
}

// TODO: use kind ID's instead of names for pattern matching.
fn interpret_change(state: &SourceFileState, changes: &TreeChanges) -> Option<ElmChange> {
    match (
        attach_kinds(&changes.old_removed).as_slice(),
        attach_kinds(&changes.new_added).as_slice(),
    ) {
        ([("lower_case_identifier", before)], [("lower_case_identifier", after)]) => {
            Some(ElmChange::NameChanged(
                code_slice(&state.checkpointed_code, &before.byte_range()),
                code_slice(&state.latest_code, &after.byte_range()),
            ))
        }
        ([("upper_case_identifier", before)], [("upper_case_identifier", after)]) => {
            match before.parent().unwrap().kind() {
                "as_clause" => Some(ElmChange::AsClauseChanged(
                    code_slice(&state.checkpointed_code, &before.byte_range()),
                    code_slice(&state.latest_code, &after.byte_range()),
                )),
                _ => Some(ElmChange::TypeChanged(
                    code_slice(&state.checkpointed_code, &before.byte_range()),
                    code_slice(&state.latest_code, &after.byte_range()),
                )),
            }
        }
        ([], [("import_clause", after)]) => Some(ElmChange::ImportAdded(code_slice(
            &state.latest_code,
            &after.byte_range(),
        ))),
        ([("import_clause", before)], []) => Some(ElmChange::ImportRemoved(code_slice(
            &state.checkpointed_code,
            &before.byte_range(),
        ))),
        ([], [("type_declaration", after)]) => Some(ElmChange::TypeAdded(code_slice(
            &state.latest_code,
            &after.byte_range(),
        ))),
        ([("type_declaration", before)], []) => Some(ElmChange::TypeRemoved(code_slice(
            &state.checkpointed_code,
            &before.byte_range(),
        ))),
        ([], [("type_alias_declaration", after)]) => Some(ElmChange::TypeAliasAdded(code_slice(
            &state.latest_code,
            &after.byte_range(),
        ))),
        ([("type_alias_declaration", before)], []) => Some(ElmChange::TypeAliasRemoved(
            code_slice(&state.checkpointed_code, &before.byte_range()),
        )),
        ([], [(",", _), ("field_type", after)]) => Some(ElmChange::FieldAdded(code_slice(
            &state.latest_code,
            &after.byte_range(),
        ))),

        ([], [("field_type", after), (",", _)]) => Some(ElmChange::FieldAdded(code_slice(
            &state.latest_code,
            &after.byte_range(),
        ))),
        ([(",", _), ("field_type", before)], []) => Some(ElmChange::FieldRemoved(code_slice(
            &state.checkpointed_code,
            &before.byte_range(),
        ))),
        ([("field_type", before), (",", _)], []) => Some(ElmChange::FieldRemoved(code_slice(
            &state.checkpointed_code,
            &before.byte_range(),
        ))),
        (
            [("upper_case_identifier", qualifier), ("dot", _), ("upper_case_identifier", before)],
            [("upper_case_identifier", after)],
        ) => {
            let name_before = code_slice(&state.checkpointed_code, &before.byte_range());
            let name_after = code_slice(&state.latest_code, &after.byte_range());
            if name_before == name_after {
                Some(ElmChange::QualifierRemoved(
                    name_before,
                    code_slice(&state.checkpointed_code, &qualifier.byte_range()),
                ))
            } else {
                None
            }
        }
        (
            [("upper_case_identifier", qualifier), ("dot", _), ("lower_case_identifier", before)],
            [("lower_case_identifier", after)],
        ) => {
            let name_before = code_slice(&state.checkpointed_code, &before.byte_range());
            let name_after = code_slice(&state.latest_code, &after.byte_range());
            if name_before == name_after {
                Some(ElmChange::QualifierRemoved(
                    name_before,
                    code_slice(&state.checkpointed_code, &qualifier.byte_range()),
                ))
            } else {
                None
            }
        }
        (
            [("upper_case_identifier", before)],
            [("upper_case_identifier", qualifier), ("dot", _), ("upper_case_identifier", after)],
        ) => {
            let name_before = code_slice(&state.checkpointed_code, &before.byte_range());
            let name_after = code_slice(&state.latest_code, &after.byte_range());
            if name_before == name_after {
                Some(ElmChange::QualifierAdded(
                    name_before,
                    code_slice(&state.latest_code, &qualifier.byte_range()),
                ))
            } else {
                None
            }
        }
        (
            [("lower_case_identifier", before)],
            [("upper_case_identifier", qualifier), ("dot", _), ("lower_case_identifier", after)],
        ) => {
            let name_before = code_slice(&state.checkpointed_code, &before.byte_range());
            let name_after = code_slice(&state.latest_code, &after.byte_range());
            if name_before == name_after {
                Some(ElmChange::QualifierAdded(
                    name_before,
                    code_slice(&state.latest_code, &qualifier.byte_range()),
                ))
            } else {
                None
            }
        }
        ([("as_clause", before)], []) => Some(ElmChange::AsClauseRemoved(
            code_slice(
                &state.checkpointed_code,
                &before.prev_sibling().unwrap().byte_range(),
            ),
            code_slice(
                &state.checkpointed_code,
                &before.child_by_field_name("name").unwrap().byte_range(),
            ),
        )),
        ([], [("as_clause", after)]) => Some(ElmChange::AsClauseAdded(
            code_slice(
                &state.latest_code,
                &after.prev_sibling().unwrap().byte_range(),
            ),
            code_slice(
                &state.latest_code,
                &after.child_by_field_name("name").unwrap().byte_range(),
            ),
        )),
        (before, after) => {
            println!("NOT-MATCH BEFORE: {:?}", before);
            println!("NOT-MATCH AFTER: {:?}", after);
            None
        }
    }
}

fn attach_kinds<'a>(nodes: &'a [Node<'a>]) -> Vec<(&'a str, &'a Node<'a>)> {
    nodes.iter().map(|node| (node.kind(), node)).collect()
}

#[derive(Debug)]
enum ElmChange {
    NameChanged(String, String),
    TypeChanged(String, String),
    ImportAdded(String),
    ImportRemoved(String),
    FieldAdded(String),
    FieldRemoved(String),
    TypeAdded(String),
    TypeRemoved(String),
    TypeAliasAdded(String),
    TypeAliasRemoved(String),
    QualifierAdded(String, String),
    QualifierRemoved(String, String),
    AsClauseAdded(String, String),
    AsClauseRemoved(String, String),
    AsClauseChanged(String, String),
}

#[derive(Debug)]
struct TreeChanges<'a> {
    old_removed: Vec<Node<'a>>,
    new_added: Vec<Node<'a>>,
}

fn diff_trees<'a>(
    state: &'a SourceFileState,
    old: &'a mut TreeCursor,
    new: &'a mut TreeCursor,
) -> TreeChanges<'a> {
    loop {
        match goto_first_changed_sibling(state, old, new) {
            FirstChangedSibling::NoneFound => {
                return TreeChanges {
                    old_removed: Vec::new(),
                    new_added: Vec::new(),
                }
            }
            FirstChangedSibling::OldAtFirstAdditional => {
                return TreeChanges {
                    old_removed: collect_remaining_siblings(old),
                    new_added: Vec::new(),
                }
            }
            FirstChangedSibling::NewAtFirstAdditional => {
                return TreeChanges {
                    old_removed: Vec::new(),
                    new_added: collect_remaining_siblings(new),
                }
            }
            FirstChangedSibling::OldAndNewAtFirstChanged => {}
        };
        let first_old_changed = old.node();
        let first_new_changed = new.node();
        let (old_removed_count, new_added_count) = count_changed_siblings(state, old, new);

        // If only a single sibling changed and it's kind remained the same,
        // then we descend into that child.
        if old_removed_count == 1
            && new_added_count == 1
            && first_old_changed.kind_id() == first_new_changed.kind_id()
            && first_old_changed.child_count() > 0
            && first_new_changed.child_count() > 0
        {
            old.goto_first_child();
            new.goto_first_child();
            continue;
        }

        let mut old_removed = Vec::with_capacity(old_removed_count);
        while old_removed.len() < old_removed_count {
            old_removed.push(old.node());
            old.goto_next_sibling();
        }

        let mut new_added = Vec::with_capacity(new_added_count);
        while new_added.len() < new_added_count {
            new_added.push(new.node());
            new.goto_next_sibling();
        }

        return TreeChanges {
            old_removed,
            new_added,
        };
    }
}

// This type solely exists to list the potential results of calling
// `goto_first_changed_sibling`. The comments below each show one example of
// a set of old and new nodes that would lead to that particular value being
// returned. The arrows indicate which nodes the TreeCursors will end up
// pointing at.
enum FirstChangedSibling {
    //            v
    // old: [ a b c ]
    // new: [ a b c ]
    //            ^
    NoneFound,
    //            v
    // old: [ a b c d e ]
    // new: [ a b x y ]
    //            ^
    OldAndNewAtFirstChanged,
    //            v
    // old: [ a b c d e ]
    // new: [ a b ]
    //          ^
    OldAtFirstAdditional,
    //          v
    // old: [ a b ]
    // new: [ a b c d e ]
    //            ^
    NewAtFirstAdditional,
}

// Move both cursors forward through sibbling nodes in lock step, stopping when
// we encounter a difference between the old and new node.
fn goto_first_changed_sibling(
    state: &SourceFileState,
    old: &mut TreeCursor,
    new: &mut TreeCursor,
) -> FirstChangedSibling {
    loop {
        if has_node_changed(state, &old.node(), &new.node()) {
            return FirstChangedSibling::OldAndNewAtFirstChanged;
        } else {
            match (old.goto_next_sibling(), new.goto_next_sibling()) {
                (true, true) => continue,
                (false, false) => return FirstChangedSibling::NoneFound,
                (true, false) => return FirstChangedSibling::OldAtFirstAdditional,
                (false, true) => return FirstChangedSibling::NewAtFirstAdditional,
            }
        }
    }
}

fn collect_remaining_siblings<'a>(cursor: &'a mut TreeCursor) -> Vec<Node<'a>> {
    let mut acc = vec![cursor.node()];
    while cursor.goto_next_sibling() {
        acc.push(cursor.node());
    }
    acc
}

// Find how many old siblings were replaced with how many new ones. For example,
// given the following examples:
//
//     old: [ a b c d e f g h ]
//            |         | | |
//     new: [ a x y     f g h ]
//
// This function would return `(4, 2)` when passed the old and new sibling nodes
// of this example, because 4 of the old sibling nodes were replaced with 2 new
// ones.
//
// We go about finding these counts by skipping to the end, then counting the
// amount of equal nodes we encounter as we move backwards from the last old
// and new node in lock step. We do this because on average it's less work to
// proof two node are the same than it is to proof they are different. By
// walking backwards we only need to proof two nodes are different ones.
fn count_changed_siblings<'a>(
    state: &'a SourceFileState,
    old: &'a TreeCursor,
    new: &'a TreeCursor,
) -> (usize, usize) {
    let mut old_sibling = old.node();
    let mut new_sibling = new.node();

    // We initialize the counts at 1, because we assume the node we're currenly
    // on is the first changed node.
    let mut old_siblings_removed = 1;
    let mut new_siblings_added = 1;

    // Walk forward, basically counting all remaining old and new siblings.
    loop {
        match old_sibling.next_sibling() {
            None => break,
            Some(next) => {
                old_siblings_removed += 1;
                old_sibling = next;
            }
        }
    }
    loop {
        match new_sibling.next_sibling() {
            None => break,
            Some(next) => {
                new_siblings_added += 1;
                new_sibling = next;
            }
        }
    }

    // Walk backwards again until we encounter a changed node.
    loop {
        if has_node_changed(state, &old_sibling, &new_sibling)
            || old_siblings_removed == 0
            || new_siblings_added == 0
        {
            break;
        }
        match (old_sibling.prev_sibling(), new_sibling.prev_sibling()) {
            (Some(next_old), Some(next_new)) => {
                old_sibling = next_old;
                new_sibling = next_new;
                old_siblings_removed -= 1;
                new_siblings_added -= 1;
            }
            (_, _) => {
                break;
            }
        }
    }

    (old_siblings_removed, new_siblings_added)
}

// Check if a node has changed. We have a couple of cheap checks that can
// confirm the node _hasn't_ changed, so we try those first.
//
// TODO: Incorporate tree-sitter's `has_changes` in here somehow, for bettter
// performance.
fn has_node_changed(state: &SourceFileState, old: &Node, new: &Node) -> bool {
    old.id() != new.id()
        && (old.kind_id() != new.kind_id() || have_node_contents_changed(state, old, new))
}

// Compare two nodes by comparing snippets of code covered by them. This is
// supposed to be a 100% accurate, albeit potentially slower equivalency check.
//
// TODO: code formatters can change code in ways that don't matter but would
// fail this check. Consider alternative approaches.
// TODO: compare u8 array slices here instead of parsing to string.
fn have_node_contents_changed(state: &SourceFileState, old: &Node, new: &Node) -> bool {
    let old_bytes = code_slice(&state.checkpointed_code, &old.byte_range());
    let new_bytes = code_slice(&state.latest_code, &new.byte_range());
    old_bytes != new_bytes
}

fn code_slice(code: &SourceCode, range: &Range<usize>) -> String {
    std::string::String::from_utf8(code.bytes[range.start..range.end].to_vec()).unwrap()
}

// TODO: reuse parser.
fn parse(prev_tree: Option<&Tree>, code: &[u8]) -> Option<Tree> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(tree_sitter_elm::language())
        .expect("Error loading elm grammer");
    parser.parse(code, prev_tree)
}

fn print_latest_tree(state: &SourceFileState) {
    let mut cursor = state.latest_code.tree.walk();
    print_tree_helper(&state.latest_code, 0, &mut cursor);
    println!();
}

fn print_tree_helper(code: &SourceCode, indent: usize, cursor: &mut tree_sitter::TreeCursor) {
    let node = cursor.node();
    println!(
        "{}[{} {:?}] {:?}{}",
        "  ".repeat(indent),
        node.kind(),
        node.start_position().row + 1,
        code_slice(code, &node.byte_range()),
        if node.has_changes() { " (changed)" } else { "" },
    );
    if cursor.goto_first_child() {
        print_tree_helper(code, indent + 1, cursor);
        cursor.goto_parent();
    }
    if cursor.goto_next_sibling() {
        print_tree_helper(code, indent, cursor);
    }
}

// A stack with a maximum size. If a push would ever make the stack grow beyond
// its capacity, then the stack forgets its oldest element before pushing the
// new element.
mod sized_stack {
    use std::collections::VecDeque;

    pub struct SizedStack<T> {
        capacity: usize,
        items: VecDeque<T>,
    }

    impl<T> SizedStack<T> {
        pub fn with_capacity(capacity: usize) -> SizedStack<T> {
            SizedStack {
                capacity,
                items: VecDeque::with_capacity(capacity),
            }
        }

        pub fn push(&mut self, item: T) {
            self.items.truncate(self.capacity - 1);
            self.items.push_front(item);
        }

        pub fn pop(&mut self) -> Option<T> {
            self.items.pop_front()
        }
    }
}
