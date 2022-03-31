// A module to support tests of the diffing logic by running simulations against
// it.

use crate::analysis_thread::Msg;
use crate::lib::intersperse::Intersperse;
use crate::lib::source_code::{
    Buffer, Edit, EditorId, RefactorAllowed, SourceFileSnapshot,
};
use core::ops::Range;
use ropey::Rope;
use std::collections::HashMap;
use std::iter::FromIterator;
use std::path::PathBuf;
use std::sync::mpsc::Sender;

pub type Simulation = Vec<Step>;

pub enum Step {
    OpenFile(PathBuf),
    MoveCursor { line: usize, word: String },
    Insert(String),
    Delete(String),
}

pub fn create(
    path: PathBuf,
    source: &str,
) -> Result<(Rope, Option<Simulation>), Error> {
    let mut lines = source.lines();
    let mut opt_simulation_script_padding = None;

    let code = Rope::from_iter(
        lines
            .by_ref()
            .take_while(|line| {
                if let Some(padding) = line.strip_suffix("START SIMULATION") {
                    opt_simulation_script_padding = Some(padding.len());
                    false
                } else {
                    true
                }
            })
            .my_intersperse("\n"),
    );

    if let Some(simulation_script_padding) = opt_simulation_script_padding {
        let mut simulation = vec![Step::OpenFile(path)];
        loop {
            let line = match lines.next() {
                None => return Err(Error::FileEndCameBeforeSimulationEnd),
                Some(line) => line
                    .get(simulation_script_padding..)
                    .ok_or(
                        Error::SimulationInstructionsDontHaveConsistentPadding,
                    )?
                    .to_string(),
            };
            match line.split(' ').collect::<Vec<&str>>().as_slice() {
                ["END", "SIMULATION"] => break,
                ["OPEN", "FILE", file_path] => {
                    simulation.push(Step::OpenFile(PathBuf::from(file_path)));
                }
                ["MOVE", "CURSOR", "TO", "LINE", line_str, strs @ ..] => {
                    let line = line_str.parse().map_err(|_| {
                        Error::CannotParseLineNumber(line.to_string())
                    })?;
                    let word = strs.join(" ");
                    simulation.push(Step::MoveCursor { line, word });
                }
                ["INSERT", strs @ ..] => {
                    simulation.push(Step::Insert(strs.join(" ")))
                }
                ["DELETE", strs @ ..] => {
                    simulation.push(Step::Delete(strs.join(" ")))
                }
                _ => return Err(Error::CannotParseSimulationLine(line)),
            };
        }
        Ok((code, Some(simulation)))
    } else {
        Ok((code, None))
    }
}

pub fn run(
    simulation: Simulation,
    files: HashMap<PathBuf, SourceFileSnapshot>,
    sender: Sender<Msg>,
) -> Result<HashMap<PathBuf, SourceFileSnapshot>, Error> {
    let mut runner = SimulationRunner::new(files, sender);
    for step in simulation {
        match step {
            Step::OpenFile(path) => {
                runner.open_file(path)?;
            }
            Step::MoveCursor { line, word } => {
                runner.move_cursor(line, &word)?;
            }
            Step::Insert(str) => {
                runner.insert(&str);
            }
            Step::Delete(str) => {
                runner.delete(&str)?;
            }
        }
    }
    let changed_files = runner.finish();
    Ok(changed_files)
}

struct SimulationRunner {
    sender: Sender<Msg>,
    open_file: Option<(PathBuf, SimulationFileState)>,
    other_files: HashMap<PathBuf, SimulationFileState>,
}

struct SimulationFileState {
    current_code: SourceFileSnapshot,
    current_position: usize,
}

impl SimulationRunner {
    fn new(
        files: HashMap<PathBuf, SourceFileSnapshot>,
        sender: Sender<Msg>,
    ) -> SimulationRunner {
        SimulationRunner {
            sender,
            open_file: None,
            other_files: HashMap::from_iter(files.into_iter().map(
                |(path, current_code)| {
                    (
                        path,
                        SimulationFileState {
                            current_position: 0,
                            current_code,
                        },
                    )
                },
            )),
        }
    }

