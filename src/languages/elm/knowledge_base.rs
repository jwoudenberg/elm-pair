use crate::languages::elm::idat;
use crate::support::source_code::Buffer;
use crate::Error;
use ropey::RopeSlice;
use serde::Deserialize;
use std::collections::HashMap;
use std::io::BufReader;
use std::iter::FromIterator;
use std::path::{Path, PathBuf};

pub struct KnowledgeBase {
    buffer_path: HashMap<Buffer, PathBuf>,
    buffer_project: HashMap<Buffer, Project>,
    compilation_params: HashMap<Buffer, CompilationParams>,
    project_root: HashMap<PathBuf, PathBuf>,
}

macro_rules! memoize {
    ($cache:expr, $question:expr, $answer:expr) => {{
        if !$cache.contains_key($question) {
            let res = match $answer {
                Ok(ok) => ok,
                Err(err) => return Err(err),
            };
            $cache.insert($question.clone(), res);
        }
        Ok($cache.get($question).unwrap())
    }};
}

impl KnowledgeBase {
    pub fn new() -> KnowledgeBase {
        KnowledgeBase {
            buffer_path: HashMap::new(),
            buffer_project: HashMap::new(),
            project_root: HashMap::new(),
            compilation_params: HashMap::new(),
        }
    }

    pub(crate) fn constructors_for_type<'a, 'b>(
        &'a mut self,
        buffer: Buffer,
        module_name: RopeSlice<'b>,
        type_name: RopeSlice<'b>,
    ) -> Result<&'a Vec<String>, Error> {
        self.module_exports(buffer, module_name)?
            .iter()
            .find_map(|export| match export {
                ElmExport::Value { .. } => None,
                ElmExport::Type { name, constructors } => {
                    if type_name.eq(name) {
                        Some(constructors)
                    } else {
                        None
                    }
                }
            })
            .ok_or_else(|| Error::ElmNoSuchTypeInModule {
                module_name: module_name.to_string(),
                type_name: type_name.to_string(),
            })
    }

    pub(crate) fn module_exports(
        &mut self,
        buffer: Buffer,
        module: RopeSlice,
    ) -> Result<&Vec<ElmExport>, Error> {
        let project = self.buffer_project(buffer)?;
        match project.modules.get(&module.to_string()) {
            None => panic!("no such module"),
            Some(ElmModule::_InProject { .. }) => panic!("not implemented yet"),
            Some(ElmModule::FromDependency { exposed_modules }) => {
                Ok(exposed_modules)
            }
        }
    }

    pub(crate) fn buffer_project(
        &mut self,
        buffer: Buffer,
    ) -> Result<&Project, Error> {
        memoize!(self.buffer_project, &buffer, {
            // TODO: Avoid need to copy buffer_path here.
            let buffer_path = self.buffer_path(buffer)?.to_owned();
            let project_root = self.project_root(&buffer_path)?;
            let file =
                std::fs::File::open(project_root.join("elm.json")).unwrap();
            let reader = BufReader::new(file);
            let _elm_json: ElmJson = serde_json::from_reader(reader).unwrap();
            let modules_from_dependencies =
                from_idat(project_root.join("elm-stuff/0.19.1/i.dat"))?;
            let project = Project {
                // TODO: Add project sourcefiles.
                // TODO: Remove harcoded Elm version.
                modules: modules_from_dependencies,
            };
            Ok(project)
        })
    }

    pub(crate) fn get_compilation_params(
        &mut self,
        buffer: Buffer,
    ) -> Result<&CompilationParams, Error> {
        memoize!(self.compilation_params, &buffer, {
            let buffer_path = self.buffer_path(buffer)?.to_owned();
            let root = self.project_root(&buffer_path)?.to_owned();
            let elm_bin = find_executable("elm")?;
            Ok(CompilationParams { root, elm_bin })
        })
    }

    pub(crate) fn insert_buffer_path(&mut self, buffer: Buffer, path: PathBuf) {
        self.buffer_path.insert(buffer, path);
    }

    fn buffer_path(&mut self, buffer: Buffer) -> Result<&PathBuf, Error> {
        memoize!(self.buffer_path, &buffer, {
            Err(Error::ElmNoProjectStoredForBuffer(buffer))
        })
    }

    // TODO: return path to `elm.json` instead.
    fn project_root(&mut self, maybe_root: &Path) -> Result<&PathBuf, Error> {
        memoize!(self.project_root, &maybe_root.to_path_buf(), {
            if maybe_root.join("elm.json").exists() {
                Ok(maybe_root.to_owned())
            } else {
                // TODO: find a way that queries for multiple file can share a
                // reference to the same PathBuf.
                match maybe_root.parent() {
                    None => Err(Error::NoElmJsonFoundInAnyAncestorDirectory),
                    Some(parent) => {
                        let root = self.project_root(&parent.to_owned())?;
                        Ok(root.to_owned())
                    }
                }
            }
        })
    }
}

