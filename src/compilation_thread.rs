use crate::analysis_thread;
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
    let temp_path = crate::elm_pair_dir()?.join("Temp.elm");
    std::fs::write(&temp_path, &snapshot.bytes.bytes().collect::<Vec<u8>>())
        .map_err(|err| {
            log::mk_err!(
                "error while writing to file {:?}: {:?}",
                temp_path,
                err
            )
        })?;

    // Run Elm compiler against temporary file.
    let output = std::process::Command::new(&buffer_info.elm_bin)
        .arg("make")
        .arg("--report=json")
        .arg(temp_path)
        .current_dir(&buffer_info.root)
        .output()
        .map_err(|err| log::mk_err!("error running `elm make`: {:?}", err))?;

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
            elm_bin: PathBuf::from(
                option_env!("ELM_BINARY_PATH").unwrap_or("elm"),
            ),
        };
        Ok(info)
    }
}
