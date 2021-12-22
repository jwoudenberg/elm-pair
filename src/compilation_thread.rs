use crate::analysis_thread;
use crate::languages::elm::project_root_for_path;
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
) -> Result<(), Error> {
    CompilationLoop {
        analysis_sender,
        buffer_info: HashMap::new(),
        compilation_candidates: SizedStack::with_capacity(
            crate::MAX_COMPILATION_CANDIDATES,
        ),
    }
    .start(compilation_receiver)
}

struct CompilationLoop {
    analysis_sender: Sender<analysis_thread::Msg>,
    buffer_info: HashMap<Buffer, BufferInfo>,
    compilation_candidates: SizedStack<SourceFileSnapshot>,
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
        let buffer_info = self
            .buffer_info
            .get_mut(&snapshot.buffer)
            .ok_or(Error::ElmNoProjectStoredForBuffer(snapshot.buffer))?;
        if is_new_revision(&mut buffer_info.last_checked_revision, &snapshot) {
            log::info!(
                "running compilation for revision {:?} of buffer {:?}",
                snapshot.revision,
                snapshot.buffer
            );
            if does_snapshot_compile(buffer_info, &snapshot)? {
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

fn does_snapshot_compile(
    buffer_info: &BufferInfo,
    snapshot: &SourceFileSnapshot,
) -> Result<bool, Error> {
    // Write lates code to temporary file. We don't compile the original source
    // file, because the version stored on disk is likely ahead or behind the
    // version in the editor.
    let mut temp_path = buffer_info.root.join("elm-stuff/elm-pair");
    std::fs::create_dir_all(&temp_path)
        .map_err(Error::ElmCompilationFailedToCreateTempDir)?;
    temp_path.push("Temp.elm");
    std::fs::write(&temp_path, &snapshot.bytes.bytes().collect::<Vec<u8>>())
        .map_err(Error::ElmCompilationFailedToWriteCodeToTempFile)?;

    // Run Elm compiler against temporary file.
    let output = std::process::Command::new(&buffer_info.elm_bin)
        .arg("make")
        .arg("--report=json")
        .arg(temp_path)
        .current_dir(&buffer_info.root)
        .output()
        .map_err(Error::ElmCompilationFailedToRunElmMake)?;

    Ok(output.status.success())
}

struct BufferInfo {
    last_checked_revision: Option<usize>,
    // Root of the Elm project containing this source file.
    root: PathBuf,
    // Absolute path to the `elm` compiler.
    elm_bin: PathBuf,
}

impl BufferInfo {
    fn new(path: &Path) -> Result<BufferInfo, Error> {
        let info = BufferInfo {
            last_checked_revision: None,
            root: project_root_for_path(path)?.to_owned(),
            elm_bin: find_executable("elm")?,
        };
        Ok(info)
    }
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
    Err(Error::ElmDidNotFindCompilerBinaryOnPath)
}
