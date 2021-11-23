use core::ops::Range;
use ropey::Rope;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::{Arc, Mutex};
use tree_sitter::{InputEdit, Node, Tree, TreeCursor};

const MAX_COMPILATION_CANDIDATES: usize = 10;

pub fn main() {
    std::process::exit(match run() {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("error: {:?}", err);
            1
        }
    });
}

fn run() -> Result<(), Error> {
    let (requester, processor) = validation::channel(MAX_COMPILATION_CANDIDATES);
    // We're using a sync_channel of size 1 here because:
    // - Size 0 would mean sending blocks until the receiver is requesting a new
    //   message. This is not okay, because we want to be able to queue up a
    //   Msg::CompilationSucceeded for after current reparsing succeeds.
    // - Size >1 means we start buffering editor events in this program. I want
    //   to try to stay ouf the buffering business and leave it to the OS, until
    //   I see a benefit in doing it in this program (I don't currently).
    let (sender, receiver) = sync_channel(1);
    let sender_clone = sender.clone();
    let thread_error = Arc::new(Mutex::new(None));
    let thread_error_clone1 = thread_error.clone();
    let thread_error_clone2 = thread_error.clone();
    std::thread::spawn(move || {
        report_error(
            thread_error_clone1,
            run_compilation_thread(&sender_clone, processor),
        );
    });
    std::thread::spawn(move || {
        let log_change = |elm_change| println!("CHANGE: {:?}", elm_change);
        report_error(
            thread_error_clone2,
            run_change_analysis_thread(requester, &mut receiver.iter(), log_change),
        );
    });
    run_editor_listener_thread(thread_error, &sender)
}

#[derive(Clone)]
struct SourceFileSnapshot {
    // Once calculated the file_data never changes. We wrap it in an `Arc` to
    // avoid needing to copy it.
    file_data: Arc<FileData>,
    // The full contents of the file, stored in a Rope datastructure. This
    // datastructure offers cheap modifications in random locations, and cheap
    // cloning (both of which we'll do a lot).
    bytes: Rope,
    // The tree-sitter concrete syntax tree representing the code in `bytes`.
    // This tree by itself is not enough to recover the source code, which is
    // why we also keep the original source code in `bytes`.
    tree: Tree,
    // A number that gets incremented for each change to this snapshot.
    revision: usize,
}

impl crate::validation::Revision for SourceFileSnapshot {
    fn revision(&self) -> usize {
        self.revision
    }
}

struct FileData {
    // Absolute path to this source file.
    _path: PathBuf,
    // Root of the Elm project containing this source file.
    project_root: PathBuf,
    // Absolute path to the `elm` compiler.
    elm_bin: PathBuf,
}

// The event type central to this application. The main thread of the program
// will process these one-at-a-time.
enum Msg {
    ReceivedEditorEvent(Edit),
    CompilationSucceeded,
}

// A change made by the user reported by the editor.
struct Edit {
    // The file that was changed.
    file: PathBuf,
    // A tree-sitter InputEdit value, describing what part of the file was changed.
    input_edit: InputEdit,
    // Bytes representing the new contents of the file at the location described
    // by `input_edit`.
    new_bytes: String,
}

#[derive(Debug)]
enum Error {
    DidNotFindElmBinaryOnPath,
    CouldNotReadCurrentWorkingDirectory(std::io::Error),
    DidNotFindPathEnvVar,
    NoElmJsonFoundInAnyAncestorDirectoryOf(PathBuf),
    FifoCreationFailed(nix::errno::Errno),
    FifoOpeningFailed(std::io::Error),
    FifoLineReadingFailed(std::io::Error),
    CompilationFailedToCreateTempDir(std::io::Error),
    CompilationFailedToWriteCodeToTempFile(std::io::Error),
    CompilationFailedToRunElmMake(std::io::Error),
    SendingMsgFromEditorListenerThreadFailed,
    SendingMsgFromCompilationThreadFailed,
    JsonParsingEditorEventFailed(serde_json::error::Error),
    TreeSitterParsingFailed,
    TreeSitterSettingLanguageFailed(tree_sitter::LanguageError),
}

fn parse_editor_event(serialized_event: &str) -> Result<Edit, Error> {
    let (
        file,
        new_bytes,
        start_byte,
        old_end_byte,
        new_end_byte,
        start_row,
        start_col,
        old_end_row,
        old_end_col,
        new_end_row,
        new_end_col,
    ) = serde_json::from_str(serialized_event).map_err(Error::JsonParsingEditorEventFailed)?;
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
    Ok(Edit {
        file,
        input_edit,
        new_bytes,
    })
}

fn run_change_analysis_thread<I, F>(
    mut validator: validation::Requester<SourceFileSnapshot>,
    msgs: &mut I,
    mut on_change: F,
) -> Result<(), Error>
where
    I: Iterator<Item = Msg>,
    F: FnMut(Option<ElmChange>),
{
    let mut latest_code = None;
    let mut last_compiling_version = None;
    for msg in msgs {
        handle_msg(&mut validator, &mut latest_code, msg)?;
        validator.update_last_valid(&mut last_compiling_version);
        if let (Some(latest_code), Some(last_compiling_version)) =
            (&mut latest_code, &mut last_compiling_version)
        {
            let elm_change = analyze_changes(latest_code, last_compiling_version)?;
            on_change(elm_change);
        }
    }
    Ok(())
}