pub struct CompilationParams {
    // Root of the Elm project containing this source file.
    pub root: PathBuf,
    // Absolute path to the `elm` compiler.
    pub elm_bin: PathBuf,
}

#[derive(Debug)]
pub struct Project {
    pub modules: HashMap<String, ElmModule>,
}

#[derive(Debug)]
pub enum ElmModule {
    _InProject { path: PathBuf },
    FromDependency { exposed_modules: Vec<ElmExport> },
}

#[derive(Debug)]
pub enum ElmExport {
    Value {
        name: String,
    },
    Type {
        name: String,
        constructors: Vec<String>,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct ElmJson {
    #[serde(rename = "source-directories")]
    _source_directories: Vec<PathBuf>,
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

fn from_idat(path: PathBuf) -> Result<HashMap<String, ElmModule>, Error> {
    let file = std::fs::File::open(path).unwrap();
    let reader = BufReader::new(file);
    let iter =
        idat::parse(reader)?
            .into_iter()
            .filter_map(|(canonical_name, i)| {
                let idat::Name(name) = canonical_name.module;
                let module = elm_module_from_interface(i)?;
                Some((name, module))
            });
    Ok(HashMap::from_iter(iter))
}

fn elm_module_from_interface(
    dep_i: idat::DependencyInterface,
) -> Option<ElmModule> {
    if let idat::DependencyInterface::Public(interface) = dep_i {
        // TODO: add binops
        let values = interface.values.into_iter().map(elm_export_from_value);
        let unions = interface.unions.into_iter().map(elm_export_from_union);
        let aliases = interface.aliases.into_iter().map(elm_export_from_alias);
        let exposed_modules =
            Vec::from_iter(values.chain(unions).chain(aliases));
        Some(ElmModule::FromDependency { exposed_modules })
    } else {
        None
    }
}

fn elm_export_from_value(
    (idat::Name(name), _): (idat::Name, idat::CanonicalAnnotation),
) -> ElmExport {
    ElmExport::Value { name }
}

fn elm_export_from_union(
    (idat::Name(name), union): (idat::Name, idat::Union),
) -> ElmExport {
    let constructor_names = |canonical_union: idat::CanonicalUnion| {
        let iter = canonical_union
            .alts
            .into_iter()
            .map(|idat::Ctor(idat::Name(name), _, _, _)| name);
        Vec::from_iter(iter)
    };
    let constructors = match union {
        idat::Union::Open(canonical_union) => {
            constructor_names(canonical_union)
        }
        idat::Union::Closed(canonical_union) => {
            constructor_names(canonical_union)
        }
        idat::Union::Private(_) =>
        // We're reading this information for use by other modules.
        // These external modules can't see private constructors,
        // so we don't need to return them here.
        {
            Vec::new()
        }
    };
    ElmExport::Type { name, constructors }
}

fn elm_export_from_alias(
    (idat::Name(name), _): (idat::Name, idat::Alias),
) -> ElmExport {
    ElmExport::Type {
        name,
        constructors: Vec::new(),
    }
}
