use core::ops::Range;
use sized_stack::SizedStack;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::{Arc, Condvar, Mutex};
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
    let compilation_thread_state = Arc::new(CompilationThreadState::new());
    let compilation_thread_state_clone = Arc::clone(&compilation_thread_state);
    // We're using a sync_channel of size 1 here because:
    // - Size 0 would mean sending blocks until the receiver is requesting a new
    //   message. This is not okay, because we want to be able to queue up a
    //   Msg::CompilationSucceeded for after current reparsing succeeds.
    // - Size >1 means we start buffering editor events in this program. I want
    //   to try to stay ouf the buffering business and leave it to the OS, until
    //   I see a benefit in doing it in this program (I don't currently).
    let (sender, receiver) = sync_channel(1);
    let sender_clone = sender.clone();
    std::thread::spawn(move || {
        report_error(
            &sender_clone,
            run_compilation_thread(&sender_clone, compilation_thread_state_clone),
        );
    });
    std::thread::spawn(move || {
        report_error(&sender, run_editor_listener_thread(&sender));
    });
    handle_msgs(compilation_thread_state, &mut receiver.iter())
}

struct CompilationThreadState {
    // A stack of source file snapshots the compilation thread should attempt
    // to compile.
    candidates: Mutex<SizedStack<SourceFileSnapshot>>,
    // A condvar triggered whenever a new candidate is added to `candidates`.
    new_candidate_condvar: Condvar,
    // The compilation thread uses this field to communicate compilation
    // successes back to the main thread. We're not using a channel for this
    // because we don't want to block either on sending or receivning
    // compilation results. A newer compilation result should overwrite an older
    // one.
    last_compilation_success: Mutex<Option<SourceFileSnapshot>>,
}

impl CompilationThreadState {
    fn new() -> CompilationThreadState {
        CompilationThreadState {
            last_compilation_success: Mutex::new(None),
            new_candidate_condvar: Condvar::new(),
            candidates: Mutex::new(SizedStack::with_capacity(MAX_COMPILATION_CANDIDATES)),
        }
    }
}

#[derive(Clone)]
struct SourceFileSnapshot {
    // Once calculated the file_data never changes. We wrap it in an `Arc` to
    // avoid needing to copy it.
    file_data: Arc<FileData>,
    // Vec offers a '.splice()' operation we need to replace bits of the vector
    // in response to updates made in the editor. This is probably not super
    // efficient though, so we should look for a better datastructure here.
    //
    // TODO: look into better data structures for splice-heavy workloads.
    bytes: Vec<u8>,
    // The tree-sitter concrete syntax tree representing the code in `bytes`.
    // This tree by itself is not enough to recover the source code, which is
    // why we also keep the original source code in `bytes`.
    tree: Tree,
    // A number that gets incremented each time we change the source code.
    revision: u64,
}

struct FileData {
    // Absolute path to this source file.
    _path: PathBuf,
    // Root of the Elm project containing this source file.
    project_root: PathBuf,
    // Absolute path to the `elm` compiler.
    elm_bin: PathBuf,
}

struct SourceFileState {
    last_compiling_version: Option<SourceFileSnapshot>,
    latest_code: SourceFileSnapshot,
}

// The event type central to this application. The main thread of the program
// will process these one-at-a-time.
enum Msg {
    ThreadFailedWithError(Error),
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
    new_bytes: Vec<u8>,
}

#[derive(Debug)]
enum Error {
    DidNotFindElmBinaryOnPath,
    CouldNotReadCurrentWorkingDirectory(std::io::Error),
    DidNotFindPathEnvVar,
    NoElmJsonFoundInAnyAncestorDirectoryOf(PathBuf),
    FoundPoisonedMutexWhileUpdatingLastCompilingVersion,
    FoundPoisonedMutexWhileAddingCompilationCandidate,
    FoundPoisonedMutexWhileWritingLastCompilationSuccess,
    FoundPoisonedMutexWhileReadingCompilationCandidates,
    FoundPoisonedMutexWhileWaitingForCompilationCandidates,
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
    let changed_code: String = changed_code; // Add type-annotation for changed_code.
    Ok(Edit {
        file,
        input_edit,
        new_bytes: changed_code.into_bytes(),
    })
}

