// A module to support tests of the diffing logic by running simulations against
// it.

use crate::lib::source_code::{Buffer, Edit};
use core::ops::Range;
use ropey::Rope;
use std::io::BufRead;
use std::path::Path;

fn find_start_simulation_script<I>(
    lines: &mut I,
) -> Result<(String, usize), Error>
where
    I: Iterator<Item = Result<String, Error>>,
{
    let mut code: Vec<String> = Vec::new();
    loop {
        let line = match lines.next() {
            None => return Err(Error::FromFileFailedNoStartSimulationFound),
            Some(Err(err)) => return Err(err),
            Some(Ok(line)) => line,
        };
        if let Some(padding) = line.strip_suffix("START SIMULATION") {
            break Ok((code.join("\n"), padding.len()));
        } else {
            code.push(line);
        }
    }
}

pub struct Simulation {
    pub start_bytes: Rope,
    pub end_bytes: Rope,
    pub _edits: Vec<Edit>,
}

impl Simulation {
    pub fn from_file(path: &Path) -> Result<Simulation, Error> {
        let file =
            std::fs::File::open(path).map_err(Error::FromFileOpenFailed)?;
        let mut lines = std::io::BufReader::new(file)
            .lines()
            .map(|line| line.map_err(Error::FromFileReadingLineFailed));
        let (code, simulation_script_padding) =
            find_start_simulation_script(&mut lines)?;
        let mut builder = SimulationBuilder::new(Rope::from_str(&code));
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
                _ => return Err(Error::CannotParseSimulationLine(line)),
            };
        }
        Ok(builder.finish())
    }
}

struct SimulationBuilder {
    start_bytes: Rope,
    current_bytes: Rope,
    current_position: usize,
    edits: Vec<Edit>,
}

impl SimulationBuilder {
    fn new(start_bytes: Rope) -> SimulationBuilder {
        SimulationBuilder {
            current_position: 0,
            current_bytes: start_bytes.clone(),
            start_bytes,
            edits: Vec::new(),
        }
    }

    fn add_edit(&mut self, range: &Range<usize>, new_bytes: String) {
        let edit = Edit::new(
            Buffer {
                editor_id: 0,
                buffer_id: 0,
            },
            &mut self.current_bytes,
            range,
            new_bytes,
        );
        self.edits.push(edit);
    }

    fn move_cursor(mut self, line: usize, word: &str) -> Result<Self, Error> {
        if line == 0 {
            return Err(Error::MoveCursorFailedLineZeroNotAllowed);
        }
        self.current_position =
            self.current_bytes.try_line_to_char(line - 1)?;
        let line_end = self.current_bytes.try_line_to_char(line)?;
        let word_rope = Rope::from_str(word);
        while self.current_position < line_end {
            let prefix = self
                .current_bytes
                .get_slice(
                    self.current_position
                        ..(self.current_position + word_rope.len_chars()),
                )
                .ok_or(Error::GettingRopeSlice)?;
            if prefix == word_rope {
                return Ok(self);
            };
            self.current_position += 1;
        }
        Err(Error::MoveCursorDidNotFindWordOnLine)
    }

    fn insert(mut self, str: &str) -> Self {
        let range = self.current_position..self.current_position;
        self.add_edit(&range, str.to_owned());
        self
    }

    fn delete(mut self, to_delete: &str) -> Result<Self, Error> {
        let str_rope = Rope::from_str(to_delete);
        let range = self.current_position
            ..(self.current_position + str_rope.len_chars());
        let at_cursor = self.current_bytes.slice(range.clone());
        if at_cursor != to_delete {
            return Err(Error::TextToDeleteDoesNotMatchStringAtCursor {
                _to_delete: to_delete.to_owned(),
                _at_cursor: at_cursor.to_string(),
            });
        }
        self.add_edit(&range, String::new());
        Ok(self)
    }

    fn finish(self) -> Simulation {
        Simulation {
            start_bytes: self.start_bytes,
            end_bytes: self.current_bytes,
            _edits: self.edits,
        }
    }
}

#[derive(Debug)]
pub enum Error {
    FromFileFailedNoStartSimulationFound,
    CannotParseSimulationLine(String),
    CannotParseLineNumber(String),
    FileEndCameBeforeSimulationEnd,
    SimulationInstructionsDontHaveConsistentPadding,
    FromFileOpenFailed(std::io::Error),
    FromFileReadingLineFailed(std::io::Error),
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
