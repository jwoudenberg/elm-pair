use crate::analysis_thread;
use crate::elm::compiler::Compiler;
use crate::elm::project_root_for_path;
use crate::sized_stack::SizedStack;
use crate::support::log;
use crate::support::source_code::{Buffer, SourceFileSnapshot};
use crate::{Error, MsgLoop};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};

pub(crate) enum Msg {
    CompilationRequested(SourceFileSnapshot),
    OpenedNewSourceFile { buffer: Buffer, path: PathBuf },
}

pub(crate) fn run(
    compilation_receiver: Receiver<Msg>,
    analysis_sender: Sender<analysis_thread::Msg>,
    compiler: Compiler,
) -> Result<(), Error> {
    CompilationLoop {
        analysis_sender,
        buffer_info: HashMap::new(),
        compilation_candidates: SizedStack::with_capacity(
            crate::MAX_COMPILATION_CANDIDATES,
        ),
        compiler,
    }
    .start(compilation_receiver)
}

struct CompilationLoop {
    analysis_sender: Sender<analysis_thread::Msg>,
    buffer_info: HashMap<Buffer, BufferInfo>,
    compilation_candidates: SizedStack<SourceFileSnapshot>,
    compiler: Compiler,
}

impl MsgLoop<Error> for CompilationLoop {
    type Msg = Msg;

    fn on_msg(&mut self, msg: Msg) -> Result<bool, Error> {
        match msg {
            Msg::CompilationRequested(snapshot) => {
                self.compilation_candidates.push(snapshot)
            }
            Msg::OpenedNewSourceFile { buffer, path } => {
                self.buffer_info.insert(buffer, BufferInfo::new(&path)?);
            }
        }
        Ok(true)
    }

    fn on_idle(&mut self) -> Result<(), Error> {
        let snapshot = match self.compilation_candidates.pop() {
            None => return Ok(()),
            Some(code) => code,
        };
        let buffer_info =
            self.buffer_info.get_mut(&snapshot.buffer).ok_or_else(|| {
                log::mk_err!(
                    "no elm project stored for buffer {:?}",
                    snapshot.buffer
                )
            })?;
        if is_new_revision(&mut buffer_info.last_checked_revision, &snapshot) {
            log::info!(
                "running compilation for revision {:?} of buffer {:?}",
                snapshot.revision,
                snapshot.buffer
            );
            if self
                .compiler
                .make(&buffer_info.root, &snapshot.bytes)?
                .status
                .success()
            {
                self.analysis_sender.send(
                    analysis_thread::Msg::CompilationSucceeded(snapshot),
                )?;
            }
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

struct BufferInfo {
    last_checked_revision: Option<usize>,
    // Root of the Elm project containing this source file.
    root: PathBuf,
}

impl BufferInfo {
    fn new(path: &Path) -> Result<BufferInfo, Error> {
        let info = BufferInfo {
            last_checked_revision: None,
            root: project_root_for_path(path)?.to_owned(),
        };
        Ok(info)
    }
}