fn handle_msgs<I>(
    compilation_thread_state: Arc<CompilationThreadState>,
    msgs: &mut I,
) -> Result<(), Error>
where
    I: Iterator<Item = Msg>,
{
    let mut state = match initial_state(&compilation_thread_state, msgs)? {
        None => return Ok(()),
        Some(state) => state,
    };
    debug_print_latest_tree(&state);

    // Subsequent parses of a file.
    for msg in msgs {
        refresh_last_compiling_version(&mut state, &compilation_thread_state)?;
        match msg {
            Msg::ThreadFailedWithError(err) => return Err(err),
            Msg::CompilationSucceeded => reparse_tree(&mut state)?,
            Msg::ReceivedEditorEvent(edit) => apply_edit(&mut state, edit)?,
        }
        if let Some(last_compiling_version) = &state.last_compiling_version {
            let mut last_compiling_version_cursor = last_compiling_version.tree.walk();
            let mut latest_cursor = state.latest_code.tree.walk();
            let changes = diff_trees(
                last_compiling_version,
                &state.latest_code,
                &mut last_compiling_version_cursor,
                &mut latest_cursor,
            );
            if !changes.is_empty() {
                let elm_change = interpret_change(&changes);
                println!("CHANGE: {:?}", elm_change);
            }
        }
        if !state.latest_code.tree.root_node().has_error() {
            add_compilation_candidate(&state.latest_code, &compilation_thread_state)?;
        }
    }
    Ok(())
}

fn initial_state<I>(
    compilation_thread_state: &Arc<CompilationThreadState>,
    msgs: &mut I,
) -> Result<Option<SourceFileState>, Error>
where
    I: Iterator<Item = Msg>,
{
    let mut first_edit = None;
    for msg in msgs {
        // First event returns the initial state.
        match msg {
            Msg::ReceivedEditorEvent(edit) => {
                first_edit = Some(edit);
                break;
            }
            Msg::ThreadFailedWithError(err) => return Err(err),
            Msg::CompilationSucceeded => {}
        };
    }
    let Edit {
        file, new_bytes, ..
    } = match first_edit {
        None => return Ok(None),
        Some(edit) => edit,
    };
    let tree = parse(None, &new_bytes)?;
    let file_data = Arc::new(FileData {
        elm_bin: find_executable("elm")?,
        project_root: find_project_root(&file)?.to_path_buf(),
        _path: file,
    });
    let code = SourceFileSnapshot {
        tree,
        bytes: new_bytes,
        file_data,
        revision: 0,
    };
    add_compilation_candidate(&code, compilation_thread_state)?;
    Ok(Some(SourceFileState {
        last_compiling_version: None,
        latest_code: code,
    }))
}

fn refresh_last_compiling_version(
    state: &mut SourceFileState,
    compilation_thread_state: &CompilationThreadState,
) -> Result<(), Error> {
    let mut last_compilation_success = compilation_thread_state
        .last_compilation_success
        .lock()
        .map_err(|_| Error::FoundPoisonedMutexWhileUpdatingLastCompilingVersion)?;

    if let Some(snapshot) = std::mem::replace(&mut *last_compilation_success, None) {
        state.last_compiling_version = Some(snapshot);
    }
    Ok(())
}

fn add_compilation_candidate(
    code: &SourceFileSnapshot,
    compilation_thread_state: &CompilationThreadState,
) -> Result<(), Error> {
    let snapshot = code.clone();
    {
        let mut candidates = compilation_thread_state
            .candidates
            .lock()
            .map_err(|_| Error::FoundPoisonedMutexWhileAddingCompilationCandidate)?;
        candidates.push(snapshot);
        compilation_thread_state.new_candidate_condvar.notify_all();
    }
    Ok(())
}

fn report_error<I>(sender: &SyncSender<Msg>, result: Result<I, Error>) {
    match result {
        Ok(_) => {}
        Err(err) => {
            if let Err(send_err) = sender.send(Msg::ThreadFailedWithError(err)) {
                panic!( "Thread failed with error, and then also wasn't able to communicate this to the main thread. Send error: {:?}",  send_err);
            }
        }
    }
}