    fn add_edit(&mut self, range: &Range<usize>, new_bytes: String) {
        let edit = Edit::new(
            Buffer {
                editor_id: EditorId::new(0),
                buffer_id: 0,
            },
            &mut self.open_file.as_mut().unwrap().1.current_code.bytes,
            range,
            new_bytes,
        );
        let state = self.current_state();
        state.current_code.apply_edit(edit.input_edit).unwrap();
        let new_code = state.current_code.clone();
        self.sender
            .send(Msg::SourceCodeModified {
                code: new_code,
                refactor: RefactorAllowed::Yes,
            })
            .unwrap();
    }

    fn current_state(&mut self) -> &mut SimulationFileState {
        &mut self.open_file.as_mut().unwrap().1
    }

    fn open_file(&mut self, path_fragment: PathBuf) -> Result<(), Error> {
        if let Some((prev_path, state)) = self.open_file.take() {
            self.other_files.insert(prev_path, state);
        }
        let mut opt_full_path: Option<PathBuf> = self
            .other_files
            .keys()
            .find(|path| path.ends_with(&path_fragment))
            .cloned();
        if let Some(full_path) = opt_full_path.take() {
            let new_open_file = self.other_files.remove(&full_path).unwrap();
            self.open_file = Some((full_path, new_open_file));
            Ok(())
        } else {
            Err(Error::CannotFindFile(path_fragment))
        }
    }

    fn move_cursor(&mut self, line: usize, word: &str) -> Result<(), Error> {
        if line == 0 {
            return Err(Error::MoveCursorFailedLineZeroNotAllowed);
        }
        let state = self.current_state();
        state.current_position =
            state.current_code.bytes.try_line_to_char(line - 1)?;
        let line_end = state.current_code.bytes.try_line_to_char(line)?;
        let word_rope = Rope::from_str(word);
        while state.current_position < line_end {
            let prefix = state
                .current_code
                .bytes
                .get_slice(
                    state.current_position
                        ..(state.current_position + word_rope.len_chars()),
                )
                .ok_or(Error::GettingRopeSlice)?;
            if prefix == word_rope {
                return Ok(());
            };
            state.current_position += 1;
        }
        Err(Error::MoveCursorDidNotFindWordOnLine)
    }

    fn insert(&mut self, str: &str) {
        let state = self.current_state();
        let range = state.current_position..state.current_position;
        self.add_edit(&range, str.to_owned());
    }

    fn delete(&mut self, to_delete: &str) -> Result<(), Error> {
        let state = self.current_state();
        let str_rope = Rope::from_str(to_delete);
        let range = state.current_position
            ..(state.current_position + str_rope.len_chars());
        let at_cursor = state.current_code.bytes.slice(range.clone());
        if at_cursor != to_delete {
            return Err(Error::TextToDeleteDoesNotMatchStringAtCursor {
                _to_delete: to_delete.to_owned(),
                _at_cursor: at_cursor.to_string(),
            });
        }
        self.add_edit(&range, String::new());
        Ok(())
    }

    fn finish(mut self) -> HashMap<PathBuf, SourceFileSnapshot> {
        if let Some((path, code)) = self.open_file.take() {
            self.other_files.insert(path, code);
        }
        HashMap::from_iter(
            self.other_files
                .into_iter()
                .map(|(path, state)| (path, state.current_code)),
        )
    }
}

#[derive(Debug)]
pub enum Error {
    CannotFindFile(PathBuf),
    CannotParseSimulationLine(String),
    CannotParseLineNumber(String),
    FileEndCameBeforeSimulationEnd,
    SimulationInstructionsDontHaveConsistentPadding,
    MoveCursorFailedLineZeroNotAllowed,
    MoveCursorDidNotFindWordOnLine,
    FailureWhileNavigatingSimulationRope(ropey::Error),
    GettingRopeSlice,
    TextToDeleteDoesNotMatchStringAtCursor {
        _to_delete: String,
        _at_cursor: String,
    },
}

impl From<ropey::Error> for Error {
    fn from(err: ropey::Error) -> Error {
        Error::FailureWhileNavigatingSimulationRope(err)
    }
}
