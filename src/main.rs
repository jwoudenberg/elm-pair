use core::ops::Range;
use mvar::MVar;
use ropey::Rope;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, SendError, Sender, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard};
use tree_sitter::{InputEdit, Node, Tree};

mod analysis_thread;
mod compilation_thread;
mod editor_listener_thread;
mod neovim;

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
    // Create channels for inter-thread communication.
    let (analysis_sender, analysis_receiver) = std::sync::mpsc::channel();
    let (compilation_sender, compilation_receiver) = std::sync::mpsc::channel();
    // We could send code updates over above channels too, but don't because:
    // 1. It would require cloning a snapshot on every change, which is often.
    // 2. By using a mutex we can block analysis of a snapshot currently being
    //    changed, meaning we already know it's no longer current.
    let latest_code = Arc::new(MVar::new_empty());

    // Start editor listener thread.
    let latest_code_for_editor_listener = latest_code.clone();
    let analysis_sender_for_editor_listener = analysis_sender.clone();
    spawn_thread(analysis_sender.clone(), || {
        editor_listener_thread::run(
            latest_code_for_editor_listener,
            compilation_sender,
            analysis_sender_for_editor_listener,
        )
    });

    // Start compilation thread.
    spawn_thread(analysis_sender.clone(), || {
        compilation_thread::run(compilation_receiver, analysis_sender)
    });

    // Main thread continues as analysis thread.
    analysis_thread::run(latest_code, analysis_receiver)
}

#[derive(Clone)]
struct SourceFileSnapshot {
    // A unique index identifying a source file open in an editor. We're not
    // using the file path for a couple of reasons:
    // - It's possible for the same file to be open in multiple editors with
    //   different unsaved changes each.
    // - A file path is stringy, so more expensive to copy.
    buffer: usize,
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
    // Root of the Elm project containing this source file.
    project_root: PathBuf,
    // Absolute path to the `elm` compiler.
    elm_bin: PathBuf,
}

// A change made by the user reported by the editor.
#[derive(Debug)]
struct Edit {
    // The buffer that was changed.
    buffer: usize,
    // A tree-sitter InputEdit value, describing what part of the file was changed.
    input_edit: InputEdit,
    // Bytes representing the new contents of the file at the location described
    // by `input_edit`.
    new_bytes: String,
}