fn run_compilation_thread(
    sender: &SyncSender<Msg>,
    compilation_thread_state: Arc<CompilationThreadState>,
) -> Result<(), Error> {
    let mut last_compiled_revision = None;
    loop {
        let candidate = pop_latest_candidate(&compilation_thread_state)?;
        if let Some(revision) = last_compiled_revision {
            if candidate.revision <= revision {
                // We've already compiled newer snapshots than this, so ignore.
                continue;
            }
        }

        if does_snapshot_compile(&candidate)? {
            last_compiled_revision = Some(candidate.revision);
            let mut last_compilation_success = compilation_thread_state
                .last_compilation_success
                .lock()
                .map_err(|_| Error::FoundPoisonedMutexWhileWritingLastCompilationSuccess)?;
            *last_compilation_success = Some(candidate);
            // Let the main thread know it should reparse. If sending this fails
            // we asume that's because a ReceivedEditorEvent got there first.
            // That's okay, because those events cause reparsing too.
            sender
                .try_send(Msg::CompilationSucceeded)
                .map_err(|_| Error::SendingMsgFromCompilationThreadFailed)?;
        }
    }
}

fn run_editor_listener_thread(sender: &SyncSender<Msg>) -> Result<(), Error> {
    let fifo_path = "/tmp/elm-pair";
    nix::unistd::mkfifo(fifo_path, nix::sys::stat::Mode::S_IRWXU)
        .map_err(Error::FifoCreationFailed)?;
    let fifo = std::fs::File::open(fifo_path).map_err(Error::FifoOpeningFailed)?;
    let buf_reader = std::io::BufReader::new(fifo).lines();
    for line in buf_reader {
        let edit = parse_editor_event(&line.map_err(Error::FifoLineReadingFailed)?)?;
        sender
            .send(Msg::ReceivedEditorEvent(edit))
            .map_err(|_| Error::SendingMsgFromEditorListenerThreadFailed)?;
    }
    Ok(())
}

fn pop_latest_candidate(
    compilation_thread_state: &CompilationThreadState,
) -> Result<SourceFileSnapshot, Error> {
    let mut candidates = compilation_thread_state
        .candidates
        .lock()
        .map_err(|_| Error::FoundPoisonedMutexWhileReadingCompilationCandidates)?;
    loop {
        match candidates.pop() {
            None => {
                candidates = compilation_thread_state
                    .new_candidate_condvar
                    .wait(candidates)
                    .map_err(|_| Error::FoundPoisonedMutexWhileWaitingForCompilationCandidates)?;
            }
            Some(next_candidate) => return Ok(next_candidate),
        }
    }
}

