use crate::elm::compiler::Compiler;
use crate::elm::io::parse_elm_json::{parse_elm_json, ElmJson};
use crate::elm::io::parse_elm_module::parse_elm_module;
use crate::elm::io::parse_elm_stuff_idat::parse_elm_stuff_idat;
use crate::elm::queries::exports;
use crate::lib::dir_walker::DirWalker;
use crate::lib::log::Error;
use abomonation_derive::Abomonation;
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub mod parse_elm_json;
pub mod parse_elm_module;
pub mod parse_elm_stuff_idat;

// This trait exists to allow dependency injection of side-effecty functions
// that read and write files into pure dataflow computation logic. The goal is
// to allow the dataflow logic to be tested in isolation.
pub trait ElmIO: Clone {
    type FilesInDir: IntoIterator<Item = PathBuf>;

    fn parse_elm_json(&self, path: &Path) -> Result<ElmJson, Error>;
    fn parse_elm_module(&self, path: &Path)
        -> Result<Vec<ExportedName>, Error>;
    // TODO: Don't allocate the response Vec. Return an interator to dataflow.
    fn parse_elm_stuff_idat(
        &self,
        path: &Path,
    ) -> Result<Vec<(String, ExportedName)>, Error>;
    fn find_files_recursively(&self, path: &Path) -> Self::FilesInDir;
}

#[derive(Abomonation, Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum ExportedName {
    Value {
        name: String,
    },
    Type {
        name: String,
        constructors: Vec<String>,
    },
    // We treat RecordTypeAlias separately from type, because it can be used as
    // both a type and a constructor in imported code, i.e. you can do this:
    //
    //     type alias Point = { x : Int, y : Int }
    //
    //     origin : Point       // using `Point` as a type
    //     origin = Point 0 0   // using `Point` as a constructor
    //
    // Modeling this as a `Type` with name `Point` and a single constructor also
    // named `Point` wouldn't be entirely accurate, because constructors of
    // custom types are imported using `exposing (MyType(..))`, whereas
    // `exposing (Point)` is enough to import both type and constructor in case
    // of a record type alias.
    RecordTypeAlias {
        name: String,
    },
}

#[derive(Clone)]
pub struct RealElmIO {
    compiler: Compiler,
    query_for_exports: Rc<exports::Query>,
}

impl RealElmIO {
    pub fn new(compiler: Compiler) -> Result<RealElmIO, Error> {
        let language = tree_sitter_elm::language();
        let query_for_exports = Rc::new(exports::Query::init(language)?);
        Ok(RealElmIO {
            compiler,
            query_for_exports,
        })
    }
}

impl ElmIO for RealElmIO {
    type FilesInDir = DirWalker;

    fn parse_elm_json(&self, path: &Path) -> Result<ElmJson, Error> {
        parse_elm_json(path)
    }

    fn parse_elm_module(
        &self,
        path: &Path,
    ) -> Result<Vec<ExportedName>, Error> {
        parse_elm_module(&self.query_for_exports, path)
    }

    fn parse_elm_stuff_idat(
        &self,
        path: &Path,
    ) -> Result<Vec<(String, ExportedName)>, Error> {
        parse_elm_stuff_idat(&self.compiler, path)
    }

    fn find_files_recursively(&self, path: &Path) -> Self::FilesInDir {
        DirWalker::new(path)
    }
}

#[cfg(test)]
pub mod mock {
    use super::*;
    use crate::elm::project;
    use crate::lib::log;
    use std::collections::HashMap;
    use std::iter::FromIterator;
    use std::rc::Rc;
    use std::sync::Mutex;

    #[derive(Clone)]
    pub struct FakeElmIO {
        pub projects: Rc<Mutex<HashMap<PathBuf, FakeElmProject>>>,
        pub modules: Rc<Mutex<HashMap<PathBuf, Vec<ExportedName>>>>,
        pub elm_jsons_parsed: Rc<Mutex<u64>>,
        pub elm_modules_parsed: Rc<Mutex<u64>>,
        pub elm_idats_parsed: Rc<Mutex<u64>>,
    }

