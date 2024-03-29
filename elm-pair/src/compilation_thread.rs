use crate::analysis_thread;
use crate::elm::compiler::Compiler;
use crate::lib::log;
use crate::lib::source_code::{Buffer, SourceFileSnapshot};
use crate::sized_stack::SizedStack;
use crate::{Error, MsgLoop};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

pub enum Msg {
    CompilationRequested(SourceFileSnapshot),
    OpenedNewSourceFile { buffer: Buffer, path: PathBuf },
}

pub fn create(
    analysis_sender: Sender<analysis_thread::Msg>,
    compiler: Compiler,
) -> Result<CompilationLoop, Error> {
    let compilation_loop = CompilationLoop {
        analysis_sender,
        buffer_info: HashMap::new(),
        compilation_candidates: SizedStack::with_capacity(
            crate::MAX_COMPILATION_CANDIDATES,
        ),
        compiler,
    };
    Ok(compilation_loop)
}

pub struct CompilationLoop {
    analysis_sender: Sender<analysis_thread::Msg>,
    buffer_info: HashMap<Buffer, BufferInfo>,
    compilation_candidates: SizedStack<SourceFileSnapshot>,
    compiler: Compiler,
}

impl MsgLoop for CompilationLoop {
    type Msg = Msg;
    type Err = Error;

    fn on_msg(&mut self, msg: Msg) -> Result<bool, Error> {
        match msg {
            Msg::CompilationRequested(snapshot) => {
                self.compilation_candidates.push(snapshot)
            }
            Msg::OpenedNewSourceFile { buffer, path } => {
                self.buffer_info.insert(buffer, BufferInfo::new(&path));
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
        let root = match &buffer_info.root {
            ElmProjectRoot::Known(root_path) => root_path,
            ElmProjectRoot::Unknown => {
                // We can't compile if we don't know the root of the project
                // this elm module is located in. We already logged an error
                // when we created the ElmProjectRoot::Unknown constructor, so
                // we're not going to log the same error again here.
                return Ok(());
            }
        };
        if is_new_revision(&mut buffer_info.last_checked_revision, &snapshot) {
            log::info!(
                "running compilation for revision {:?} of buffer {:?}",
                snapshot.revision,
                snapshot.buffer
            );
            let opt_output = self.compiler.make(root, &snapshot.bytes);
            match opt_output.map(|output| output.status.success()) {
                Err(err) => {
                    log::error!("Failure running `elm make`: {:?}", err)
                }
                Ok(false) => {}
                Ok(true) => {
                    self.analysis_sender.send(
                        analysis_thread::Msg::CompilationSucceeded(snapshot),
                    )?;
                }
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

enum ElmProjectRoot {
    Known(PathBuf),
    Unknown,
}

struct BufferInfo {
    last_checked_revision: Option<usize>,
    // Root of the Elm project containing this source file.
    root: ElmProjectRoot,
}

impl BufferInfo {
    fn new(path: &Path) -> BufferInfo {
        let root = match crate::elm::project::root(path) {
            Ok(root_path) => ElmProjectRoot::Known(root_path.to_owned()),
            Err(err) => {
                log::info!(
                    "Could not find elm project root for path {:?}: {:?}",
                    path,
                    err
                );
                ElmProjectRoot::Unknown
            }
        };
        BufferInfo {
            last_checked_revision: None,
            root,
        }
    }
}
