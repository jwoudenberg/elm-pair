use core::ops::Range;
use mvar::MVar;
use ropey::Rope;
use std::io::Write;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use tree_sitter::{InputEdit, Node, Query, QueryCursor, Tree, TreeCursor};

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
    let new_diff_available = Arc::new(MVar::new());
    let thread_error = Arc::new(MVar::new());
    let latest_code = Arc::new(MVar::new());
    let editor_driver = Arc::new(MVar::new());
    let (mut compilation_candidates, last_compiling_code) =
        compilation_thread::run(
            thread_error.clone(),
            new_diff_available.clone(),
        );
    run_diff_thread(
        thread_error.clone(),
        editor_driver.clone(),
        latest_code.clone(),
        last_compiling_code,
        new_diff_available.clone(),
    );
    run_editor_listener_thread(
        &thread_error,
        |snapshot| compilation_candidates.push(snapshot),
        &latest_code,
        &new_diff_available,
        &editor_driver,
    )
}

fn run_diff_thread<W>(
    error_var: Arc<MVar<Error>>,
    editor_driver_var: Arc<MVar<neovim::NeovimDriver<W>>>,
    latest_code: Arc<MVar<SourceFileSnapshot>>,
    last_compiling_code: Arc<MVar<SourceFileSnapshot>>,
    new_diff_available: Arc<MVar<()>>,
) where
    W: Write + Send + 'static,
{
    std::thread::spawn(move || {
        std::thread::spawn(move || {
            crate::report_error(
                error_var,
                run_diff_loop(
                    editor_driver_var,
                    latest_code,
                    last_compiling_code,
                    new_diff_available,
                ),
            )
        });
    });
}

fn run_diff_loop<W>(
    editor_driver_var: Arc<MVar<neovim::NeovimDriver<W>>>,
    latest_code: Arc<MVar<SourceFileSnapshot>>,
    last_compiling_code: Arc<MVar<SourceFileSnapshot>>,
    new_diff_available: Arc<MVar<()>>,
) -> Result<(), Error>
where
    W: Write,
{
    let editor_driver = editor_driver_var.take();
    let mut diff_iterator = DiffIterator {
        latest_code,
        last_compiling_code,
        new_diff_available,
    };
    diff_iterator.try_for_each(|diff| {
        let opt_change = analyze_diff(&diff);
        if let Some(change) = opt_change {
            let refactor = elm_refactor(&diff, &change);
            editor_driver.apply_edits(refactor)?;
        }
        Ok(())
    })
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

struct FileData {
    // Absolute path to this source file.
    path: PathBuf,
    // Root of the Elm project containing this source file.
    project_root: PathBuf,
    // Absolute path to the `elm` compiler.
    elm_bin: PathBuf,
}

// A change made by the user reported by the editor.
#[derive(Debug)]
struct Edit {
    // The file that was changed.
    file: PathBuf,
    // A tree-sitter InputEdit value, describing what part of the file was changed.
    input_edit: InputEdit,
    // Bytes representing the new contents of the file at the location described
    // by `input_edit`.
    new_bytes: String,
}

fn mk_edit(
    code: &SourceFileSnapshot,
    range: &Range<usize>,
    new_bytes: String,
) -> Edit {
    let new_end_byte = range.start + new_bytes.len();
    Edit {
        file: code.file_data.path.clone(),
        new_bytes,
        input_edit: tree_sitter::InputEdit {
            start_byte: range.start,
            old_end_byte: range.end,
            new_end_byte,
            start_position: byte_to_point(code, range.start),
            old_end_position: byte_to_point(code, range.end),
            new_end_position: byte_to_point(code, new_end_byte),
        },
    }
}

fn byte_to_point(code: &SourceFileSnapshot, byte: usize) -> tree_sitter::Point {
    let row = code.bytes.byte_to_line(byte);
    tree_sitter::Point {
        row,
        column: code.bytes.byte_to_char(byte) - code.bytes.line_to_char(row),
    }
}

#[derive(Debug)]
enum Error {
    DidNotFindElmBinaryOnPath,
    CouldNotReadCurrentWorkingDirectory(std::io::Error),
    DidNotFindPathEnvVar,
    NoElmJsonFoundInAnyAncestorDirectoryOf(PathBuf),
    SocketCreationFailed(std::io::Error),
    AcceptingIncomingSocketConnectionFailed(std::io::Error),
    CloningSocketFailed(std::io::Error),
    NeovimMessageDecodingFailed(neovim::DecodingError),
    CompilationFailedToCreateTempDir(std::io::Error),
    CompilationFailedToWriteCodeToTempFile(std::io::Error),
    CompilationFailedToRunElmMake(std::io::Error),
    TreeSitterParsingFailed,
    TreeSitterSettingLanguageFailed(tree_sitter::LanguageError),
}

struct SourceFileDiff {
    old: SourceFileSnapshot,
    new: SourceFileSnapshot,
}

struct DiffIterator {
    latest_code: Arc<MVar<SourceFileSnapshot>>,
    last_compiling_code: Arc<MVar<SourceFileSnapshot>>,
    new_diff_available: Arc<MVar<()>>,
}

impl<'a> Iterator for DiffIterator {
    type Item = SourceFileDiff;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            self.new_diff_available.take();
            if let (Some(old), Some(new)) = (
                self.last_compiling_code.try_read(),
                self.latest_code.try_read(),
            ) {
                return Some(SourceFileDiff { old, new });
            }
        }
    }
}

// TODO: Remove this function (currently only used by simulation).
fn handle_edit_event(
    latest_code: &mut Option<SourceFileSnapshot>,
    edit: Edit,
) -> Result<(), Error> {
    // Edit the old tree-sitter tree, or create it if we don't have one yet for
    // this file.
    match latest_code {
        None => get_initial_snapshot_from_first_edit(latest_code, edit)?,
        Some(code) => apply_edit(code, edit),
    }

    let latest_code = match latest_code {
        None => return Ok(()),
        Some(code) => code,
    };

    // Update the tree-sitter syntax three. Note: this is a separate step from
    // editing the old tree. See the tree-sitter docs on parsing for more info.
    reparse_tree(latest_code)?;
    Ok(())
}