    #[derive(Clone)]
    pub struct FakeElmProject {
        elm_json: ElmJson,
        dependencies: Vec<(String, ExportedName)>,
    }

    impl FakeElmIO {
        pub fn new(
            projects: Vec<(PathBuf, FakeElmProject)>,
            modules: Vec<(PathBuf, Vec<ExportedName>)>,
        ) -> FakeElmIO {
            FakeElmIO {
                projects: Rc::new(Mutex::new(HashMap::from_iter(
                    projects.into_iter(),
                ))),
                modules: Rc::new(Mutex::new(HashMap::from_iter(
                    modules.into_iter(),
                ))),
                elm_jsons_parsed: Rc::new(Mutex::new(0)),
                elm_modules_parsed: Rc::new(Mutex::new(0)),
                elm_idats_parsed: Rc::new(Mutex::new(0)),
            }
        }
    }

    impl ElmIO for FakeElmIO {
        type FilesInDir = Vec<PathBuf>;

        fn parse_elm_json(&self, path: &Path) -> Result<ElmJson, Error> {
            if path.file_name() != Some(std::ffi::OsStr::new("elm.json")) {
                return Err(log::mk_err!("not an elm.json file: {:?}", path));
            }
            let mut elm_jsons_parsed = self.elm_jsons_parsed.lock().unwrap();
            let project_root = project::root_from_elm_json_path(path)?;
            *elm_jsons_parsed += 1;
            self.projects
                .lock()
                .unwrap()
                .get(project_root)
                .ok_or_else(|| log::mk_err!("did not find project {:?}", path))
                .map(|project| project.elm_json.clone())
        }

        fn parse_elm_module(
            &self,
            path: &Path,
        ) -> Result<Vec<ExportedName>, Error> {
            let mut elm_modules_parsed =
                self.elm_modules_parsed.lock().unwrap();
            let opt_module = self
                .modules
                .lock()
                .unwrap()
                .get(path)
                .map(std::clone::Clone::clone);
            if let Some(module) = opt_module {
                *elm_modules_parsed += 1;
                Ok(module)
            } else {
                Ok(Vec::new())
            }
        }

        fn parse_elm_stuff_idat(
            &self,
            path: &Path,
        ) -> Result<Vec<(String, ExportedName)>, Error> {
            let projects = self.projects.lock().unwrap();
            let project_root = project::root_from_idat_path(path)?;
            let project = projects.get(project_root).ok_or_else(|| {
                log::mk_err!("did not find project {:?}", project_root)
            })?;
            let mut elm_idats_parsed = self.elm_idats_parsed.lock().unwrap();
            *elm_idats_parsed += 1;
            let dependencies = project.dependencies.clone();
            Ok(dependencies)
        }

        fn find_files_recursively(&self, dir: &Path) -> Self::FilesInDir {
            self.modules
                .lock()
                .unwrap()
                .keys()
                .filter(|path| path.starts_with(dir))
                .map(PathBuf::clone)
                .collect()
        }
    }

    pub fn mk_project(
        root: &Path,
        src_dirs: Vec<&str>,
        dep_mods: Vec<&str>,
    ) -> (PathBuf, FakeElmProject) {
        (
            root.to_owned(),
            FakeElmProject {
                elm_json: ElmJson {
                    source_directories: src_dirs
                        .into_iter()
                        .map(PathBuf::from)
                        .collect(),
                },
                dependencies: dep_mods
                    .into_iter()
                    .map(|name| {
                        (
                            name.to_string(),
                            ExportedName::Value {
                                name: "ants".to_string(),
                            },
                        )
                    })
                    .collect(),
            },
        )
    }

    pub fn mk_module(path: &str) -> (PathBuf, Vec<ExportedName>) {
        (
            PathBuf::from(path),
            vec![ExportedName::Value {
                name: "bees".to_string(),
            }],
        )
    }
}