fn handle_msg(
    validator: &mut validation::Requester<SourceFileSnapshot>,
    latest_code: &mut Option<SourceFileSnapshot>,
    msg: Msg,
) -> Result<(), Error> {
    // Edit the old tree-sitter tree, or create it if we don't have one yet for
    // this file.
    match msg {
        Msg::CompilationSucceeded => {}
        Msg::ReceivedEditorEvent(edit) => match latest_code {
            None => get_initial_snapshot_from_first_edit(latest_code, edit)?,
            Some(code) => apply_edit(code, edit),
        },
    }

    let latest_code = match latest_code {
        None => return Ok(()),
        Some(code) => code,
    };

    // Update the tree-sitter syntax three. Note: this is a separate step from
    // editing the old tree. See the tree-sitter docs on parsing for more info.
    reparse_tree(latest_code)?;
    if !latest_code.tree.root_node().has_error() {
        validator.request_validation(latest_code.clone());
    }
    Ok(())
}

fn analyze_changes(
    latest_code: &mut SourceFileSnapshot,
    last_compiling_version: &mut SourceFileSnapshot,
) -> Result<Option<ElmChange>, Error> {
    let tree_changes = diff_trees(last_compiling_version, latest_code);
    let elm_change = interpret_change(&tree_changes);
    Ok(elm_change)
}

fn get_initial_snapshot_from_first_edit(
    code: &mut Option<SourceFileSnapshot>,
    Edit {
        file, new_bytes, ..
    }: Edit,
) -> Result<(), Error> {
    let bytes = Rope::from_str(&new_bytes);
    let tree = parse(None, &bytes)?;
    let file_data = Arc::new(FileData {
        elm_bin: find_executable("elm")?,
        project_root: find_project_root(&file)?.to_path_buf(),
        _path: file,
    });
    *code = Some(SourceFileSnapshot {
        tree,
        bytes,
        file_data,
        revision: 0,
    });
    Ok(())
}

fn report_error<I>(thread_error: Arc<Mutex<Option<Error>>>, result: Result<I, Error>) {
    match result {
        Ok(_) => {}
        Err(err) => {
            let mut error = thread_error.lock().unwrap();
            *error = Some(err);
        }
    }
}

fn run_compilation_thread(
    sender: &SyncSender<Msg>,
    mut validation_processor: validation::Validator<SourceFileSnapshot>,
) -> Result<(), Error> {
    loop {
        let candidate = validation_processor.next();
        if does_snapshot_compile(&candidate)? {
            validation_processor.mark_valid(candidate);
            // Let the main thread know it should reparse. If sending this fails
            // we asume that's because a ReceivedEditorEvent got there first.
            // That's okay, because those events cause reparsing too.
            sender
                .try_send(Msg::CompilationSucceeded)
                .map_err(|_| Error::SendingMsgFromCompilationThreadFailed)?;
        }
    }
}

fn run_editor_listener_thread(
    thread_error: Arc<Mutex<Option<Error>>>,
    sender: &SyncSender<Msg>,
) -> Result<(), Error> {
    let fifo_path = "/tmp/elm-pair";
    nix::unistd::mkfifo(fifo_path, nix::sys::stat::Mode::S_IRWXU)
        .map_err(Error::FifoCreationFailed)?;
    let fifo = std::fs::File::open(fifo_path).map_err(Error::FifoOpeningFailed)?;
    let buf_reader = std::io::BufReader::new(fifo).lines();
    for line in buf_reader {
        check_for_thread_error(&thread_error)?;
        let edit = parse_editor_event(&line.map_err(Error::FifoLineReadingFailed)?)?;
        sender
            .send(Msg::ReceivedEditorEvent(edit))
            .map_err(|_| Error::SendingMsgFromEditorListenerThreadFailed)?;
    }
    Ok(())
}

fn check_for_thread_error(thread_error: &Arc<Mutex<Option<Error>>>) -> Result<(), Error> {
    let mut opt_error = thread_error.lock().unwrap();
    if let Some(error) = std::mem::replace(&mut *opt_error, None) {
        return Err(error);
    }
    Ok(())
}

fn does_snapshot_compile(snapshot: &SourceFileSnapshot) -> Result<bool, Error> {
    // Write lates code to temporary file. We don't compile the original source
    // file, because the version stored on disk is likely ahead or behind the
    // version in the editor.
    let mut temp_path = snapshot.file_data.project_root.join("elm-stuff/elm-pair");
    std::fs::create_dir_all(&temp_path).map_err(Error::CompilationFailedToCreateTempDir)?;
    temp_path.push("Temp.elm");
    std::fs::write(&temp_path, &snapshot.bytes.bytes().collect::<Vec<u8>>())
        .map_err(Error::CompilationFailedToWriteCodeToTempFile)?;

    // Run Elm compiler against temporary file.
    let output = std::process::Command::new(&snapshot.file_data.elm_bin)
        .arg("make")
        .arg("--report=json")
        .arg(temp_path)
        .current_dir(&snapshot.file_data.project_root)
        .output()
        .map_err(Error::CompilationFailedToRunElmMake)?;

    Ok(output.status.success())
}