fn analyze_diff(diff: &SourceFileDiff) -> Option<ElmChange> {
    let tree_changes = diff_trees(diff);
    interpret_change(&tree_changes)
}

fn elm_refactor(diff: &SourceFileDiff, change: &ElmChange) -> Vec<Edit> {
    let query_str = r#"
[(import_clause
    exposing:
      (exposing_list
        [
          (double_dot)
          (exposed_value)
          (exposed_type)
          (exposed_operator)
        ] @exposed_value
      )
 ) @import
]"#;
    let query = Query::new(diff.new.tree.language(), query_str).unwrap();
    match change {
        ElmChange::QualifierAdded(name, qualifier) => {
            // debug_print_tree(&diff.new);
            let mut cursor = QueryCursor::new();
            let exposed = cursor
                .matches(&query, diff.new.tree.root_node(), |node| {
                    debug_code_slice(&diff.new, &node.byte_range())
                })
                .find_map(|m| {
                    let (import, exposed_val) = match m.captures {
                        [x, y] => (x, y),
                        _ => panic!("wrong number of capures"),
                    };
                    let import_node =
                        import.node.child_by_field_name("moduleName")?;
                    let import_name =
                        debug_code_slice(&diff.new, &import_node.byte_range());
                    let exposed_name = debug_code_slice(
                        &diff.new,
                        &exposed_val.node.byte_range(),
                    );
                    if import_name == *qualifier && exposed_name == *name {
                        Some(exposed_val.node)
                    } else {
                        None
                    }
                })
                .unwrap();
            let range = || {
                let next = exposed.next_sibling();
                if let Some(node) = next {
                    if node.kind() == "," {
                        return exposed.start_byte()..node.end_byte();
                    }
                }
                let prev = exposed.prev_sibling();
                if let Some(node) = prev {
                    if node.kind() == "," {
                        return node.start_byte()..exposed.end_byte();
                    }
                }
                exposed.byte_range()
            };
            vec![mk_edit(&diff.new, &range(), String::new())]
        }
        _ => Vec::new(),
    }
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
        path: file,
    });
    *code = Some(SourceFileSnapshot {
        tree,
        bytes,
        file_data,
        revision: 0,
    });
    Ok(())
}

fn report_error<I>(thread_error: Arc<MVar<Error>>, result: Result<I, Error>) {
    match result {
        Ok(_) => {}
        Err(err) => {
            // If there's already an error in there, leave it. The first error
            // is probably reported is probably the closest to the problem.
            thread_error.try_put(err);
        }
    }
}

fn run_editor_listener_thread<R>(
    thread_error: &MVar<Error>,
    mut request_compilation: R,
    latest_code_var: &MVar<SourceFileSnapshot>,
    new_diff_available: &MVar<()>,
    editor_driver: &MVar<neovim::NeovimDriver<UnixStream>>,
) -> Result<(), Error>
where
    R: FnMut(SourceFileSnapshot),
{
    let socket_path = "/tmp/elm-pair.sock";
    let listener =
        UnixListener::bind(socket_path).map_err(Error::SocketCreationFailed)?;
    // TODO: Figure out how to deal with multiple connections.
    for socket in listener.incoming().into_iter() {
        let read_socket =
            socket.map_err(Error::AcceptingIncomingSocketConnectionFailed)?;
        let write_socket = read_socket
            .try_clone()
            .map_err(Error::CloningSocketFailed)?;
        let neovim = neovim::Neovim::new(read_socket, write_socket, |change| {
            if let Some(error) = thread_error.try_take() {
                return Err(error);
            }
            let mut latest_code = latest_code_var.try_take();
            let opt_edit = change.apply(&mut latest_code);
            match latest_code {
                None => return Ok(()),
                Some(mut code) => {
                    if let Some(edit) = opt_edit {
                        code.tree.edit(&edit);
                        reparse_tree(&mut code)?;
                    }
                    if !code.tree.root_node().has_error() {
                        request_compilation(code.clone());
                    }
                    latest_code_var.write(code);
                }
            };
            new_diff_available.write(());
            Ok(())
        });
        editor_driver.write(neovim.driver());
        neovim.start()?;
    }
    Ok(())
}

