use crate::analysis_thread;
use crate::sized_stack::SizedStack;
use crate::{Error, FileData, MsgLoop, SourceFileSnapshot};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};

pub(crate) enum Msg {
    CompilationRequested(SourceFileSnapshot),
    OpenedNewSourceFile { buffer: usize, path: PathBuf },
}

pub(crate) fn run(
    compilation_receiver: Receiver<Msg>,
    analysis_sender: Sender<analysis_thread::Msg>,
) -> Result<(), Error> {
    CompilationLoop {
        analysis_sender,
        last_validated_revision: None,
        compilation_candidates: SizedStack::with_capacity(
            crate::MAX_COMPILATION_CANDIDATES,
        ),
        file_data: HashMap::new(),
    }
    .start(compilation_receiver)
}

struct CompilationLoop {
    analysis_sender: Sender<analysis_thread::Msg>,
    last_validated_revision: Option<usize>,
    compilation_candidates: SizedStack<SourceFileSnapshot>,
    file_data: HashMap<usize, FileData>,
}

impl MsgLoop<Error> for CompilationLoop {
    type Msg = Msg;

    fn on_msg(&mut self, msg: Msg) -> Result<bool, Error> {
        match msg {
            Msg::CompilationRequested(snapshot) => {
                self.compilation_candidates.push(snapshot)
            }
            Msg::OpenedNewSourceFile { buffer, path } => {
                self.file_data.insert(
                    buffer,
                    FileData {
                        project_root: crate::find_project_root(&path)?
                            .to_path_buf(),
                        elm_bin: crate::find_executable("elm")?,
                    },
                );
            }
        }
        Ok(true)
    }

    fn on_idle(&mut self) -> Result<(), Error> {
        let snapshot = match self.compilation_candidates.pop() {
            None => return Ok(()),
            Some(code) => code,
        };
        let file_data = self
            .file_data
            .get(&snapshot.buffer)
            .ok_or(Error::NoFileDataStoredForBuffer(snapshot.buffer))?;

        if is_new_revision(&mut self.last_validated_revision, &snapshot)
            && crate::does_snapshot_compile(file_data, &snapshot)?
        {
            self.analysis_sender
                .send(analysis_thread::Msg::CompilationSucceeded(snapshot))?;
        }
        Ok(())
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
