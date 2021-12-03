use crate::analysis_thread;
use crate::sized_stack::SizedStack;
use crate::{Error, FileData, MsgLoop, SourceFileSnapshot};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
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
                        project_root: find_project_root(&path)?.to_path_buf(),
                        elm_bin: find_executable("elm")?,
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

fn find_executable(name: &str) -> Result<PathBuf, Error> {
    let cwd = std::env::current_dir()
        .map_err(Error::CouldNotReadCurrentWorkingDirectory)?;
    let path = std::env::var_os("PATH").ok_or(Error::DidNotFindPathEnvVar)?;
    let dirs = std::env::split_paths(&path);
    for dir in dirs {
        let mut bin_path = cwd.join(dir);
        bin_path.push(name);
        if bin_path.is_file() {
            return Ok(bin_path);
        };
    }
    Err(Error::DidNotFindElmBinaryOnPath)
}

fn find_project_root(source_file: &Path) -> Result<&Path, Error> {
    let mut maybe_root = source_file;
    loop {
        match maybe_root.parent() {
            None => {
                return Err(Error::NoElmJsonFoundInAnyAncestorDirectoryOf(
                    source_file.to_path_buf(),
                ));
            }
            Some(parent) => {
                if parent.join("elm.json").exists() {
                    return Ok(parent);
                } else {
                    maybe_root = parent;
                }
            }
        }
    }
}
