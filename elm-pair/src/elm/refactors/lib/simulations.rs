// Support for simulation tests of refactor logic.
// Each simulation test is a file containing three parts:
//
// 1. A bit of Elm code.
// 2. A set of edits the simulation test should make to the Elm code above.
// 3. The expected Elm code after Elm-pair refactor logic responds to the edits.

use crate::analysis_thread::{diff_trees, SourceFileDiff};
use crate::elm::compiler::Compiler;
use crate::elm::RefactorEngine;
use crate::lib::included_answer_test as ia_test;
use crate::lib::log;
use crate::lib::simulation::Simulation;
use crate::lib::source_code::{Buffer, EditorId, SourceFileSnapshot};
use std::collections::HashMap;
use std::path::Path;

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
            $crate::elm::refactors::lib::simulations::run_simulation_test(&path);
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

fn run_simulation_test_helper(path: &Path, input: &str) -> Result<String, Error> {
    let simulation = Simulation::from_str(input)?;
    let buffer = Buffer {
        buffer_id: 0,
        editor_id: EditorId::new(0),
    };
    let old = SourceFileSnapshot::new(buffer, simulation.start_bytes)?;
    let new = SourceFileSnapshot::new(buffer, simulation.end_bytes)?;
    let diff = SourceFileDiff {
        old,
        new: new.clone(),
    };
    let tree_changes = diff_trees(&diff);
    let compiler = Compiler::new().unwrap();
    let mut refactor_engine = RefactorEngine::new(compiler)?;
    refactor_engine.init_buffer(
        buffer,
        &path
            .canonicalize()
            .map_err(|err| log::mk_err!("failed to canonicalize path: {:?}", err))?,
    )?;
    let mut buffers = HashMap::new();
    buffers.insert(buffer, new);
    let (edits, _) = refactor_engine
        .respond_to_change(&diff, tree_changes, &HashMap::new(), &HashMap::new())?
        .edits(&mut buffers)?;
    let new_code = buffers.remove(&buffer).unwrap();
    if edits.is_empty() || diff.old.bytes == new_code.bytes {
        Ok("No refactor for this change.".to_owned())
    } else if new_code.tree.root_node().has_error() {
        Ok(format!(
            "Refactor produced invalid code:\n{}",
            new_code.bytes
        ))
    } else {
        Ok(new_code.bytes.to_string())
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
