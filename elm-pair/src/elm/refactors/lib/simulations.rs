// Support for simulation tests of refactor logic.
// Each simulation test is a file containing three parts:
//
// 1. A bit of Elm code.
// 2. A set of edits the simulation test should make to the Elm code above.
// 3. The expected Elm code after Elm-pair refactor logic responds to the edits.
//
// Alternatively the simulation test can be a directory containing multiple Elm
// files containing a separate SIMULATION file (point 2), and multiple Elm files
// containing original and refactored Elm code (points 1 and 3).

use crate::analysis_thread;
use crate::analysis_thread::Msg;
use crate::editors;
use crate::elm::compiler::Compiler;
use crate::lib::included_answer_test as ia_test;
use crate::lib::log;
use crate::lib::simulation;
use crate::lib::source_code::{update_bytes, Buffer, Edit, SourceFileSnapshot};
use crate::MsgLoop;
use std::collections::HashMap;
use std::iter::FromIterator;
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
            path.push(module_name.to_owned());
            if !path.exists() {
                path.set_extension("elm");
            }
            println!("Run simulation {:?}", &path);
            $crate::elm::refactors::lib::simulations::run_simulation_test(
                &path,
            );
        }
    };
}
pub use simulation_test;

pub fn run_simulation_test(path: &Path) {
    ia_test::for_path(path, |inputs| match run_simulation_test_helper(inputs) {
        Err(Error::ElmPair(err)) => {
            eprintln!("{:?}", err);
            panic!();
        }
        Err(Error::RunningSimulation(err)) => {
            eprintln!("{:?}", err);
            panic!();
        }
        Ok(()) => {}
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

impl editors::Driver for MockEditorDriver {
    fn kind(&self) -> editors::Kind {
        editors::Kind::Neovim
    }

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

    fn show_file(&self, _path: &Path) -> bool {
        true
    }
}

fn run_simulation_test_helper(
    inputs: &mut HashMap<PathBuf, String>,
) -> Result<(), Error> {
    let (sender, mut receiver) = mpsc::channel();
    let compiler = Compiler::new().unwrap();
    let mut analysis_loop = analysis_thread::create(compiler)?;
    let editor_id = editors::Id::new(0);
    let editor_driver = MockEditorDriver::new();

    let mut opt_simulation = None;

    // Apply simulation commands in source file to get simulated code change.
    let old_code_by_path = HashMap::from_iter(inputs.iter().enumerate().map(
        |(buffer_id, (path, str))| {
            let (bytes, opt_simulation_) =
                simulation::create(path.clone(), str).unwrap();
            opt_simulation = opt_simulation.take().or(opt_simulation_);
            (
                path.clone(),
                SourceFileSnapshot::new(
                    Buffer {
                        buffer_id: buffer_id as u32,
                        editor_id,
                    },
                    bytes,
                )
                .unwrap(),
            )
        },
    ));

    // Queue up messages for analysis thread that simulate code change.
    sender
        .send(Msg::EditorConnected(
            editor_id,
            Box::new(editor_driver.clone()),
        ))
        .unwrap();
    for (path, code) in &old_code_by_path {
        sender
            .send(Msg::OpenedNewSourceFile {
                path: path.to_owned(),
                code: code.clone(),
            })
            .unwrap();
        sender
            .send(Msg::CompilationSucceeded(code.clone()))
            .unwrap();
    }

    let new_code_by_path = simulation::run(
        opt_simulation.ok_or_else(|| {
            log::mk_err!("Did not find test file containing simulation.")
        })?,
        old_code_by_path.clone(),
        sender.clone(),
    )?;

    // Run the analysis loop to process queued messages.
    MsgLoop::step(&mut analysis_loop, &mut receiver)?;

    // Now that the diffing/refactoring logic has ran, we can drop the sender.
    // We're explicitly dropping the sender here to ensure it stays alive up to
    // this point. If we'd dropped it earlier, after sending the last message
    // for example, then the analysis thread will receive a disconnect message
    // and not run any diffing logic.
    drop(sender);

    // Editor-driver should now have received refactors resulting from messages.
    let mut refactored_code_by_path = new_code_by_path;
    let apply_edits_calls = editor_driver.apply_edits_calls.lock().unwrap();
    let path_by_buffer_id: HashMap<Buffer, PathBuf> = HashMap::from_iter(
        old_code_by_path
            .iter()
            .map(|(path, code)| (code.buffer, path.clone())),
    );
    for edit in apply_edits_calls.iter().flatten() {
        let path = path_by_buffer_id.get(&edit.buffer).unwrap();
        let refactored_code = refactored_code_by_path.get_mut(path).unwrap();
        update_bytes(
            &mut refactored_code.bytes,
            edit.input_edit.start_byte,
            edit.input_edit.old_end_byte,
            &edit.new_bytes,
        );
        refactored_code.apply_edit(edit.input_edit)?;
    }

    // Return post-refactor code, for comparison against expected value.
    for (path, input) in inputs {
        let old_code = old_code_by_path.get(path).unwrap();
        let refactored_code = refactored_code_by_path.get(path).unwrap();
        *input = if apply_edits_calls.is_empty()
            || old_code.bytes == refactored_code.bytes
        {
            "No refactor for this change.".to_owned()
        } else {
            refactored_code.bytes.to_string()
        };
    }

    Ok(())
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