fn find_executable(name: &str) -> Result<PathBuf, Error> {
    let cwd = std::env::current_dir().map_err(Error::CouldNotReadCurrentWorkingDirectory)?;
    let path = std::env::var_os("PATH").ok_or(Error::DidNotFindPathEnvVar)?;
    let dirs = std::env::split_paths(&path);
    for dir in dirs {
        let mut bin_path = cwd.join(dir);
        bin_path.push(name);
        if bin_path.is_file() {
            return Ok(bin_path);
        };
    }
    Err(Error::DidNotFindElmBinaryOnPath)
}

fn find_project_root(source_file: &Path) -> Result<&Path, Error> {
    let mut maybe_root = source_file;
    loop {
        match maybe_root.parent() {
            None => {
                return Err(Error::NoElmJsonFoundInAnyAncestorDirectoryOf(
                    source_file.to_path_buf(),
                ));
            }
            Some(parent) => {
                if parent.join("elm.json").exists() {
                    return Ok(parent);
                } else {
                    maybe_root = parent;
                }
            }
        }
    }
}

fn apply_edit(code: &mut SourceFileSnapshot, edit: Edit) {
    code.tree.edit(&edit.input_edit);
    let bytes = &mut code.bytes;
    let start = bytes.byte_to_char(edit.input_edit.start_byte);
    let end = bytes.byte_to_char(edit.input_edit.old_end_byte);
    bytes.remove(start..end);
    bytes.insert(start, &edit.new_bytes);
    code.revision += 1;
}

fn reparse_tree(code: &mut SourceFileSnapshot) -> Result<(), Error> {
    let new_tree = parse(Some(&code.tree), &code.bytes)?;
    code.tree = new_tree;
    Ok(())
}

