pub mod added_constructors_to_exposing_list;
pub mod added_exposing_list_to_import;
pub mod added_module_qualifier_to_name;
pub mod changed_as_clause;
pub mod changed_module_qualifier;
pub mod changed_values_in_exposing_list;
pub mod removed_constructors_from_exposing_list;
pub mod removed_exposing_list_from_import;
pub mod removed_module_qualifier_from_name;
pub mod typed_unimported_qualified_value;

#[cfg(test)]
pub mod simulations {
    use crate::analysis_thread::{diff_trees, SourceFileDiff};
    use crate::elm::compiler::Compiler;
    use crate::elm::RefactorEngine;
    use crate::support::log;
    use crate::support::source_code::{Buffer, SourceFileSnapshot};
    use crate::test_support::included_answer_test as ia_test;
    use crate::test_support::simulation::Simulation;
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
                $crate::elm::refactors::simulations::run_simulation_test(&path);
            }
        };
    }
    pub use simulation_test;

    pub fn run_simulation_test(path: &Path) {
        match run_simulation_test_helper(path) {
            Err(Error::ElmPair(err)) => {
                eprintln!("{:?}", err);
                panic!();
            }
            Err(Error::RunningSimulation(err)) => {
                eprintln!("{:?}", err);
                panic!();
            }
            Ok(res) => ia_test::assert_eq_answer_in(&res, path),
        }
    }

    fn run_simulation_test_helper(path: &Path) -> Result<String, Error> {
        let simulation = Simulation::from_file(path)?;
        let buffer = Buffer {
            buffer_id: 0,
            editor_id: 0,
        };
        let old = SourceFileSnapshot::new(buffer, simulation.start_bytes)?;
        let new = SourceFileSnapshot::new(buffer, simulation.end_bytes)?;
        let mut diff = SourceFileDiff { old, new };
        let tree_changes = diff_trees(&diff);
        let compiler = Compiler::new().unwrap();
        let mut refactor_engine = RefactorEngine::new(compiler)?;
        refactor_engine.init_buffer(
            buffer,
            &path.canonicalize().map_err(|err| {
                log::mk_err!("failed to canonicalize path: {:?}", err)
            })?,
        )?;
        let edits = refactor_engine
            .respond_to_change(&diff, tree_changes)?
            .edits(&mut diff.new)?;
        if edits.is_empty() || diff.old.bytes == diff.new.bytes {
            Ok("No refactor for this change.".to_owned())
        } else if diff.new.tree.root_node().has_error() {
            Ok(format!(
                "Refactor produced invalid code:\n{}",
                diff.new.bytes
            ))
        } else {
            Ok(diff.new.bytes.to_string())
        }
    }

    #[derive(Debug)]
    pub enum Error {
        RunningSimulation(crate::test_support::simulation::Error),
        ElmPair(crate::Error),
    }

    impl From<crate::test_support::simulation::Error> for Error {
        fn from(err: crate::test_support::simulation::Error) -> Error {
            Error::RunningSimulation(err)
        }
    }

    impl From<crate::Error> for Error {
        fn from(err: crate::Error) -> Error {
            Error::ElmPair(err)
        }
    }
}
