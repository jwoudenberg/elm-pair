use crate::support::source_code::Buffer;
use crate::Error;
use std::collections::HashMap;
use std::path::PathBuf;

pub struct KnowledgeBase {
    buffer_path: HashMap<BufferPath, PathBuf>,
    project_root: HashMap<ProjectRoot, PathBuf>,
    compilation_params: HashMap<GetCompilationParams, CompilationParams>,
}

impl KnowledgeBase {
    pub fn new() -> KnowledgeBase {
        KnowledgeBase {
            buffer_path: HashMap::new(),
            project_root: HashMap::new(),
            compilation_params: HashMap::new(),
        }
    }

    pub(crate) fn insert_buffer_path(&mut self, buffer: Buffer, path: PathBuf) {
        self.insert(BufferPath(buffer), path);
    }

    pub(crate) fn get_compilation_params(
        &mut self,
        buffer: Buffer,
    ) -> Result<&CompilationParams, Error> {
        self.ask(&GetCompilationParams(buffer))
    }
}

#[derive(PartialEq, Clone, Eq, Hash)]
struct BufferPath(Buffer);

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

#[derive(PartialEq, Clone, Eq, Hash)]
struct ProjectRoot(PathBuf);

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

#[derive(PartialEq, Clone, Eq, Hash)]
struct GetCompilationParams(Buffer);

pub struct CompilationParams {
    // Root of the Elm project containing this source file.
    pub root: PathBuf,
    // Absolute path to the `elm` compiler.
    pub elm_bin: PathBuf,
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

pub trait Query<Q>
where
    Q: std::hash::Hash + std::cmp::Eq + Clone + 'static,
{
    type Answer;
    type Error;

    // Functions to implement for concrete query types.
    fn answer(&mut self, question: &Q) -> Result<Self::Answer, Self::Error>;
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
