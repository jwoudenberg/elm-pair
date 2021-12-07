use crate::analysis_thread;
use crate::sized_stack::SizedStack;
use crate::{Buffer, MsgLoop, SourceFileSnapshot};
use knowledge_base::Query;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, SendError, Sender};

pub(crate) enum Msg {
    CompilationRequested(SourceFileSnapshot),
    OpenedNewSourceFile { buffer: Buffer, path: PathBuf },
}

#[derive(Debug)]
pub(crate) enum Error {
    _NoElmJsonFoundInAnyAncestorDirectoryOf(PathBuf),
    DidNotFindElmBinaryOnPath,
    CouldNotReadCurrentWorkingDirectory(std::io::Error),
    DidNotFindPathEnvVar,
    CompilationFailedToCreateTempDir(std::io::Error),
    CompilationFailedToWriteCodeToTempFile(std::io::Error),
    CompilationFailedToRunElmMake(std::io::Error),
    NoElmProjectStoredForBuffer(Buffer),
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
        last_checked_revisions: HashMap::new(),
        compilation_candidates: SizedStack::with_capacity(
            crate::MAX_COMPILATION_CANDIDATES,
        ),
        project: HashMap::new(),
        knowledge_base: KnowledgeBase {
            project_root: HashMap::new(),
        },
    }
    .start(compilation_receiver)
}

struct CompilationLoop {
    analysis_sender: Sender<analysis_thread::Msg>,
    last_checked_revisions: HashMap<Buffer, usize>,
    compilation_candidates: SizedStack<SourceFileSnapshot>,
    project: HashMap<Buffer, ElmProject>,
    knowledge_base: KnowledgeBase,
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
                        // root: find_project_root(&path)?.to_path_buf(),
                        root: self
                            .knowledge_base
                            .ask(&ProjectRoot(path))
                            .to_path_buf(),
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

        if is_new_revision(&mut self.last_checked_revisions, &snapshot)
            && does_snapshot_compile(project, &snapshot)?
        {
            self.analysis_sender
                .send(analysis_thread::Msg::CompilationSucceeded(snapshot))?;
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

#[derive(PartialEq, Clone, Eq, Hash)]
struct ProjectRoot(PathBuf);

struct KnowledgeBase {
    project_root: HashMap<ProjectRoot, PathBuf>,
}

impl Query<ProjectRoot> for KnowledgeBase {
    type Answer = PathBuf;

    fn store(&mut self) -> &mut HashMap<ProjectRoot, Self::Answer> {
        &mut self.project_root
    }

    fn answer(
        &mut self,
        ProjectRoot(maybe_root): &ProjectRoot,
    ) -> Self::Answer {
        if maybe_root.join("elm.json").exists() {
            maybe_root.to_owned()
        } else {
            // TODO: find a way that queries for multiple file can share a
            // reference to the same PathBuf.
            self.ask(&ProjectRoot(maybe_root.parent().unwrap().to_owned()))
                .to_owned()
        }
    }
}

mod knowledge_base {
    use std::collections::HashMap;

    pub trait Query<Q>
    where
        Q: std::hash::Hash + std::cmp::Eq + Clone + 'static,
    {
        type Answer;

        // Functions to implement for concrete query types.
        fn answer(&mut self, question: &Q) -> Self::Answer;
        fn store(&mut self) -> &mut HashMap<Q, Self::Answer>;

        // Function to call for making queries.
        fn ask(&mut self, question: &Q) -> &Self::Answer {
            if !self.store().contains_key(question) {
                let answer = self.answer(question);
                self.store().insert(question.clone(), answer);
            }
            self.store().get(question).unwrap()
        }
    }
}
