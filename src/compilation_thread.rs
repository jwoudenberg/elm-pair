use crate::analysis_thread;
use crate::sized_stack::SizedStack;
use crate::{MsgLoop, SourceFileSnapshot};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, SendError, Sender};

pub(crate) enum Msg {
    CompilationRequested(SourceFileSnapshot),
    OpenedNewSourceFile { buffer: usize, path: PathBuf },
}

#[derive(Debug)]
pub(crate) enum Error {
    NoElmJsonFoundInAnyAncestorDirectoryOf(PathBuf),
    DidNotFindElmBinaryOnPath,
    CouldNotReadCurrentWorkingDirectory(std::io::Error),
    DidNotFindPathEnvVar,
    CompilationFailedToCreateTempDir(std::io::Error),
    CompilationFailedToWriteCodeToTempFile(std::io::Error),
    CompilationFailedToRunElmMake(std::io::Error),
    NoElmProjectStoredForBuffer(usize),
    FailedToSendMessage,
}

impl<T> From<SendError<T>> for Error {
    fn from(_err: SendError<T>) -> Error {
        Error::FailedToSendMessage
    }
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
        project: HashMap::new(),
    }
    .start(compilation_receiver)
}

struct CompilationLoop {
    analysis_sender: Sender<analysis_thread::Msg>,
    last_validated_revision: Option<usize>,
    compilation_candidates: SizedStack<SourceFileSnapshot>,
    project: HashMap<usize, ElmProject>,
}

struct ElmProject {
    // Root of the Elm project containing this source file.
    root: PathBuf,
    // Absolute path to the `elm` compiler.
    elm_bin: PathBuf,
}

impl MsgLoop<Error> for CompilationLoop {
    type Msg = Msg;

    fn on_msg(&mut self, msg: Msg) -> Result<bool, Error> {
        match msg {
            Msg::CompilationRequested(snapshot) => {
                self.compilation_candidates.push(snapshot)
            }
            Msg::OpenedNewSourceFile { buffer, path } => {
                self.project.insert(
                    buffer,
                    ElmProject {
                        root: find_project_root(&path)?.to_path_buf(),
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
        let project = self
            .project
            .get(&snapshot.buffer)
            .ok_or(Error::NoElmProjectStoredForBuffer(snapshot.buffer))?;

        if is_new_revision(&mut self.last_validated_revision, &snapshot)
            && does_snapshot_compile(project, &snapshot)?
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

fn does_snapshot_compile(
    project: &ElmProject,
    snapshot: &SourceFileSnapshot,
) -> Result<bool, Error> {
    // Write lates code to temporary file. We don't compile the original source
    // file, because the version stored on disk is likely ahead or behind the
    // version in the editor.
    let mut temp_path = project.root.join("elm-stuff/elm-pair");
    std::fs::create_dir_all(&temp_path)
        .map_err(Error::CompilationFailedToCreateTempDir)?;
    temp_path.push("Temp.elm");
    std::fs::write(&temp_path, &snapshot.bytes.bytes().collect::<Vec<u8>>())
        .map_err(Error::CompilationFailedToWriteCodeToTempFile)?;

    // Run Elm compiler against temporary file.
    let output = std::process::Command::new(&project.elm_bin)
        .arg("make")
        .arg("--report=json")
        .arg(temp_path)
        .current_dir(&project.root)
        .output()
        .map_err(Error::CompilationFailedToRunElmMake)?;

    Ok(output.status.success())
}