fn byte_to_point(code: &Rope, byte: usize) -> tree_sitter::Point {
    let row = code.byte_to_line(byte);
    tree_sitter::Point {
        row,
        column: code.byte_to_char(byte) - code.line_to_char(row),
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
    NeovimMessageDecodingFailed(neovim::Error),
    CompilationFailedToCreateTempDir(std::io::Error),
    CompilationFailedToWriteCodeToTempFile(std::io::Error),
    CompilationFailedToRunElmMake(std::io::Error),
    TreeSitterParsingFailed,
    TreeSitterSettingLanguageFailed(tree_sitter::LanguageError),
    EditorRequestedNonExistingLocalCopy,
    FailedToSendMessage,
    NoFileDataStoredForBuffer(usize),
}

impl<T> From<SendError<T>> for Error {
    fn from(_err: SendError<T>) -> Error {
        Error::FailedToSendMessage
    }
}

fn spawn_thread<F>(error_channel: Sender<analysis_thread::Msg>, f: F)
where
    F: FnOnce() -> Result<(), Error>,
    F: Send + 'static,
{
    std::thread::spawn(move || {
        match f() {
            Ok(_) => {}
            Err(err) => {
                error_channel
                    .send(analysis_thread::Msg::ThreadFailed(err))
                    // If sending fails there's nothing more we can do to report
                    // this error, hence the unwrap().
                    .unwrap();
            }
        }
    });
}

fn debug_code_slice(code: &SourceFileSnapshot, range: &Range<usize>) -> String {
    let start = code.bytes.byte_to_char(range.start);
    let end = code.bytes.byte_to_char(range.end);
    code.bytes.slice(start..end).to_string()
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

trait MsgLoop<E> {
    type Msg;

    // This function is called for every new message that arrives. If we return
    // a `false` value at any point we stop the loop.
    //
    // This function doesn't return until it has processed at least one message,
    // and then until it has emptied the current contents of the queue.
    fn on_msg(&mut self, msg: Self::Msg) -> Result<bool, E>;

    // After each batch of messages this function is called once to do other
    // work. After it returns we wait for more messages.
    fn on_idle(&mut self) -> Result<(), E>;

    // --- Implementation, not for overriding ---
    fn start(&mut self, mut receiver: Receiver<Self::Msg>) -> Result<(), E> {
        while self.process_msg_batch(&mut receiver)? {
            self.on_idle()?;
        }
        Ok(())
    }

    fn process_msg_batch(
        &mut self,
        receiver: &mut Receiver<Self::Msg>,
    ) -> Result<bool, E> {
        match self.process_msg_batch_helper(receiver) {
            Ok(res) => res,
            Err(TryRecvError::Empty) => Ok(true),
            Err(TryRecvError::Disconnected) => Ok(false),
        }
    }

    fn process_msg_batch_helper(
        &mut self,
        receiver: &mut Receiver<Self::Msg>,
    ) -> Result<Result<bool, E>, TryRecvError> {
        let mut msg = receiver.recv()?;
        loop {
            let res = self.on_msg(msg);
            match res {
                Ok(do_continue) => {
                    if !do_continue {
                        return Ok(res);
                    }
                }
                Err(_) => return Ok(res),
            }
            msg = receiver.try_recv()?;
        }
    }
}

// An API for communicatating with an editor.
trait Editor {
    type Driver: EditorDriver;

    // Listen for changes to source files happening in the editor.
    fn listen<F, G>(
        self,
        load_code_copy: F,
        store_new_code: G,
    ) -> Result<(), Error>
    where
        F: FnMut(usize) -> Result<SourceFileSnapshot, Error>,
        G: FnMut(EditorEvent) -> Result<(), Error>;

    // Obtain an EditorDriver for sending commands to the editor.
    fn driver(&self) -> Self::Driver;
}

enum EditorEvent {
    OpenedNewSourceFile {
        buffer: usize,
        path: PathBuf,
        bytes: Rope,
    },
    ModifiedSourceFile {
        _buffer: usize,
        code: SourceFileSnapshot,
        edit: InputEdit,
    },
}

// An API for sending commands to an editor. This is defined as a trait to
// support different kinds of editors.
trait EditorDriver: 'static + Send {
    fn apply_edits(&self, edits: Vec<Edit>) -> Result<(), Error>;
}

// A thread sync structure similar to Haskell's MVar. A variable, potentially
// empty, that can be shared across threads. Doesn't (currently) do blocking
// reads and writes though, because this codebase doesn't need it.
mod mvar {
    use crate::lock;
    use std::sync::Mutex;

    pub struct MVar<T> {
        val: Mutex<Option<T>>,
    }

    impl<T> MVar<T> {
        pub fn new_empty() -> MVar<T> {
            MVar {
                val: Mutex::new(None),
            }
        }

        // Write a value to the MVar, possibly overwriting a previous value.
        pub fn write(&self, new: T) {
            let mut val = lock(&self.val);
            *val = Some(new);
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

        // Push an item on the stack.
        pub fn push(&mut self, item: T) {
            self.items.truncate(self.capacity - 1);
            self.items.push_front(item);
        }

        // Pop an item of the stack. This function blocks until an item becomes
        // available.
        pub fn pop(&mut self) -> Option<T> {
            self.items.pop_front()
        }
    }
}

fn does_snapshot_compile(
    file_data: &FileData,
    snapshot: &SourceFileSnapshot,
) -> Result<bool, Error> {
    // Write lates code to temporary file. We don't compile the original source
    // file, because the version stored on disk is likely ahead or behind the
    // version in the editor.
    let mut temp_path = file_data.project_root.join("elm-stuff/elm-pair");
    std::fs::create_dir_all(&temp_path)
        .map_err(Error::CompilationFailedToCreateTempDir)?;
    temp_path.push("Temp.elm");
    std::fs::write(&temp_path, &snapshot.bytes.bytes().collect::<Vec<u8>>())
        .map_err(Error::CompilationFailedToWriteCodeToTempFile)?;

    // Run Elm compiler against temporary file.
    let output = std::process::Command::new(&file_data.elm_bin)
        .arg("make")
        .arg("--report=json")
        .arg(temp_path)
        .current_dir(&file_data.project_root)
        .output()
        .map_err(Error::CompilationFailedToRunElmMake)?;

    Ok(output.status.success())
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
    use ropey::Rope;
    use std::collections::VecDeque;
    use std::io::BufRead;
    use std::path::Path;
    use tree_sitter::InputEdit;

    // TODO: drop dependency on these internals.
    use crate::analysis_thread::{analyze_diff, ElmChange, SourceFileDiff};
    use crate::editor_listener_thread::{
        apply_source_file_edit, init_source_file_snapshot,
    };

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
        let mut latest_code: Option<crate::SourceFileSnapshot> = None;
        let mut last_compiling_code = None;
        let diff_iterator = simulation.msgs.into_iter().filter_map(|msg| {
            let res = {
                match msg {
                    Msg::CompilationSucceeded => {
                        last_compiling_code = latest_code.clone();
                        Ok(())
                    }
                    Msg::ReceivedEditorEvent(event) => {
                        if let Some(code) = &mut latest_code {
                            let edit = change_source(&mut code.bytes, event);
                            apply_source_file_edit(code, edit)
                        } else {
                            init_source_file_snapshot(0, initial_source(event))
                                .map(|code| {
                                    latest_code = Some(code);
                                })
                        }
                    }
                }
            };
            if let Err(err) = res {
                return Some(Err(err));
            }
            match (last_compiling_code.clone(), latest_code.clone()) {
                (Some(old), Some(new)) => Some(Ok(SourceFileDiff { old, new })),
                _ => None,
            }
        });
        diff_iterator
            .map(|res| res.map(|diff| analyze_diff(&diff)))
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
        ReceivedEditorEvent(SimulatedSourceChange),
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
            let mut builder = SimulationBuilder::new(code);
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
        current_bytes: Vec<u8>,
        current_position: usize,
        msgs: VecDeque<Msg>,
    }

    struct SimulatedSourceChange {
        new_bytes: String,
        start_byte: usize,
        old_end_byte: usize,
    }

    fn initial_source(change: SimulatedSourceChange) -> Rope {
        let mut builder = ropey::RopeBuilder::new();
        builder.append(&change.new_bytes);
        builder.finish()
    }

    fn change_source(
        code: &mut Rope,
        change: SimulatedSourceChange,
    ) -> InputEdit {
        let start_char = code.byte_to_char(change.start_byte);
        let old_end_char = code.byte_to_char(change.old_end_byte);
        let old_end_position = crate::byte_to_point(code, change.old_end_byte);
        code.remove(start_char..old_end_char);
        code.insert(start_char, &change.new_bytes);
        let new_end_byte = change.start_byte + change.new_bytes.len();
        InputEdit {
            start_byte: change.start_byte,
            old_end_byte: change.old_end_byte,
            new_end_byte,
            start_position: crate::byte_to_point(code, change.start_byte),
            old_end_position,
            new_end_position: crate::byte_to_point(code, new_end_byte),
        }
    }

    impl SimulationBuilder {
        fn new(initial_bytes: Vec<u8>) -> SimulationBuilder {
            let init_msg = Msg::ReceivedEditorEvent(SimulatedSourceChange {
                new_bytes: std::string::String::from_utf8(
                    initial_bytes.clone(),
                )
                .unwrap(),
                start_byte: 0,
                old_end_byte: 0,
            });
            let mut msgs = VecDeque::new();
            msgs.push_front(init_msg);
            SimulationBuilder {
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
            self.msgs.push_back(Msg::ReceivedEditorEvent(
                SimulatedSourceChange {
                    new_bytes: str.to_owned(),
                    start_byte: range.start,
                    old_end_byte: range.start,
                },
            ));
            self
        }

        fn delete(mut self, str: &str) -> Result<Self, Error> {
            let bytes = str.as_bytes();
            let range =
                self.current_position..(self.current_position + bytes.len());
            if self.current_bytes.get(range.clone()) == Some(bytes) {
                self.current_bytes.splice(range.clone(), []);
                self.msgs.push_back(Msg::ReceivedEditorEvent(
                    SimulatedSourceChange {
                        new_bytes: String::new(),
                        start_byte: range.start,
                        old_end_byte: range.end,
                    },
                ));
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