fn does_snapshot_compile(snapshot: &SourceFileSnapshot) -> Result<bool, Error> {
    // Write lates code to temporary file. We don't compile the original source
    // file, because the version stored on disk is likely ahead or behind the
    // version in the editor.
    let mut temp_path = snapshot.file_data.project_root.join("elm-stuff/elm-pair");
    std::fs::create_dir_all(&temp_path).map_err(Error::CompilationFailedToCreateTempDir)?;
    temp_path.push("Temp.elm");
    std::fs::write(&temp_path, &snapshot.bytes)
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

fn apply_edit(state: &mut SourceFileState, edit: Edit) -> Result<(), Error> {
    println!("edit: {:?}", edit.input_edit);
    state.latest_code.tree.edit(&edit.input_edit);
    state.latest_code.revision += 1;
    let range = edit.input_edit.start_byte..edit.input_edit.old_end_byte;
    state.latest_code.bytes.splice(range, edit.new_bytes);
    reparse_tree(state)
}

fn reparse_tree(state: &mut SourceFileState) -> Result<(), Error> {
    let new_tree = parse(Some(&state.latest_code.tree), &state.latest_code.bytes)?;
    state.latest_code.tree = new_tree;
    debug_print_latest_tree(state);
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
        ([], [(",", _), ("field_type", after)]) => Some(ElmChange::FieldAdded(debug_code_slice(
            changes.new_code,
            &after.byte_range(),
        ))),

        ([], [("field_type", after), (",", _)]) => Some(ElmChange::FieldAdded(debug_code_slice(
            changes.new_code,
            &after.byte_range(),
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

struct TreeChanges<'a> {
    old_code: &'a SourceFileSnapshot,
    new_code: &'a SourceFileSnapshot,
    old_removed: Vec<Node<'a>>,
    new_added: Vec<Node<'a>>,
}

impl<'a> TreeChanges<'a> {
    fn is_empty(&self) -> bool {
        self.old_removed.is_empty() && self.new_added.is_empty()
    }
}

fn diff_trees<'a>(
    old_code: &'a SourceFileSnapshot,
    new_code: &'a SourceFileSnapshot,
    old: &'a mut TreeCursor,
    new: &'a mut TreeCursor,
) -> TreeChanges<'a> {
    loop {
        match goto_first_changed_sibling(old_code, new_code, old, new) {
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
            count_changed_siblings(old_code, new_code, old, new);

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
// TODO: compare u8 array slices here instead of parsing to string.
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
    std::string::String::from_utf8(code.bytes[range.start..range.end].to_vec()).unwrap()
}

// TODO: reuse parser.
fn parse(prev_tree: Option<&Tree>, code: &[u8]) -> Result<Tree, Error> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(tree_sitter_elm::language())
        .map_err(Error::TreeSitterSettingLanguageFailed)?;
    match parser.parse(code, prev_tree) {
        None => Err(Error::TreeSitterParsingFailed),
        Some(tree) => Ok(tree),
    }
}

fn debug_print_latest_tree(state: &SourceFileState) {
    let mut cursor = state.latest_code.tree.walk();
    debug_print_tree_helper(&state.latest_code, 0, &mut cursor);
    println!();
}

fn debug_print_tree_helper(
    code: &SourceFileSnapshot,
    indent: usize,
    cursor: &mut tree_sitter::TreeCursor,
) {
    let node = cursor.node();
    println!(
        "{}[{} {:?}] {:?}{}",
        "  ".repeat(indent),
        node.kind(),
        node.start_position().row + 1,
        debug_code_slice(code, &node.byte_range()),
        if node.has_changes() { " (changed)" } else { "" },
    );
    if cursor.goto_first_child() {
        debug_print_tree_helper(code, indent + 1, cursor);
        cursor.goto_parent();
    }
    if cursor.goto_next_sibling() {
        debug_print_tree_helper(code, indent, cursor);
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

#[cfg(test)]
mod tests {
    use crate::simulation::run_simulation_test;
    use std::path::Path;

    #[test]
    fn first_test() {
        run_simulation_test(Path::new("./tests/FirstTest.elm"));
    }
}

// A module to support tests of the diffing logic by running simulations against
// it.
#[cfg(test)]
mod simulation {
    use crate::{CompilationThreadState, Edit, Msg};
    use std::collections::VecDeque;
    use std::io::BufRead;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use tree_sitter::{InputEdit, Point};

    pub fn run_simulation_test(path: &Path) {
        if let Err(err) = run_simulation_test_helper(path) {
            panic!("simulation failed with: {:?}", err);
        }
    }

    fn run_simulation_test_helper(path: &Path) -> Result<(), Error> {
        let mut simulation = Simulation::from_file(path)?;
        crate::handle_msgs(simulation.compilation_thread_state.clone(), &mut simulation)
            .map_err(Error::RunningSimulationFailed)?;
        Ok(())
    }

    struct Simulation {
        compilation_thread_state: Arc<CompilationThreadState>,
        msgs: VecDeque<Msg>,
    }

    impl Iterator for Simulation {
        type Item = Msg;

        fn next(&mut self) -> Option<Self::Item> {
            // Bunch of .unwrap()'s here. We're bound by the iterator contract
            // here, and it's preventing us from communicating errors upwards.
            // We could return `None` instead of panicing, but that would hide
            // problems. We could not use an interator, which might be nicer.
            // Since this is test code let's let it slide for now.
            self.msgs.pop_front().map(|msg| {
                if let Msg::CompilationSucceeded = msg {
                    let last_snapshot = {
                        self.compilation_thread_state
                            .candidates
                            .lock()
                            .unwrap()
                            .pop()
                            .unwrap()
                    };
                    let mut last_compilation_success = self
                        .compilation_thread_state
                        .last_compilation_success
                        .lock()
                        .unwrap();
                    *last_compilation_success = Some(last_snapshot);
                }
                msg
            })
        }
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
                new_bytes: initial_bytes.clone(),
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
            self.msgs.push_back(Msg::ReceivedEditorEvent(Edit {
                file: self.file.clone(),
                new_bytes: Vec::new(),
                input_edit: InputEdit {
                    start_byte: range.start,
                    old_end_byte: range.start,
                    new_end_byte: range.end,
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
                    new_bytes: Vec::new(),
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
            Simulation {
                compilation_thread_state: Arc::new(CompilationThreadState::new()),
                msgs: self.msgs,
            }
        }
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
