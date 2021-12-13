use crate::analysis_thread;
use crate::sized_stack::SizedStack;
use crate::support::source_code::{Buffer, SourceFileSnapshot};
use crate::{Error, MsgLoop};
use knowledge_base::Query;
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
        kb: KnowledgeBase {
            buffer_path: HashMap::new(),
            project_root: HashMap::new(),
            compilation_params: HashMap::new(),
        },
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
                self.kb.insert(BufferPath(buffer), path);
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
                self.kb.ask(&GetCompilationParams(snapshot.buffer))?;
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

struct KnowledgeBase {
    buffer_path: HashMap<BufferPath, PathBuf>,
    project_root: HashMap<ProjectRoot, PathBuf>,
    compilation_params: HashMap<GetCompilationParams, CompilationParams>,
}

#[derive(PartialEq, Clone, Eq, Hash)]
struct BufferPath(Buffer);

#[derive(PartialEq, Clone, Eq, Hash)]
struct GetCompilationParams(Buffer);

struct CompilationParams {
    // Root of the Elm project containing this source file.
    root: PathBuf,
    // Absolute path to the `elm` compiler.
    elm_bin: PathBuf,
}

#[derive(PartialEq, Clone, Eq, Hash)]
struct ProjectRoot(PathBuf);

impl Query<BufferPath> for KnowledgeBase {
    type Answer = PathBuf;
    type Error = Error;

    fn store(&mut self) -> &mut HashMap<BufferPath, Self::Answer> {
        &mut self.buffer_path
    }

    fn answer(
        &mut self,
        BufferPath(buffer): &BufferPath,
    ) -> Result<Self::Answer, Self::Error> {
        Err(Error::ElmNoProjectStoredForBuffer(*buffer))
    }
}

impl Query<ProjectRoot> for KnowledgeBase {
    type Answer = PathBuf;
    type Error = Error;

    fn store(&mut self) -> &mut HashMap<ProjectRoot, Self::Answer> {
        &mut self.project_root
    }

    fn answer(
        &mut self,
        ProjectRoot(maybe_root): &ProjectRoot,
    ) -> Result<Self::Answer, Self::Error> {
        if maybe_root.join("elm.json").exists() {
            Ok(maybe_root.to_owned())
        } else {
            // TODO: find a way that queries for multiple file can share a
            // reference to the same PathBuf.
            match maybe_root.parent() {
                None => Err(Error::NoElmJsonFoundInAnyAncestorDirectory),
                Some(parent) => {
                    let root = self.ask(&ProjectRoot(parent.to_owned()))?;
                    Ok(root.to_owned())
                }
            }
        }
    }
}

impl Query<GetCompilationParams> for KnowledgeBase {
    type Answer = CompilationParams;
    type Error = Error;

    fn store(&mut self) -> &mut HashMap<GetCompilationParams, Self::Answer> {
        &mut self.compilation_params
    }

    fn answer(
        &mut self,
        GetCompilationParams(buffer): &GetCompilationParams,
    ) -> Result<Self::Answer, Self::Error> {
        let buffer_path = self.ask(&BufferPath(*buffer))?.to_owned();
        let root = self.ask(&ProjectRoot(buffer_path))?.to_owned();
        let elm_bin = find_executable("elm")?;
        Ok(CompilationParams { root, elm_bin })
    }
}

mod knowledge_base {
    use std::collections::HashMap;

    pub trait Query<Q>
    where
        Q: std::hash::Hash + std::cmp::Eq + Clone + 'static,
    {
        type Answer;
        type Error;

        // Functions to implement for concrete query types.
        fn answer(&mut self, question: &Q)
            -> Result<Self::Answer, Self::Error>;
        fn store(&mut self) -> &mut HashMap<Q, Self::Answer>;

        // Function to call for making queries.
        fn ask(&mut self, question: &Q) -> Result<&Self::Answer, Self::Error> {
            if !self.store().contains_key(question) {
                let answer = self.answer(question)?;
                self.store().insert(question.clone(), answer);
            }
            Ok(self.store().get(question).unwrap())
        }

        // Inserting information manually, instead of on-demand
        fn insert(&mut self, question: Q, answer: Self::Answer) {
            self.store().insert(question, answer);
        }
    }
}
