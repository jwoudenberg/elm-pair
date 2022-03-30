// Support for simulation tests of refactor logic.
// Each simulation test is a file containing three parts:
//
// 1. A bit of Elm code.
// 2. A set of edits the simulation test should make to the Elm code above.
// 3. The expected Elm code after Elm-pair refactor logic responds to the edits.

use crate::analysis_thread;
use crate::analysis_thread::{EditorDriver, Msg};
use crate::elm::compiler::Compiler;
use crate::lib::included_answer_test as ia_test;
use crate::lib::simulation::Simulation;
use crate::lib::source_code::{
    update_bytes, Buffer, Edit, EditorId, RefactorAllowed, SourceFileSnapshot,
};
use crate::MsgLoop;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};

#[macro_export]
macro_rules! simulation_test {
    ($name:ident) => {
        #[test]
        fn $name() {
            let mut path = std::path::PathBuf::new();
            path.push("./tests/refactor-simulations");
            let module_name = stringify!($name);
            path.push(module_name.to_owned() + ".elm");
            println!("Run simulation {:?}", &path);
            $crate::elm::refactors::lib::simulations::run_simulation_test(
                &path,
            );
        }
    };
}
pub use simulation_test;

pub fn run_simulation_test(path: &Path) {
    ia_test::for_file(path, |input| {
        match run_simulation_test_helper(path, input) {
            Err(Error::ElmPair(err)) => {
                eprintln!("{:?}", err);
                panic!();
            }
            Err(Error::RunningSimulation(err)) => {
                eprintln!("{:?}", err);
                panic!();
            }
            Ok(res) => res,
        }
    })
}

#[derive(Clone)]
struct MockEditorDriver {
    apply_edits_calls: Arc<Mutex<Vec<Vec<Edit>>>>,
    open_files_calls: Arc<Mutex<Vec<Vec<PathBuf>>>>,
}

impl MockEditorDriver {
    fn new() -> MockEditorDriver {
        MockEditorDriver {
            apply_edits_calls: Arc::new(Mutex::new(Vec::new())),
            open_files_calls: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl EditorDriver for MockEditorDriver {
    fn apply_edits(&self, edits: Vec<Edit>) -> bool {
        let mut apply_edits_calls = self.apply_edits_calls.lock().unwrap();
        apply_edits_calls.push(edits);
        true
    }

    fn open_files(&self, files: Vec<PathBuf>) -> bool {
        let mut open_files_calls = self.open_files_calls.lock().unwrap();
        open_files_calls.push(files);
        true
    }
}

fn run_simulation_test_helper(
    path: &Path,
    input: &str,
) -> Result<String, Error> {
    let (sender, mut receiver) = mpsc::channel();
    let compiler = Compiler::new().unwrap();
    let mut analysis_loop = analysis_thread::create(compiler)?;
    let editor_id = EditorId::new(0);
    let editor_driver = MockEditorDriver::new();
    let buffer = Buffer {
        buffer_id: 0,
        editor_id,
    };

    // Apply simulation commands in source file to get simulated code change.
    let simulation = Simulation::from_str(input)?;
    let old_code = SourceFileSnapshot::new(buffer, simulation.start_bytes)?;
    let mut new_code = SourceFileSnapshot::new(buffer, simulation.end_bytes)?;
    new_code.revision += 2;

    // Queue up messages for analysis thread that simulate code change.
    sender
        .send(Msg::EditorConnected(
            editor_id,
            Box::new(editor_driver.clone()),
        ))
        .unwrap();
    sender
        .send(Msg::OpenedNewSourceFile {
            path: path.to_owned(),
            code: old_code.clone(),
        })
        .unwrap();
    sender
        .send(Msg::CompilationSucceeded(old_code.clone()))
        .unwrap();
    sender
        .send(Msg::SourceCodeModified {
            code: new_code.clone(),
            refactor: RefactorAllowed::Yes,
        })
        .unwrap();

    // Run the analysis loop to process queued messages.
    MsgLoop::step(&mut analysis_loop, &mut receiver)?;

    // Editor-driver should now have received refactors resulting from messages.
    let mut refactored_code = new_code.clone();
    let apply_edits_calls = editor_driver.apply_edits_calls.lock().unwrap();
    for edit in apply_edits_calls.iter().flatten() {
        update_bytes(
            &mut refactored_code.bytes,
            edit.input_edit.start_byte,
            edit.input_edit.old_end_byte,
            &edit.new_bytes,
        );
        refactored_code.apply_edit(edit.input_edit)?;
    }

    // Return post-refactor code, for comparison against expected value.
    if apply_edits_calls.is_empty()
        || old_code.bytes == refactored_code.bytes
        || new_code.bytes == refactored_code.bytes
    {
        Ok("No refactor for this change.".to_owned())
    } else {
        Ok(refactored_code.bytes.to_string())
    }
}

#[derive(Debug)]
pub enum Error {
    RunningSimulation(crate::lib::simulation::Error),
    ElmPair(crate::Error),
}

impl From<crate::lib::simulation::Error> for Error {
    fn from(err: crate::lib::simulation::Error) -> Error {
        Error::RunningSimulation(err)
    }
}

impl From<crate::Error> for Error {
    fn from(err: crate::Error) -> Error {
        Error::ElmPair(err)
    }
}