fn find_executable(name: &str) -> Result<PathBuf, Error> {
    let cwd = std::env::current_dir()
        .map_err(Error::CouldNotReadCurrentWorkingDirectory)?;
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
        (
            [("lower_case_identifier", before)],
            [("lower_case_identifier", after)],
        ) => Some(ElmChange::NameChanged(
            debug_code_slice(changes.old_code, &before.byte_range()),
            debug_code_slice(changes.new_code, &after.byte_range()),
        )),
        (
            [("upper_case_identifier", before)],
            [("upper_case_identifier", after)],
        ) => match before.parent()?.kind() {
            "as_clause" => Some(ElmChange::AsClauseChanged(
                debug_code_slice(changes.old_code, &before.byte_range()),
                debug_code_slice(changes.new_code, &after.byte_range()),
            )),
            _ => Some(ElmChange::TypeChanged(
                debug_code_slice(changes.old_code, &before.byte_range()),
                debug_code_slice(changes.new_code, &after.byte_range()),
            )),
        },
        ([], [("import_clause", after)]) => Some(ElmChange::ImportAdded(
            debug_code_slice(changes.new_code, &after.byte_range()),
        )),
        ([("import_clause", before)], []) => Some(ElmChange::ImportRemoved(
            debug_code_slice(changes.old_code, &before.byte_range()),
        )),
        ([], [("type_declaration", after)]) => Some(ElmChange::TypeAdded(
            debug_code_slice(changes.new_code, &after.byte_range()),
        )),
        ([("type_declaration", before)], []) => Some(ElmChange::TypeRemoved(
            debug_code_slice(changes.old_code, &before.byte_range()),
        )),
        ([], [("type_alias_declaration", after)]) => {
            Some(ElmChange::TypeAliasAdded(debug_code_slice(
                changes.new_code,
                &after.byte_range(),
            )))
        }
        ([("type_alias_declaration", before)], []) => {
            Some(ElmChange::TypeAliasRemoved(debug_code_slice(
                changes.old_code,
                &before.byte_range(),
            )))
        }
        ([], [("field_type", after)]) => Some(ElmChange::FieldAdded(
            debug_code_slice(changes.new_code, &after.byte_range()),
        )),
        ([], [(",", _), ("field_type", after)]) => Some(ElmChange::FieldAdded(
            debug_code_slice(changes.new_code, &after.byte_range()),
        )),
        ([], [("field_type", after), (",", _)]) => Some(ElmChange::FieldAdded(
            debug_code_slice(changes.new_code, &after.byte_range()),
        )),
        ([("field_type", before)], []) => Some(ElmChange::FieldRemoved(
            debug_code_slice(changes.old_code, &before.byte_range()),
        )),
        ([(",", _), ("field_type", before)], []) => {
            Some(ElmChange::FieldRemoved(debug_code_slice(
                changes.old_code,
                &before.byte_range(),
            )))
        }
        ([("field_type", before), (",", _)], []) => {
            Some(ElmChange::FieldRemoved(debug_code_slice(
                changes.old_code,
                &before.byte_range(),
            )))
        }
        (
            [("upper_case_identifier", qualifier), ("dot", _), ("upper_case_identifier", before)],
            [("upper_case_identifier", after)],
        ) => {
            let name_before =
                debug_code_slice(changes.old_code, &before.byte_range());
            let name_after =
                debug_code_slice(changes.new_code, &after.byte_range());
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
            let name_before =
                debug_code_slice(changes.old_code, &before.byte_range());
            let name_after =
                debug_code_slice(changes.new_code, &after.byte_range());
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
            let name_before =
                debug_code_slice(changes.old_code, &before.byte_range());
            let name_after =
                debug_code_slice(changes.new_code, &after.byte_range());
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
            let name_before =
                debug_code_slice(changes.old_code, &before.byte_range());
            let name_after =
                debug_code_slice(changes.new_code, &after.byte_range());
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
            debug_code_slice(
                changes.old_code,
                &before.prev_sibling()?.byte_range(),
            ),
            debug_code_slice(
                changes.old_code,
                &before.child_by_field_name("name")?.byte_range(),
            ),
        )),
        ([], [("as_clause", after)]) => Some(ElmChange::AsClauseAdded(
            debug_code_slice(
                changes.new_code,
                &after.prev_sibling()?.byte_range(),
            ),
            debug_code_slice(
                changes.new_code,
                &after.child_by_field_name("name")?.byte_range(),
            ),
        )),
        _ => {
            // debug_print_tree_changes(changes);
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

fn diff_trees(diff: &SourceFileDiff) -> TreeChanges<'_> {
    let old_code = &diff.old;
    let new_code = &diff.new;
    let mut old = diff.old.tree.walk();
    let mut new = diff.new.tree.walk();
    loop {
        match goto_first_changed_sibling(old_code, new_code, &mut old, &mut new)
        {
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
                (true, false) => {
                    return FirstChangedSibling::OldAtFirstAdditional
                }
                (false, true) => {
                    return FirstChangedSibling::NewAtFirstAdditional
                }
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

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<T> {
    // `mutex.lock()` only fails if the lock is 'poisoned', meaning another
    // thread panicked while accessing it. In this program we have no intent
    // to recover from panicked threads, so letting the original problem
    // showball by calling `unwrap()` here is fine.
    mutex.lock().unwrap()
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

trait EditorSourceChange {
    fn apply(&self, code: &mut Option<SourceFileSnapshot>)
        -> Option<InputEdit>;
}

mod neovim {
    use crate::{
        Edit, EditorSourceChange, Error, InputEdit, SourceFileSnapshot,
    };
    use byteorder::ReadBytesExt;
    use ropey::RopeBuilder;
    use std::io::{BufReader, Read, Write};
    use std::ops::DerefMut;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    pub(crate) struct Neovim<R, W, P> {
        read: BufReader<R>,
        write: Arc<Mutex<W>>,
        apply_buf_change: P,
    }

    impl<R: Read, W: Write, P: FnMut(BufChange) -> Result<(), Error>>
        Neovim<R, W, P>
    {
        pub fn new(read: R, write: W, apply_buf_change: P) -> Self
        where
            R: Read,
        {
            Neovim {
                read: BufReader::new(read),
                write: Arc::new(Mutex::new(write)),
                apply_buf_change,
            }
        }

        pub fn driver(&self) -> NeovimDriver<W> {
            NeovimDriver {
                write: self.write.clone(),
            }
        }

        pub fn start(mut self) -> Result<(), Error> {
            // TODO: figure out how to stop this loop when we the reader closes.
            loop {
                self.parse_msg()?;
            }
        }

        // Messages we receive from neovim's webpack-rpc API:
        // neovim api:  https://neovim.io/doc/user/api.html
        // webpack-rpc: https://github.com/msgpack-rpc/msgpack-rpc/blob/master/spec.md
        //
        // TODO handle neovim API versions
        fn parse_msg(&mut self) -> Result<(), Error> {
            let array_len =
                rmp::decode::read_array_len(&mut self.read).map_err(val_err)?;
            let type_ =
                rmp::decode::read_int(&mut self.read).map_err(num_val_err)?;
            if array_len == 3 && type_ == 2 {
                self.parse_notification_msg()
            } else {
                decoding_error(DecodingError::UnknownMessageType(
                    array_len, type_,
                ))
            }
        }

        fn parse_notification_msg(&mut self) -> Result<(), Error> {
            let mut buffer = [0u8; 30];
            // TODO: Don't parse to UTF8 here. Compare bytestrings instead.
            let method = rmp::decode::read_str(&mut self.read, &mut buffer)
                .map_err(str_err)?;
            match method {
                "nvim_error_event" => self.parse_error_event(),
                "nvim_buf_lines_event" => self.parse_buf_lines_event(),
                "nvim_buf_changedtick_event" => {
                    self.parse_buf_changedtick_event()
                }
                "nvim_buf_detach_event" => self.parse_buf_detach_event(),
                "buffer_opened" => self.parse_buffer_opened(),
                method => decoding_error(DecodingError::UnknownEventMethod(
                    method.to_owned(),
                )),
            }
        }

        fn parse_error_event(&mut self) -> Result<(), Error> {
            let array_len =
                rmp::decode::read_array_len(&mut self.read).map_err(val_err)?;
            if array_len < 2 {
                return decoding_error(
                    DecodingError::NotEnoughArgsInBufLinesEvent(array_len),
                );
            }
            let type_ =
                rmp::decode::read_int(&mut self.read).map_err(num_val_err)?;
            let msg = read_string(&mut self.read)?;
            skip_objects(&mut self.read, array_len - 2)?;

            decoding_error(DecodingError::ReceivedErrorEvent(type_, msg))
        }

        fn parse_buffer_opened(&mut self) -> Result<(), Error> {
            let array_len =
                rmp::decode::read_array_len(&mut self.read).map_err(val_err)?;
            if array_len < 2 {
                return decoding_error(
                    DecodingError::NotEnoughArgsInBufLinesEvent(array_len),
                );
            }
            let buf =
                rmp::decode::read_int(&mut self.read).map_err(num_val_err)?;
            skip_objects(&mut self.read, array_len - 1)?;
            self.nvim_buf_attach(buf)
        }

        fn parse_buf_lines_event(&mut self) -> Result<(), Error> {
            let array_len =
                rmp::decode::read_array_len(&mut self.read).map_err(val_err)?;
            if array_len < 6 {
                return decoding_error(
                    DecodingError::NotEnoughArgsInBufLinesEvent(array_len),
                );
            }
            let buf = read_buf(&mut self.read)?;
            let changedtick =
                rmp::decode::read_int(&mut self.read).map_err(num_val_err)?;
            let firstline =
                rmp::decode::read_int(&mut self.read).map_err(num_val_err)?;
            let lastline =
                rmp::decode::read_int(&mut self.read).map_err(num_val_err)?;
            let mut line_count =
                rmp::decode::read_array_len(&mut self.read).map_err(val_err)?;
            let mut linedata = Vec::with_capacity(line_count as usize);
            while line_count > 0 {
                line_count -= 1;
                linedata.push(read_string(&mut self.read)?);
            }
            // I'm not using the `more` argument for anything.
            let _more =
                rmp::decode::read_bool(&mut self.read).map_err(val_err)?;
            let extra_args = array_len - 6;
            skip_objects(&mut self.read, extra_args)?;
            (self.apply_buf_change)(BufChange {
                _buf: buf as u64,
                changedtick,
                firstline,
                lastline,
                linedata,
            })
        }

        fn parse_buf_changedtick_event(&mut self) -> Result<(), Error> {
            // We're not interested in these events, so we skip them.
            let array_len =
                rmp::decode::read_array_len(&mut self.read).map_err(val_err)?;
            skip_objects(&mut self.read, array_len)?;
            Ok(())
        }

        fn parse_buf_detach_event(&mut self) -> Result<(), Error> {
            // Re-attach this buffer
            // TODO: consider when we might not want to reattach.
            let array_len =
                rmp::decode::read_array_len(&mut self.read).map_err(val_err)?;
            if array_len < 1 {
                return decoding_error(
                    DecodingError::NotEnoughArgsInBufLinesEvent(array_len),
                );
            }
            let buf = read_buf(&mut self.read)?;
            skip_objects(&mut self.read, array_len - 1)?;
            self.nvim_buf_attach(buf)
        }

        fn nvim_buf_attach(&self, buf: u8) -> Result<(), Error> {
            let mut write_guard = crate::lock(&self.write);
            let write = write_guard.deref_mut();
            rmp::encode::write_array_len(write, 3).unwrap();
            rmp::encode::write_i8(write, 2).unwrap();
            write_str(write, "nvim_buf_attach");
            // nvim_buf_attach arguments
            rmp::encode::write_array_len(write, 3).unwrap();
            rmp::encode::write_u8(write, buf).unwrap(); //buf
            rmp::encode::write_bool(write, true).unwrap(); // send_buffer
            rmp::encode::write_map_len(write, 0).unwrap(); // opts
            Ok(())
        }
    }

    pub struct BufChange {
        _buf: u64,
        changedtick: u64,
        firstline: i64,
        lastline: i64,
        linedata: Vec<String>,
    }

    impl EditorSourceChange for BufChange {
        fn apply(
            &self,
            opt_code: &mut Option<SourceFileSnapshot>,
        ) -> Option<InputEdit> {
            match (self.lastline, opt_code) {
                (-1, code) => {
                    let mut builder = RopeBuilder::new();
                    for line in &self.linedata {
                        builder.append(line);
                        builder.append("\n");
                    }
                    let bytes = builder.finish();
                    *code = Some(SourceFileSnapshot {
                        tree: crate::parse(None, &bytes).unwrap(),
                        bytes,
                        revision: self.changedtick as usize,
                        file_data: Arc::new(crate::FileData {
                            // TODO: put real data here.
                            path: PathBuf::new(),
                            project_root: PathBuf::from(
                                "/home/jasper/dev/elm-pair/tests",
                            ),
                            elm_bin: PathBuf::from("elm"),
                        }),
                    });
                    None
                }
                (_, None) => panic!("incremental update for unknown code."),
                (lastline, Some(code)) => {
                    let start_line = self.firstline as usize;
                    let old_end_line = lastline as usize;
                    let start_char = code.bytes.line_to_char(start_line);
                    let start_byte = code.bytes.line_to_byte(start_line);
                    let old_end_char = code.bytes.line_to_char(old_end_line);
                    let old_end_byte = code.bytes.line_to_byte(old_end_line);
                    let mut new_end_byte = start_byte;
                    code.bytes.remove(start_char..old_end_char);
                    for line in &self.linedata {
                        code.bytes.insert(start_char, line.as_str());
                        new_end_byte += line.len();
                        let new_end_char =
                            code.bytes.byte_to_char(new_end_byte);
                        code.bytes.insert_char(new_end_char, '\n');
                        new_end_byte += 1;
                    }
                    code.revision = self.changedtick as usize;
                    Some(InputEdit {
                        start_byte,
                        old_end_byte,
                        new_end_byte,
                        start_position: crate::byte_to_point(code, start_byte),
                        old_end_position: crate::byte_to_point(
                            code,
                            old_end_byte,
                        ),
                        new_end_position: crate::byte_to_point(
                            code,
                            new_end_byte,
                        ),
                    })
                }
            }
        }
    }

    #[derive(Debug)]
    pub enum DecodingError {
        DecodingFailedWithInvalidMarkerRead(rmp::decode::Error),
        DecodingFailedWithInvalidDataRead(rmp::decode::Error),
        DecodingFailedWithTypeMismatch(rmp::Marker),
        DecodingFailedWithOutOfRange,
        DecodingFailedWithInvalidUtf8(core::str::Utf8Error),
        DecodingFailedWithBufferSizeTooSmall(u32),
        DecodingFailedWhileSkipping(std::io::Error),
        UnknownMessageType(u32, u8),
        UnknownEventMethod(String),
        NotEnoughArgsInBufLinesEvent(u32),
        ReceivedErrorEvent(u64, String),
    }

    fn decoding_error(err: DecodingError) -> Result<(), Error> {
        Err(Error::NeovimMessageDecodingFailed(err))
    }

    // Skip `count` messagepack options. If one of these objects is an array or
    // map, skip its contents too.
    fn skip_objects<R>(read: &mut BufReader<R>, count: u32) -> Result<(), Error>
    where
        R: Read,
    {
        let mut count = count;
        while count > 0 {
            count -= 1;
            let marker = rmp::decode::read_marker(read).map_err(marker_err)?;
            match marker {
                rmp::Marker::FixPos(_) => {}
                rmp::Marker::FixNeg(_) => {}
                rmp::Marker::Null => {}
                rmp::Marker::True => {}
                rmp::Marker::False => {}
                rmp::Marker::U8 => skip_bytes(read, 1)?,
                rmp::Marker::U16 => skip_bytes(read, 2)?,
                rmp::Marker::U32 => skip_bytes(read, 4)?,
                rmp::Marker::U64 => skip_bytes(read, 8)?,
                rmp::Marker::I8 => skip_bytes(read, 1)?,
                rmp::Marker::I16 => skip_bytes(read, 2)?,
                rmp::Marker::I32 => skip_bytes(read, 4)?,
                rmp::Marker::I64 => skip_bytes(read, 8)?,
                rmp::Marker::F32 => skip_bytes(read, 4)?,
                rmp::Marker::F64 => skip_bytes(read, 8)?,
                rmp::Marker::FixStr(bytes) => skip_bytes(read, bytes as u64)?,
                rmp::Marker::Str8 => {
                    let bytes = read
                        .read_u8()
                        .map_err(DecodingError::DecodingFailedWhileSkipping)
                        .map_err(Error::NeovimMessageDecodingFailed)?;
                    skip_bytes(read, bytes as u64)?
                }
                rmp::Marker::Str16 => {
                    let bytes = read
                        .read_u16::<byteorder::BigEndian>()
                        .map_err(DecodingError::DecodingFailedWhileSkipping)
                        .map_err(Error::NeovimMessageDecodingFailed)?;
                    skip_bytes(read, bytes as u64)?
                }
                rmp::Marker::Str32 => {
                    let bytes = read
                        .read_u32::<byteorder::BigEndian>()
                        .map_err(DecodingError::DecodingFailedWhileSkipping)
                        .map_err(Error::NeovimMessageDecodingFailed)?;
                    skip_bytes(read, bytes as u64)?
                }
                rmp::Marker::Bin8 => {
                    let bytes = read
                        .read_u8()
                        .map_err(DecodingError::DecodingFailedWhileSkipping)
                        .map_err(Error::NeovimMessageDecodingFailed)?;
                    skip_bytes(read, bytes as u64)?
                }
                rmp::Marker::Bin16 => {
                    let bytes = read
                        .read_u16::<byteorder::BigEndian>()
                        .map_err(DecodingError::DecodingFailedWhileSkipping)
                        .map_err(Error::NeovimMessageDecodingFailed)?;
                    skip_bytes(read, bytes as u64)?
                }
                rmp::Marker::Bin32 => {
                    let bytes = read
                        .read_u32::<byteorder::BigEndian>()
                        .map_err(DecodingError::DecodingFailedWhileSkipping)
                        .map_err(Error::NeovimMessageDecodingFailed)?;
                    skip_bytes(read, bytes as u64)?
                }
                rmp::Marker::FixArray(objects) => {
                    count += objects as u32;
                }
                rmp::Marker::Array16 => {
                    let objects = read
                        .read_u16::<byteorder::BigEndian>()
                        .map_err(DecodingError::DecodingFailedWhileSkipping)
                        .map_err(Error::NeovimMessageDecodingFailed)?;
                    count += objects as u32;
                }
                rmp::Marker::Array32 => {
                    let objects = read
                        .read_u32::<byteorder::BigEndian>()
                        .map_err(DecodingError::DecodingFailedWhileSkipping)
                        .map_err(Error::NeovimMessageDecodingFailed)?;
                    count += objects;
                }
                rmp::Marker::FixMap(entries) => {
                    count += 2 * entries as u32;
                }
                rmp::Marker::Map16 => {
                    let entries = read
                        .read_u16::<byteorder::BigEndian>()
                        .map_err(DecodingError::DecodingFailedWhileSkipping)
                        .map_err(Error::NeovimMessageDecodingFailed)?;
                    count += 2 * entries as u32;
                }
                rmp::Marker::Map32 => {
                    let entries = read
                        .read_u32::<byteorder::BigEndian>()
                        .map_err(DecodingError::DecodingFailedWhileSkipping)
                        .map_err(Error::NeovimMessageDecodingFailed)?;
                    count += 2 * entries;
                }
                rmp::Marker::FixExt1 => skip_bytes(read, 2)?,
                rmp::Marker::FixExt2 => skip_bytes(read, 3)?,
                rmp::Marker::FixExt4 => skip_bytes(read, 5)?,
                rmp::Marker::FixExt8 => skip_bytes(read, 9)?,
                rmp::Marker::FixExt16 => skip_bytes(read, 17)?,
                rmp::Marker::Ext8 => {
                    let bytes = read
                        .read_u8()
                        .map_err(DecodingError::DecodingFailedWhileSkipping)
                        .map_err(Error::NeovimMessageDecodingFailed)?;
                    skip_bytes(read, 1 + bytes as u64)?
                }
                rmp::Marker::Ext16 => {
                    let bytes = read
                        .read_u16::<byteorder::BigEndian>()
                        .map_err(DecodingError::DecodingFailedWhileSkipping)
                        .map_err(Error::NeovimMessageDecodingFailed)?;
                    skip_bytes(read, 1 + bytes as u64)?
                }
                rmp::Marker::Ext32 => {
                    let bytes = read
                        .read_u32::<byteorder::BigEndian>()
                        .map_err(DecodingError::DecodingFailedWhileSkipping)
                        .map_err(Error::NeovimMessageDecodingFailed)?;
                    skip_bytes(read, 1 + bytes as u64)?
                }
                rmp::Marker::Reserved => {}
            }
        }
        Ok(())
    }

    fn skip_bytes<R>(read: &mut BufReader<R>, count: u64) -> Result<(), Error>
    where
        R: Read,
    {
        std::io::copy(&mut read.take(count), &mut std::io::sink())
            .map_err(DecodingError::DecodingFailedWhileSkipping)
            .map_err(Error::NeovimMessageDecodingFailed)?;
        Ok(())
    }

    fn read_string<R>(read: &mut BufReader<R>) -> Result<String, Error>
    where
        R: Read,
    {
        let len = rmp::decode::read_str_len(read).map_err(val_err)?;
        let mut buffer = vec![0; len as usize];
        read.read_exact(&mut buffer)
            .map_err(DecodingError::DecodingFailedWhileSkipping)
            .map_err(Error::NeovimMessageDecodingFailed)?;
        std::string::String::from_utf8(buffer).map_err(|err| {
            Error::NeovimMessageDecodingFailed(
                DecodingError::DecodingFailedWithInvalidUtf8(err.utf8_error()),
            )
        })
    }

    fn read_buf<R>(read: &mut BufReader<R>) -> Result<u8, Error>
    where
        R: Read,
    {
        let (_, buf) = rmp::decode::read_fixext1(read).map_err(val_err)?;
        Ok(buf as u8)
    }

    fn marker_err(error: rmp::decode::MarkerReadError) -> Error {
        match error {
            rmp::decode::MarkerReadError(sub_error) => {
                Error::NeovimMessageDecodingFailed(
                    DecodingError::DecodingFailedWithInvalidMarkerRead(
                        sub_error,
                    ),
                )
            }
        }
    }

    fn val_err(error: rmp::decode::ValueReadError) -> Error {
        match error {
            rmp::decode::ValueReadError::InvalidMarkerRead(sub_error) => {
                Error::NeovimMessageDecodingFailed(
                    DecodingError::DecodingFailedWithInvalidMarkerRead(
                        sub_error,
                    ),
                )
            }
            rmp::decode::ValueReadError::InvalidDataRead(sub_error) => {
                Error::NeovimMessageDecodingFailed(
                    DecodingError::DecodingFailedWithInvalidDataRead(sub_error),
                )
            }
            rmp::decode::ValueReadError::TypeMismatch(sub_error) => {
                Error::NeovimMessageDecodingFailed(
                    DecodingError::DecodingFailedWithTypeMismatch(sub_error),
                )
            }
        }
    }

    fn num_val_err(error: rmp::decode::NumValueReadError) -> Error {
        match error {
            rmp::decode::NumValueReadError::InvalidMarkerRead(sub_error) => {
                Error::NeovimMessageDecodingFailed(
                    DecodingError::DecodingFailedWithInvalidMarkerRead(
                        sub_error,
                    ),
                )
            }
            rmp::decode::NumValueReadError::InvalidDataRead(sub_error) => {
                Error::NeovimMessageDecodingFailed(
                    DecodingError::DecodingFailedWithInvalidDataRead(sub_error),
                )
            }
            rmp::decode::NumValueReadError::TypeMismatch(sub_error) => {
                Error::NeovimMessageDecodingFailed(
                    DecodingError::DecodingFailedWithTypeMismatch(sub_error),
                )
            }
            rmp::decode::NumValueReadError::OutOfRange => {
                Error::NeovimMessageDecodingFailed(
                    DecodingError::DecodingFailedWithOutOfRange,
                )
            }
        }
    }

    fn str_err(error: rmp::decode::DecodeStringError) -> Error {
        match error {
            rmp::decode::DecodeStringError::InvalidMarkerRead(sub_error) => {
                Error::NeovimMessageDecodingFailed(
                    DecodingError::DecodingFailedWithInvalidMarkerRead(
                        sub_error,
                    ),
                )
            }
            rmp::decode::DecodeStringError::InvalidDataRead(sub_error) => {
                Error::NeovimMessageDecodingFailed(
                    DecodingError::DecodingFailedWithInvalidDataRead(sub_error),
                )
            }
            rmp::decode::DecodeStringError::TypeMismatch(sub_error) => {
                Error::NeovimMessageDecodingFailed(
                    DecodingError::DecodingFailedWithTypeMismatch(sub_error),
                )
            }
            rmp::decode::DecodeStringError::BufferSizeTooSmall(sub_error) => {
                Error::NeovimMessageDecodingFailed(
                    DecodingError::DecodingFailedWithBufferSizeTooSmall(
                        sub_error,
                    ),
                )
            }
            rmp::decode::DecodeStringError::InvalidUtf8(_, sub_error) => {
                Error::NeovimMessageDecodingFailed(
                    DecodingError::DecodingFailedWithInvalidUtf8(sub_error),
                )
            }
        }
    }

    pub struct NeovimDriver<W> {
        write: Arc<Mutex<W>>,
    }

    impl<W> NeovimDriver<W>
    where
        W: Write,
    {
        pub(crate) fn apply_edits(
            &self,
            refactor: Vec<Edit>,
        ) -> Result<(), Error> {
            println!("REFACTOR: {:?}", refactor);
            let mut write_guard = crate::lock(&self.write);
            let write = write_guard.deref_mut();

            rmp::encode::write_array_len(write, 3).unwrap(); // msgpack envelope
            rmp::encode::write_i8(write, 2).unwrap();
            write_str(write, "nvim_call_atomic");

            rmp::encode::write_array_len(write, 1).unwrap(); // nvim_call_atomic args

            rmp::encode::write_array_len(write, refactor.len() as u32).unwrap(); // calls array
            let buf = 0; // TODO: use a real value here.
            for edit in refactor {
                let start = edit.input_edit.start_position;
                let end = edit.input_edit.old_end_position;

                rmp::encode::write_array_len(write, 2).unwrap(); // call tuple
                write_str(write, "nvim_buf_set_text");

                rmp::encode::write_array_len(write, 6).unwrap(); // nvim_buf_set_text args
                rmp::encode::write_u8(write, buf).unwrap();
                rmp::encode::write_u64(write, start.row as u64).unwrap();
                rmp::encode::write_u64(write, start.column as u64).unwrap();
                rmp::encode::write_u64(write, end.row as u64).unwrap();
                rmp::encode::write_u64(write, end.column as u64).unwrap();

                rmp::encode::write_array_len(write, 1).unwrap(); // array of lines
                write_str(write, &edit.new_bytes);
            }
            Ok(())
        }
    }

    pub fn write_str<W>(write: &mut W, str: &str)
    where
        W: Write,
    {
        let bytes = str.as_bytes();
        rmp::encode::write_str_len(write, bytes.len() as u32).unwrap();
        write.write_all(bytes).unwrap();
    }
}

// A thread sync structure similar to Haskell's MVar. A variable, potentially
// empty, that can be shared across threads.
mod mvar {
    use crate::lock;
    use std::sync::{Condvar, Mutex};

    pub struct MVar<T> {
        val: Mutex<Option<T>>,
        full_condvar: Condvar,
    }

    impl<T> MVar<T> {
        // Create a new empty MVar.
        pub fn new() -> MVar<T> {
            MVar {
                val: Mutex::new(None),
                full_condvar: Condvar::new(),
            }
        }

        // Write a value to the MVar, possibly overwriting a previous value.
        pub fn write(&self, new: T) {
            let mut val = lock(&self.val);
            *val = Some(new);
            self.full_condvar.notify_all();
        }

        // Write a value to an empty MVar. Returns true if the write succeeded,
        // or false if there already was a value in the MVar.
        pub fn try_put(&self, new: T) -> bool {
            let mut val = lock(&self.val);
            match val.as_ref() {
                None => {
                    *val = Some(new);
                    self.full_condvar.notify_all();
                    true
                }
                Some(_) => false,
            }
        }

        // Take the value from the MVar. If the MVar does not contain a value,
        // block until it does.
        pub fn take(&self) -> T {
            let mut opt_val = crate::lock(&self.val);
            loop {
                match opt_val.take() {
                    None {} => {
                        opt_val = self.full_condvar.wait(opt_val).unwrap()
                    }
                    Some(val) => return val,
                }
            }
        }

        // Take the value from an MVar if it has one, leaving the MVar empty.
        pub fn try_take(&self) -> Option<T> {
            lock(&self.val).take()
        }

        // Clone the current value in the MVar and return it.
        pub fn try_read(&self) -> Option<T>
        where
            T: Clone,
        {
            crate::lock(&self.val).clone()
        }
    }
}

// A stack (last in, first out) with a maximum size. If a push would ever make
// the stack grow beyond its capacity, then the stack forgets its oldest element
// before pushing the new element.
mod sized_stack {
    use std::collections::VecDeque;
    use std::sync::{Condvar, Mutex};

    pub struct SizedStack<T> {
        capacity: usize,
        items: Mutex<VecDeque<T>>,
        new_item_condvar: Condvar,
    }

    impl<T> SizedStack<T> {
        pub fn with_capacity(capacity: usize) -> SizedStack<T> {
            SizedStack {
                capacity,
                items: Mutex::new(VecDeque::with_capacity(capacity)),
                new_item_condvar: Condvar::new(),
            }
        }

        // Push an item on the stack.
        pub fn push(&self, item: T) {
            let mut items = crate::lock(&self.items);
            items.truncate(self.capacity - 1);
            items.push_front(item);
            self.new_item_condvar.notify_all();
        }

        // Pop an item of the stack. This function blocks until an item becomes
        // available.
        pub fn pop(&self) -> T {
            let mut items = crate::lock(&self.items);
            loop {
                match items.pop_front() {
                    None => {
                        items = self.new_item_condvar.wait(items).unwrap();
                    }
                    Some(item) => return item,
                }
            }
        }
    }
}

fn does_snapshot_compile(snapshot: &SourceFileSnapshot) -> Result<bool, Error> {
    // Write lates code to temporary file. We don't compile the original source
    // file, because the version stored on disk is likely ahead or behind the
    // version in the editor.
    let mut temp_path =
        snapshot.file_data.project_root.join("elm-stuff/elm-pair");
    std::fs::create_dir_all(&temp_path)
        .map_err(Error::CompilationFailedToCreateTempDir)?;
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

mod compilation_thread {
    use crate::mvar::MVar;
    use crate::sized_stack::SizedStack;
    use crate::{Error, SourceFileSnapshot};
    use std::sync::Arc;

    pub struct CompilationCandidates {
        candidates: Arc<SizedStack<SourceFileSnapshot>>,
        last_submitted_revision: Option<usize>,
    }

    impl CompilationCandidates {
        pub(crate) fn push(&mut self, snapshot: SourceFileSnapshot) {
            if !is_new_revision(&mut self.last_submitted_revision, &snapshot) {
                return;
            }
            self.candidates.push(snapshot);
        }
    }

    fn run_compilation_loop<F>(
        candidates: Arc<SizedStack<SourceFileSnapshot>>,
        new_diff_available: Arc<MVar<()>>,
        last_compiling_code: Arc<MVar<SourceFileSnapshot>>,
        compiles: F,
    ) -> Result<(), Error>
    where
        F: Fn(&SourceFileSnapshot) -> Result<bool, Error>,
    {
        let mut last_validated_revision = None;
        loop {
            let snapshot = candidates.pop();
            if !is_new_revision(&mut last_validated_revision, &snapshot) {
                continue;
            }
            if compiles(&snapshot)? {
                last_compiling_code.write(snapshot);
                new_diff_available.write(());
            }
        }
    }

    fn is_new_revision(
        last_checked_revision: &mut Option<usize>,
        code: &SourceFileSnapshot,
    ) -> bool {
        let is_new = match last_checked_revision {
            None => true,
            Some(old) => code.revision > *old,
        };
        if is_new {
            *last_checked_revision = Some(code.revision);
        }
        is_new
    }

    pub(crate) fn run(
        error_var: Arc<MVar<Error>>,
        new_diff_available: Arc<MVar<()>>,
    ) -> (CompilationCandidates, Arc<MVar<SourceFileSnapshot>>) {
        let candidates = Arc::new(SizedStack::with_capacity(
            crate::MAX_COMPILATION_CANDIDATES,
        ));
        let compilation_candidates = CompilationCandidates {
            candidates: candidates.clone(),
            last_submitted_revision: None,
        };
        let last_compiling_code = Arc::new(MVar::new());
        let last_compiling_code_clone = last_compiling_code.clone();
        std::thread::spawn(move || {
            crate::report_error(
                error_var,
                run_compilation_loop(
                    candidates,
                    new_diff_available,
                    last_compiling_code,
                    crate::does_snapshot_compile,
                ),
            );
        });
        (compilation_candidates, last_compiling_code_clone)
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
        let separator =
            "\n\n".to_owned() + prefix + "=== expected output below ===\n";
        let contents = assert_ok(std::fs::read_to_string(path));
        match contents.split_once(&separator) {
            None => {
                let mut file = assert_ok(
                    std::fs::OpenOptions::new().append(true).open(path),
                );
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
    use crate::{Edit, ElmChange};
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
                let module_name =
                    crate::simulation::snake_to_camel(stringify!($name));
                path.push(module_name + ".elm");
                println!("Run simulation {:?}", &path);
                crate::simulation::run_simulation_test(&path);
            }
        };
    }
    pub use simulation_test;

    struct Simulation {
        msgs: VecDeque<Msg>,
    }

    pub fn run_simulation_test(path: &Path) {
        match run_simulation_test_helper(path) {
            Err(err) => panic!("simulation failed with: {:?}", err),
            Ok(val) => assert_eq_answer_in(&format!("{:?}", val), path),
        }
    }

    fn run_simulation_test_helper(
        path: &Path,
    ) -> Result<Option<ElmChange>, Error> {
        let simulation = Simulation::from_file(path)?;
        let mut latest_code = None;
        let mut last_compiling_code = None;
        let diff_iterator = simulation.msgs.into_iter().filter_map(|msg| {
            let res = {
                match msg {
                    Msg::CompilationSucceeded => {
                        last_compiling_code = latest_code.clone();
                        Ok(())
                    }
                    Msg::ReceivedEditorEvent(edit) => {
                        crate::handle_edit_event(&mut latest_code, edit)
                    }
                }
            };
            if let Err(err) = res {
                return Some(Err(err));
            }
            match (last_compiling_code.clone(), latest_code.clone()) {
                (Some(old), Some(new)) => {
                    Some(Ok(crate::SourceFileDiff { old, new }))
                }
                _ => None,
            }
        });
        diff_iterator
            .map(|res| res.map(|diff| crate::analyze_diff(&diff)))
            .last()
            .transpose()
            .map(Option::flatten)
            .map_err(Error::RunningSimulationFailed)
    }

    fn find_start_simulation_script<I>(
        lines: &mut I,
    ) -> Result<(Vec<u8>, usize), Error>
    where
        I: Iterator<Item = Result<String, Error>>,
    {
        let mut code: Vec<u8> = Vec::new();
        loop {
            let line = match lines.next() {
                None => {
                    return Err(Error::FromFileFailedNoStartSimulationFound)
                }
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

    enum Msg {
        ReceivedEditorEvent(Edit),
        CompilationSucceeded,
    }

    impl Simulation {
        fn from_file(path: &Path) -> Result<Simulation, Error> {
            let file =
                std::fs::File::open(path).map_err(Error::FromFileOpenFailed)?;
            let mut lines = std::io::BufReader::new(file)
                .lines()
                .map(|line| line.map_err(Error::FromFileReadingLineFailed));
            let (code, simulation_script_padding) =
                find_start_simulation_script(&mut lines)?;
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
                        let line = line_str.parse().map_err(|_| {
                            Error::CannotParseLineNumber(line.to_string())
                        })?;
                        builder = builder.move_cursor(line, &strs.join(" "))?
                    }
                    ["INSERT", strs @ ..] => {
                        builder = builder.insert(&strs.join(" "))
                    }
                    ["DELETE", strs @ ..] => {
                        builder = builder.delete(&strs.join(" "))?
                    }
                    ["COMPILATION", "SUCCEEDS"] => {
                        builder = builder.compilation_succeeds()
                    }
                    _ => return Err(Error::CannotParseSimulationLine(line)),
                };
            }
            Ok(builder.finish())
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
                new_bytes: std::string::String::from_utf8(
                    initial_bytes.clone(),
                )
                .unwrap(),
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
                    Some(10 /* \n */) => {
                        return Err(Error::MoveCursorDidNotFindWordOnLine)
                    }
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
            let range =
                self.current_position..(self.current_position + bytes.len());
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
            Simulation { msgs: self.msgs }
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
