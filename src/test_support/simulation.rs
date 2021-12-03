// A module to support tests of the diffing logic by running simulations against
// it.

use crate::test_support::included_answer_test::assert_eq_answer_in;
use ropey::Rope;
use std::collections::VecDeque;
use std::io::BufRead;
use std::path::Path;
use tree_sitter::InputEdit;

// TODO: drop dependency on these internals.
use crate::analysis_thread::elm::ElmChange;
use crate::analysis_thread::{analyze_diff, SourceFileDiff};
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
            let module_name = crate::test_support::simulation::snake_to_camel(
                stringify!($name),
            );
            path.push(module_name + ".elm");
            println!("Run simulation {:?}", &path);
            crate::test_support::simulation::run_simulation_test(&path);
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

fn run_simulation_test_helper(path: &Path) -> Result<Option<ElmChange>, Error> {
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
                        init_source_file_snapshot(0, initial_source(event)).map(
                            |code| {
                                latest_code = Some(code);
                            },
                        )
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
                    .ok_or(
                        Error::SimulationInstructionsDontHaveConsistentPadding,
                    )?
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

fn change_source(code: &mut Rope, change: SimulatedSourceChange) -> InputEdit {
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
            new_bytes: std::string::String::from_utf8(initial_bytes.clone())
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
        self.msgs
            .push_back(Msg::ReceivedEditorEvent(SimulatedSourceChange {
                new_bytes: str.to_owned(),
                start_byte: range.start,
                old_end_byte: range.start,
            }));
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
    RunningSimulationFailed(crate::editor_listener_thread::Error),
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
