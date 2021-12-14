use crate::analysis_thread;
use crate::languages::elm::knowledge_base::{CompilationParams, KnowledgeBase};
use crate::sized_stack::SizedStack;
use crate::support::source_code::{Buffer, SourceFileSnapshot};
use crate::{Error, MsgLoop};
use std::collections::HashMap;
use std::path::PathBuf;
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
        last_checked_revisions: HashMap::new(),
        compilation_candidates: SizedStack::with_capacity(
            crate::MAX_COMPILATION_CANDIDATES,
        ),
        kb: KnowledgeBase::new(),
    }
    .start(compilation_receiver)
}

struct CompilationLoop {
    analysis_sender: Sender<analysis_thread::Msg>,
    last_checked_revisions: HashMap<Buffer, usize>,
    compilation_candidates: SizedStack<SourceFileSnapshot>,
    kb: KnowledgeBase,
}

impl MsgLoop<Error> for CompilationLoop {
    type Msg = Msg;

    fn on_msg(&mut self, msg: Msg) -> Result<bool, Error> {
        match msg {
            Msg::CompilationRequested(snapshot) => {
                self.compilation_candidates.push(snapshot)
            }
            Msg::OpenedNewSourceFile { buffer, path } => {
                self.kb.insert_buffer_path(buffer, path);
            }
        }
        Ok(true)
    }

    fn on_idle(&mut self) -> Result<(), Error> {
        let snapshot = match self.compilation_candidates.pop() {
            None => return Ok(()),
            Some(code) => code,
        };
        if is_new_revision(&mut self.last_checked_revisions, &snapshot) {
            eprintln!(
                "[info] running compilation for revision {:?} of buffer {:?}",
                snapshot.revision, snapshot.buffer
            );
            let compilation_params =
                self.kb.get_compilation_params(snapshot.buffer)?;
            if does_snapshot_compile(compilation_params, &snapshot)? {
                self.analysis_sender.send(
                    analysis_thread::Msg::CompilationSucceeded(snapshot),
                )?;
            }
        }
        Ok(())
    }
}

fn is_new_revision(
    last_checked_revisions: &mut HashMap<Buffer, usize>,
    code: &SourceFileSnapshot,
) -> bool {
    let is_new = match last_checked_revisions.get(&code.buffer) {
        None => true,
        Some(old) => code.revision > *old,
    };
    if is_new {
        last_checked_revisions.insert(code.buffer, code.revision);
    }
    is_new
}

fn does_snapshot_compile(
    project: &CompilationParams,
    snapshot: &SourceFileSnapshot,
) -> Result<bool, Error> {
    // Write lates code to temporary file. We don't compile the original source
    // file, because the version stored on disk is likely ahead or behind the
    // version in the editor.
    let mut temp_path = project.root.join("elm-stuff/elm-pair");
    std::fs::create_dir_all(&temp_path)
        .map_err(Error::ElmCompilationFailedToCreateTempDir)?;
    temp_path.push("Temp.elm");
    std::fs::write(&temp_path, &snapshot.bytes.bytes().collect::<Vec<u8>>())
        .map_err(Error::ElmCompilationFailedToWriteCodeToTempFile)?;

    // Run Elm compiler against temporary file.
    let output = std::process::Command::new(&project.elm_bin)
        .arg("make")
        .arg("--report=json")
        .arg(temp_path)
        .current_dir(&project.root)
        .output()
        .map_err(Error::ElmCompilationFailedToRunElmMake)?;

    Ok(output.status.success())
}