// TODO: use kind ID's instead of names for pattern matching.
fn interpret_change(changes: &TreeChanges) -> Option<ElmChange> {
    match (
        attach_kinds(&changes.old_removed).as_slice(),
        attach_kinds(&changes.new_added).as_slice(),
    ) {
        ([("lower_case_identifier", before)], [("lower_case_identifier", after)]) => {
            Some(ElmChange::NameChanged(
                debug_code_slice(changes.old_code, &before.byte_range()),
                debug_code_slice(changes.new_code, &after.byte_range()),
            ))
        }
        ([("upper_case_identifier", before)], [("upper_case_identifier", after)]) => {
            match before.parent()?.kind() {
                "as_clause" => Some(ElmChange::AsClauseChanged(
                    debug_code_slice(changes.old_code, &before.byte_range()),
                    debug_code_slice(changes.new_code, &after.byte_range()),
                )),
                _ => Some(ElmChange::TypeChanged(
                    debug_code_slice(changes.old_code, &before.byte_range()),
                    debug_code_slice(changes.new_code, &after.byte_range()),
                )),
            }
        }
        ([], [("import_clause", after)]) => Some(ElmChange::ImportAdded(debug_code_slice(
            changes.new_code,
            &after.byte_range(),
        ))),
        ([("import_clause", before)], []) => Some(ElmChange::ImportRemoved(debug_code_slice(
            changes.old_code,
            &before.byte_range(),
        ))),
        ([], [("type_declaration", after)]) => Some(ElmChange::TypeAdded(debug_code_slice(
            changes.new_code,
            &after.byte_range(),
        ))),
        ([("type_declaration", before)], []) => Some(ElmChange::TypeRemoved(debug_code_slice(
            changes.old_code,
            &before.byte_range(),
        ))),
        ([], [("type_alias_declaration", after)]) => Some(ElmChange::TypeAliasAdded(
            debug_code_slice(changes.new_code, &after.byte_range()),
        )),
        ([("type_alias_declaration", before)], []) => Some(ElmChange::TypeAliasRemoved(
            debug_code_slice(changes.old_code, &before.byte_range()),
        )),
        ([], [("field_type", after)]) => Some(ElmChange::FieldAdded(debug_code_slice(
            changes.new_code,
            &after.byte_range(),
        ))),
        ([], [(",", _), ("field_type", after)]) => Some(ElmChange::FieldAdded(debug_code_slice(
            changes.new_code,
            &after.byte_range(),
        ))),
        ([], [("field_type", after), (",", _)]) => Some(ElmChange::FieldAdded(debug_code_slice(
            changes.new_code,
            &after.byte_range(),
        ))),
        ([("field_type", before)], []) => Some(ElmChange::FieldRemoved(debug_code_slice(
            changes.old_code,
            &before.byte_range(),
        ))),
        ([(",", _), ("field_type", before)], []) => Some(ElmChange::FieldRemoved(
            debug_code_slice(changes.old_code, &before.byte_range()),
        )),
        ([("field_type", before), (",", _)], []) => Some(ElmChange::FieldRemoved(
            debug_code_slice(changes.old_code, &before.byte_range()),
        )),
        (
            [("upper_case_identifier", qualifier), ("dot", _), ("upper_case_identifier", before)],
            [("upper_case_identifier", after)],
        ) => {
            let name_before = debug_code_slice(changes.old_code, &before.byte_range());
            let name_after = debug_code_slice(changes.new_code, &after.byte_range());
            if name_before == name_after {
                Some(ElmChange::QualifierRemoved(
                    name_before,
                    debug_code_slice(changes.old_code, &qualifier.byte_range()),
                ))
            } else {
                None
            }
        }
        (
            [("upper_case_identifier", qualifier), ("dot", _), ("lower_case_identifier", before)],
            [("lower_case_identifier", after)],
        ) => {
            let name_before = debug_code_slice(changes.old_code, &before.byte_range());
            let name_after = debug_code_slice(changes.new_code, &after.byte_range());
            if name_before == name_after {
                Some(ElmChange::QualifierRemoved(
                    name_before,
                    debug_code_slice(changes.old_code, &qualifier.byte_range()),
                ))
            } else {
                None
            }
        }
        (
            [("upper_case_identifier", before)],
            [("upper_case_identifier", qualifier), ("dot", _), ("upper_case_identifier", after)],
        ) => {
            let name_before = debug_code_slice(changes.old_code, &before.byte_range());
            let name_after = debug_code_slice(changes.new_code, &after.byte_range());
            if name_before == name_after {
                Some(ElmChange::QualifierAdded(
                    name_before,
                    debug_code_slice(changes.new_code, &qualifier.byte_range()),
                ))
            } else {
                None
            }
        }
        (
            [("lower_case_identifier", before)],
            [("upper_case_identifier", qualifier), ("dot", _), ("lower_case_identifier", after)],
        ) => {
            let name_before = debug_code_slice(changes.old_code, &before.byte_range());
            let name_after = debug_code_slice(changes.new_code, &after.byte_range());
            if name_before == name_after {
                Some(ElmChange::QualifierAdded(
                    name_before,
                    debug_code_slice(changes.new_code, &qualifier.byte_range()),
                ))
            } else {
                None
            }
        }
        ([("as_clause", before)], []) => Some(ElmChange::AsClauseRemoved(
            debug_code_slice(changes.old_code, &before.prev_sibling()?.byte_range()),
            debug_code_slice(
                changes.old_code,
                &before.child_by_field_name("name")?.byte_range(),
            ),
        )),
        ([], [("as_clause", after)]) => Some(ElmChange::AsClauseAdded(
            debug_code_slice(changes.new_code, &after.prev_sibling()?.byte_range()),
            debug_code_slice(
                changes.new_code,
                &after.child_by_field_name("name")?.byte_range(),
            ),
        )),
        _ => {
            debug_print_tree_changes(changes);
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

struct TreeChanges<'a> {
    old_code: &'a SourceFileSnapshot,
    new_code: &'a SourceFileSnapshot,
    old_removed: Vec<Node<'a>>,
    new_added: Vec<Node<'a>>,
}

fn diff_trees<'a>(
    old_code: &'a SourceFileSnapshot,
    new_code: &'a SourceFileSnapshot,
) -> TreeChanges<'a> {
    let mut old = old_code.tree.walk();
    let mut new = new_code.tree.walk();
    loop {
        match goto_first_changed_sibling(old_code, new_code, &mut old, &mut new) {
            FirstChangedSibling::NoneFound => {
                return TreeChanges {
                    old_code,
                    new_code,
                    old_removed: Vec::new(),
                    new_added: Vec::new(),
                }
            }
            FirstChangedSibling::OldAtFirstAdditional => {
                return TreeChanges {
                    old_code,
                    new_code,
                    old_removed: collect_remaining_siblings(old),
                    new_added: Vec::new(),
                }
            }
            FirstChangedSibling::NewAtFirstAdditional => {
                return TreeChanges {
                    old_code,
                    new_code,
                    old_removed: Vec::new(),
                    new_added: collect_remaining_siblings(new),
                }
            }
            FirstChangedSibling::OldAndNewAtFirstChanged => {}
        };
        let first_old_changed = old.node();
        let first_new_changed = new.node();
        let (old_removed_count, new_added_count) =
            count_changed_siblings(old_code, new_code, &old, &new);

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
            old_code,
            new_code,
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
    old_code: &SourceFileSnapshot,
    new_code: &SourceFileSnapshot,
    old: &mut TreeCursor,
    new: &mut TreeCursor,
) -> FirstChangedSibling {
    loop {
        if has_node_changed(old_code, new_code, &old.node(), &new.node()) {
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

fn collect_remaining_siblings(mut cursor: TreeCursor) -> Vec<Node> {
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
    old_code: &'a SourceFileSnapshot,
    new_code: &'a SourceFileSnapshot,
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
        if has_node_changed(old_code, new_code, &old_sibling, &new_sibling)
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
fn has_node_changed(
    old_code: &SourceFileSnapshot,
    new_code: &SourceFileSnapshot,
    old: &Node,
    new: &Node,
) -> bool {
    old.id() != new.id()
        && (old.kind_id() != new.kind_id()
            || have_node_contents_changed(old_code, new_code, old, new))
}

// Compare two nodes by comparing snippets of code covered by them. This is
// supposed to be a 100% accurate, albeit potentially slower equivalency check.
//
// TODO: code formatters can change code in ways that don't matter but would
// fail this check. Consider alternative approaches.
fn have_node_contents_changed(
    old_code: &SourceFileSnapshot,
    new_code: &SourceFileSnapshot,
    old: &Node,
    new: &Node,
) -> bool {
    let old_bytes = debug_code_slice(old_code, &old.byte_range());
    let new_bytes = debug_code_slice(new_code, &new.byte_range());
    old_bytes != new_bytes
}

fn debug_code_slice(code: &SourceFileSnapshot, range: &Range<usize>) -> String {
    let start = code.bytes.byte_to_char(range.start);
    let end = code.bytes.byte_to_char(range.end);
    code.bytes.slice(start..end).to_string()
}

// TODO: reuse parser.
fn parse(prev_tree: Option<&Tree>, code: &Rope) -> Result<Tree, Error> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(tree_sitter_elm::language())
        .map_err(Error::TreeSitterSettingLanguageFailed)?;
    match parser.parse(code.bytes().collect::<Vec<u8>>(), prev_tree) {
        None => Err(Error::TreeSitterParsingFailed),
        Some(tree) => Ok(tree),
    }
}

// TODO: remove debug helper when it's no longer needed.
#[allow(dead_code)]
fn debug_print_code(code: &SourceFileSnapshot) {
    println!("CODE:\n{}", code.bytes.to_string());
}

// TODO: remove debug helper when it's no longer needed.
#[allow(dead_code)]
fn debug_print_tree(code: &SourceFileSnapshot) {
    let mut cursor = code.tree.walk();
    debug_print_tree_helper(code, 0, &mut cursor);
    println!();
}

// TODO: remove debug helper when it's no longer needed.
#[allow(dead_code)]
fn debug_print_tree_changes(changes: &TreeChanges) {
    println!("REMOVED NODES:");
    for node in &changes.old_removed {
        debug_print_node(changes.old_code, 2, node);
    }
    println!("ADDED NODES:");
    for node in &changes.new_added {
        debug_print_node(changes.new_code, 2, node);
    }
}

fn debug_print_tree_helper(
    code: &SourceFileSnapshot,
    indent: usize,
    cursor: &mut tree_sitter::TreeCursor,
) {
    let node = cursor.node();
    debug_print_node(code, indent, &node);
    if cursor.goto_first_child() {
        debug_print_tree_helper(code, indent + 1, cursor);
        cursor.goto_parent();
    }
    if cursor.goto_next_sibling() {
        debug_print_tree_helper(code, indent, cursor);
    }
}

fn debug_print_node(code: &SourceFileSnapshot, indent: usize, node: &Node) {
    println!(
        "{}[{} {:?}] {:?}{}",
        "  ".repeat(indent),
        node.kind(),
        node.start_position().row + 1,
        debug_code_slice(code, &node.byte_range()),
        if node.has_changes() { " (changed)" } else { "" },
    );
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

// A concurency primitive for coordinating between a validator thread and a
// thread requesting validations.
// - There's a maximum amount of inflight validation requests. If we push an
//   additional request once at the limit, the oldest request is forgotten.
// - We only store the last valid value. If we validate another value before the
//   last one is read that old value is discarded.
mod validation {
    use crate::sized_stack::SizedStack;
    use std::sync::{Arc, Condvar, Mutex, MutexGuard};

    pub struct Requester<T> {
        shared_state: Arc<SharedState<T>>,
        last_submitted_revision: Option<usize>,
    }

    impl<T> Requester<T>
    where
        T: Revision,
    {
        pub fn request_validation(&mut self, request: T) {
            if !is_new_revision(&mut self.last_submitted_revision, &request) {
                return;
            }
            let mut requests = lock(&self.shared_state.requests);
            requests.push(request);
            self.shared_state.new_request_condvar.notify_all();
        }

        pub fn update_last_valid(&self, response_var: &mut Option<T>) {
            let mut last_validated = lock(&self.shared_state.last_validated);
            if let Some(response) = std::mem::replace(&mut *last_validated, None) {
                *response_var = Some(response);
            }
        }
    }

    pub struct Validator<T> {
        shared_state: Arc<SharedState<T>>,
        last_validated_revision: Option<usize>,
    }

    impl<T> Validator<T> {
        pub fn next(&mut self) -> T
        where
            T: Revision,
        {
            let mut requests = lock(&self.shared_state.requests);
            loop {
                match requests.pop() {
                    None => {
                        requests = self
                            .shared_state
                            .new_request_condvar
                            .wait(requests)
                            .unwrap(); // See comment on `lock` function below.
                    }
                    Some(next_request) => {
                        if is_new_revision(&mut self.last_validated_revision, &next_request) {
                            return next_request;
                        } else {
                            continue;
                        }
                    }
                }
            }
        }

        pub fn mark_valid(&mut self, valid: T) {
            let mut last_validated = lock(&self.shared_state.last_validated);
            *last_validated = Some(valid);
        }
    }

    struct SharedState<T> {
        // A stack of requests for the responder to handle.
        requests: Mutex<SizedStack<T>>,
        // A condvar triggered whenever a new request arrives.
        new_request_condvar: Condvar,
        // The last calculated response.
        last_validated: Mutex<Option<T>>,
    }

    pub trait Revision {
        fn revision(&self) -> usize;
    }

    pub fn channel<T>(max_inflight_requests: usize) -> (Requester<T>, Validator<T>) {
        let shared_state = Arc::new(SharedState {
            requests: Mutex::new(SizedStack::with_capacity(max_inflight_requests)),
            new_request_condvar: Condvar::new(),
            last_validated: Mutex::new(None),
        });
        let requester = Requester {
            shared_state: shared_state.clone(),
            last_submitted_revision: None,
        };
        let compiler = Validator {
            shared_state,
            last_validated_revision: None,
        };
        (requester, compiler)
    }

    fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<T> {
        // `mutex.lock()` only fails if the lock is 'poisoned', meaning another
        // thread panicked while accessing it. In this program we have no intent
        // to recover from panicked threads, so letting the original problem
        // showball by calling `unwrap()` here is fine.
        mutex.lock().unwrap()
    }

    fn is_new_revision<T>(last_checked_revision: &mut Option<usize>, t: &T) -> bool
    where
        T: Revision,
    {
        let revision = t.revision();
        let is_new = match last_checked_revision {
            None => true,
            Some(old) => revision > *old,
        };
        if is_new {
            *last_checked_revision = Some(revision);
        }
        is_new
    }
}

#[cfg(test)]
mod tests {
    use crate::simulation::simulation_test;

    simulation_test!(change_record_field_name);
    simulation_test!(add_field_to_record);
    simulation_test!(add_field_to_front_of_record);
    simulation_test!(add_field_to_empty_record);
    simulation_test!(remove_field_from_record);
    simulation_test!(remove_only_field_from_record);
    simulation_test!(remove_field_from_front_of_record);
    simulation_test!(add_import);
    simulation_test!(remove_import);
    simulation_test!(add_as_clause_to_import);
    simulation_test!(remove_as_clause_from_import);
    simulation_test!(change_as_clause_on_import);
    simulation_test!(change_argument_name_at_definition_site);
    simulation_test!(change_argument_name_at_usage_site);
    simulation_test!(change_let_binding_name_at_definition_site);
    simulation_test!(change_let_binding_name_at_usage_site);
    simulation_test!(change_function_name_in_type_definition);
    simulation_test!(change_function_name_at_definition_site);
    simulation_test!(change_function_name_at_usage_site);
    simulation_test!(change_type_name_at_definition_site);
    simulation_test!(change_type_name_at_usage_site);
    simulation_test!(add_type_definition);
    simulation_test!(remove_type_definition);
    simulation_test!(add_type_alias_definition);
    simulation_test!(remove_type_alias_definition);
    simulation_test!(add_module_qualifier_to_variable);
    simulation_test!(remove_module_qualifier_from_variable);
    simulation_test!(add_module_qualifier_to_type);
    simulation_test!(remove_module_qualifier_from_type);
    simulation_test!(no_interpretation_when_back_at_compiling_state);
}

// A helper for defining tests where the test input and expected output are
// included in the same file. These are like golden tests, in the sense that the
// expected output will be appended to files automatically if they're missing,
// and asserted against if present.
#[cfg(test)]
mod included_answer_test {
    use std::io::Write;
    use std::path::Path;

    pub fn assert_eq_answer_in(output: &str, path: &Path) {
        let prefix = "-- ";
        let separator = "\n\n".to_owned() + prefix + "=== expected output below ===\n";
        let contents = assert_ok(std::fs::read_to_string(path));
        match contents.split_once(&separator) {
            None => {
                let mut file = assert_ok(std::fs::OpenOptions::new().append(true).open(path));
                assert_ok(file.write_all(separator.as_bytes()));
                for line in output.lines() {
                    assert_ok(file.write_all(prefix.as_bytes()));
                    assert_ok(file.write_all(line.as_bytes()));
                    assert_ok(file.write_all("\n".as_bytes()));
                }
            }
            Some((_, expected_output_prefixed)) => {
                let expected_output = expected_output_prefixed
                    .trim_end()
                    .lines()
                    .map(|x| x.strip_prefix(&prefix).unwrap_or(x))
                    .collect::<Vec<&str>>()
                    .join("\n");
                assert_eq!(output, expected_output)
            }
        }
    }

    fn assert_ok<A, E: std::fmt::Debug>(result: Result<A, E>) -> A {
        match result {
            Err(err) => panic!("{:?}", err),
            Ok(x) => x,
        }
    }
}

// A module to support tests of the diffing logic by running simulations against
// it.
#[cfg(test)]
mod simulation {
    use crate::included_answer_test::assert_eq_answer_in;
    use crate::{Edit, ElmChange, Msg, SourceFileSnapshot};
    use std::collections::VecDeque;
    use std::io::BufRead;
    use std::path::{Path, PathBuf};
    use tree_sitter::{InputEdit, Point};

    #[macro_export]
    macro_rules! simulation_test {
        ($name:ident) => {
            #[test]
            fn $name() {
                let mut path = std::path::PathBuf::new();
                path.push("./tests");
                let module_name = crate::simulation::snake_to_camel(stringify!($name));
                path.push(module_name + ".elm");
                println!("Run simulation {:?}", &path);
                crate::simulation::run_simulation_test(&path);
            }
        };
    }
    pub use simulation_test;

    pub fn run_simulation_test(path: &Path) {
        match run_simulation_test_helper(path) {
            Err(err) => panic!("simulation failed with: {:?}", err),
            Ok(val) => assert_eq_answer_in(&format!("{:?}", val), path),
        }
    }

    fn run_simulation_test_helper(path: &Path) -> Result<Option<ElmChange>, Error> {
        let mut simulation = Simulation::from_file(path)?;
        let mut elm_change = None;
        let store_elm_change = |change| elm_change = change;
        crate::run_change_analysis_thread(
            simulation.validation_requester,
            &mut simulation.iterator,
            store_elm_change,
        )
        .map_err(Error::RunningSimulationFailed)?;
        Ok(elm_change)
    }

    struct Simulation {
        validation_requester: crate::validation::Requester<SourceFileSnapshot>,
        iterator: SimulationIterator,
    }

    impl Simulation {
        fn from_file(path: &Path) -> Result<Simulation, Error> {
            let file = std::fs::File::open(path).map_err(Error::FromFileOpenFailed)?;
            let mut lines = std::io::BufReader::new(file)
                .lines()
                .map(|line| line.map_err(Error::FromFileReadingLineFailed));
            let (code, simulation_script_padding) = find_start_simulation_script(&mut lines)?;
            let mut builder = SimulationBuilder::new(path.to_path_buf(), code);
            loop {
                let line = match lines.next() {
                    None => return Err(Error::FileEndCameBeforeSimulationEnd),
                    Some(line) => line?
                        .get(simulation_script_padding..)
                        .ok_or(Error::SimulationInstructionsDontHaveConsistentPadding)?
                        .to_string(),
                };
                match line.split(' ').collect::<Vec<&str>>().as_slice() {
                    ["END", "SIMULATION"] => break,
                    ["MOVE", "CURSOR", "TO", "LINE", line_str, strs @ ..] => {
                        let line = line_str
                            .parse()
                            .map_err(|_| Error::CannotParseLineNumber(line.to_string()))?;
                        builder = builder.move_cursor(line, &strs.join(" "))?
                    }
                    ["INSERT", strs @ ..] => builder = builder.insert(&strs.join(" ")),
                    ["DELETE", strs @ ..] => builder = builder.delete(&strs.join(" "))?,
                    ["COMPILATION", "SUCCEEDS"] => builder = builder.compilation_succeeds(),
                    _ => return Err(Error::CannotParseSimulationLine(line)),
                };
            }
            Ok(builder.finish())
        }
    }

    struct SimulationIterator {
        validation_processor: crate::validation::Validator<SourceFileSnapshot>,
        msgs: VecDeque<Msg>,
    }

    impl Iterator for SimulationIterator {
        type Item = Msg;

        fn next(&mut self) -> Option<Self::Item> {
            self.msgs.pop_front().map(|msg| {
                if let Msg::CompilationSucceeded = msg {
                    let last_snapshot = self.validation_processor.next();
                    self.validation_processor.mark_valid(last_snapshot);
                }
                msg
            })
        }
    }

    fn find_start_simulation_script<I>(lines: &mut I) -> Result<(Vec<u8>, usize), Error>
    where
        I: Iterator<Item = Result<String, Error>>,
    {
        let mut code: Vec<u8> = Vec::new();
        loop {
            let line = match lines.next() {
                None => return Err(Error::FromFileFailedNoStartSimulationFound),
                Some(Err(err)) => return Err(err),
                Some(Ok(line)) => line,
            };
            if let Some(padding) = line.strip_suffix("START SIMULATION") {
                break Ok((code, padding.len()));
            } else {
                code.push(10 /* \n */);
                code.append(&mut line.into_bytes());
            }
        }
    }

    struct SimulationBuilder {
        file: PathBuf,
        current_bytes: Vec<u8>,
        current_position: usize,
        msgs: VecDeque<Msg>,
    }

    impl SimulationBuilder {
        fn new(file: PathBuf, initial_bytes: Vec<u8>) -> SimulationBuilder {
            let init_msg = Msg::ReceivedEditorEvent(Edit {
                file: file.clone(),
                new_bytes: std::string::String::from_utf8(initial_bytes.clone()).unwrap(),
                input_edit: InputEdit {
                    start_byte: 0,
                    old_end_byte: 0,
                    new_end_byte: 0,
                    start_position: Point { row: 0, column: 0 },
                    old_end_position: Point { row: 0, column: 0 },
                    new_end_position: Point { row: 0, column: 0 },
                },
            });
            let mut msgs = VecDeque::new();
            msgs.push_front(init_msg);
            SimulationBuilder {
                file,
                current_position: 0,
                current_bytes: initial_bytes,
                msgs,
            }
        }

        fn move_cursor(mut self, line: u64, word: &str) -> Result<Self, Error> {
            self.current_position = 0;
            let mut reversed_bytes: Vec<u8> =
                self.current_bytes.clone().into_iter().rev().collect();
            if line == 0 {
                return Err(Error::MoveCursorFailedLineZeroNotAllowed);
            }
            let mut lines_to_go = line;
            while lines_to_go > 0 {
                self.current_position += 1;
                match reversed_bytes.pop() {
                    None => return Err(Error::MoveCursorFailedNotEnoughLines),
                    Some(10 /* \n */) => lines_to_go -= 1,
                    Some(_) => {}
                }
            }
            let reversed_word_bytes: Vec<u8> = word.bytes().rev().collect();
            while !reversed_bytes.ends_with(&reversed_word_bytes) {
                self.current_position += 1;
                match reversed_bytes.pop() {
                    None => return Err(Error::MoveCursorDidNotFindWordOnLine),
                    Some(10 /* \n */) => return Err(Error::MoveCursorDidNotFindWordOnLine),
                    Some(_) => {}
                }
            }
            Ok(self)
        }

        fn insert(mut self, str: &str) -> Self {
            let bytes = str.bytes();
            let range = self.current_position..self.current_position;
            self.current_bytes.splice(range.clone(), bytes.clone());
            self.current_position += bytes.len();
            self.msgs.push_back(Msg::ReceivedEditorEvent(Edit {
                file: self.file.clone(),
                new_bytes: str.to_owned(),
                input_edit: InputEdit {
                    start_byte: range.start,
                    old_end_byte: range.start,
                    new_end_byte: self.current_position,
                    start_position: Point { row: 0, column: 0 },
                    old_end_position: Point { row: 0, column: 0 },
                    new_end_position: Point { row: 0, column: 0 },
                },
            }));
            self
        }

        fn delete(mut self, str: &str) -> Result<Self, Error> {
            let bytes = str.as_bytes();
            let range = self.current_position..(self.current_position + bytes.len());
            if self.current_bytes.get(range.clone()) == Some(bytes) {
                self.current_bytes.splice(range.clone(), []);
                self.msgs.push_back(Msg::ReceivedEditorEvent(Edit {
                    file: self.file.clone(),
                    new_bytes: String::new(),
                    input_edit: InputEdit {
                        start_byte: range.start,
                        old_end_byte: range.end,
                        new_end_byte: range.start,
                        start_position: Point { row: 0, column: 0 },
                        old_end_position: Point { row: 0, column: 0 },
                        new_end_position: Point { row: 0, column: 0 },
                    },
                }));
                Ok(self)
            } else {
                Err(Error::DeleteFailedNoSuchStrAtCursor)
            }
        }

        fn compilation_succeeds(mut self) -> Self {
            self.msgs.push_back(Msg::CompilationSucceeded);
            self
        }

        fn finish(self) -> Simulation {
            let (validation_requester, validation_processor) = crate::validation::channel(1);
            Simulation {
                validation_requester,
                iterator: SimulationIterator {
                    validation_processor,
                    msgs: self.msgs,
                },
            }
        }
    }

    pub fn snake_to_camel(str: &str) -> String {
        str.split('_')
            .map(|word| {
                let (first, rest) = word.split_at(1);
                first.to_uppercase() + rest
            })
            .collect::<Vec<String>>()
            .join("")
    }

    #[derive(Debug)]
    enum Error {
        RunningSimulationFailed(crate::Error),
        FromFileFailedNoStartSimulationFound,
        CannotParseSimulationLine(String),
        CannotParseLineNumber(String),
        FileEndCameBeforeSimulationEnd,
        SimulationInstructionsDontHaveConsistentPadding,
        FromFileOpenFailed(std::io::Error),
        FromFileReadingLineFailed(std::io::Error),
        MoveCursorFailedLineZeroNotAllowed,
        MoveCursorFailedNotEnoughLines,
        MoveCursorDidNotFindWordOnLine,
        DeleteFailedNoSuchStrAtCursor,
    }
}
