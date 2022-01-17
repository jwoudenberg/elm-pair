use crate::elm::compiler::Compiler;
use crate::elm::idat;
use crate::support::log;
use crate::support::log::Error;
use abomonation_derive::Abomonation;
use differential_dataflow::operators::Join;
use differential_dataflow::operators::Reduce;
use differential_dataflow::operators::Threshold;
use serde::Deserialize;
use std::collections::HashMap;
use std::collections::HashSet;
use std::io::{BufReader, Read};
use std::iter::FromIterator;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{channel, Receiver};
use std::sync::RwLock;
use tree_sitter::{Language, Node, Query, QueryCursor, Tree};

pub struct DataflowComputation {
    // The dataflow worker contains state managed by the differential-dataflow
    // library. Differential-dataflow supports having multiple workers share
    // work, but we don't make use of that.
    worker: timely::worker::Worker<
        timely::communication::allocator::thread::Thread,
    >,
    // These probes let us check whether the dataflow computation has processed
    // all changes made to the inputs below, i.e. whether the outputs will show
    // up-to-date information.
    probes: Vec<DataflowProbe>,
    // Changes made to the dataflow inputs below will receive this timestamp.
    current_time: Timestamp,
    // An input representing projects we're currently tracking.
    project_roots_input: DataflowInput<PathBuf>,
    // An input representing events happening to files. Whether it's file
    // creation, removal, or modification, we just push a path in here to let
    // it know something's changed.
    filepath_events_input: DataflowInput<PathBuf>,
    // A channel receiver that will receive events for changes to files in Elm
    // projects being tracked.
    file_event_receiver: Receiver<notify::DebouncedEvent>,
    // A map containing all parsed informations for all projects currently being
    // tracked.
    //
    // TODO: remove this field with something better.
    // This structure needs to be rebuilt anytype any code in any Elm project
    // changes. Given no code needs all this information in one place together
    // it's wasteful to construct it.
    projects: Rc<RwLock<HashMap<PathBuf, ProjectInfo>>>,
}

type Timestamp = u32;

type DataflowInput<A> =
    differential_dataflow::input::InputSession<Timestamp, A, isize>;

type DataflowProbe = timely::dataflow::operators::probe::Handle<Timestamp>;

// This trait exists to allow dependency injection of side-effecty functions
// that read and write files into pure dataflow computation logic. The goal is
// to allow the dataflow logic to be tested in isolation.
trait ElmIO: Clone {
    type FileWatcher: notify::Watcher + 'static;
    type FilesInDir: IntoIterator<Item = PathBuf>; // TODO: remove this type.

    fn parse_elm_json(&self, path: &Path) -> Result<ElmJson, Error>;
    fn parse_elm_module(
        &self,
        query_for_exports: &QueryForExports,
        path: &Path,
    ) -> Result<Option<ElmModule>, Error>;
    fn parse_elm_stuff_idat(
        &self,
        path: &Path,
    ) -> Result<Vec<(String, ElmModule)>, Error>;
    fn find_elm_files_recursively(&self, path: &Path) -> Self::FilesInDir;
}

#[derive(Clone)]
struct RealElmIO {
    compiler: Compiler,
}

impl ElmIO for RealElmIO {
    type FileWatcher = notify::RecommendedWatcher;
    type FilesInDir = DirWalker;

    fn parse_elm_json(&self, path: &Path) -> Result<ElmJson, Error> {
        let file = std::fs::File::open(path).map_err(|err| {
            log::mk_err!("error while reading elm.json: {:?}", err)
        })?;
        let reader = BufReader::new(file);
        serde_json::from_reader(reader).map_err(|err| {
            log::mk_err!("error while parsing elm.json: {:?}", err)
        })
    }

    fn parse_elm_module(
        &self,
        query_for_exports: &QueryForExports,
        path: &Path,
    ) -> Result<Option<ElmModule>, Error> {
        let mut file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(err) => {
                if let std::io::ErrorKind::NotFound = err.kind() {
                    return Ok(None);
                } else {
                    return Err(log::mk_err!(
                        "failed to open module file: {:?}",
                        err
                    ));
                };
            }
        };
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(|err| {
            log::mk_err!("failed to read module file: {:?}", err)
        })?;
        let tree = crate::support::source_code::parse_bytes(&bytes)?;
        let exports = query_for_exports.run(&tree, &bytes)?;
        let elm_module = ElmModule { exports };
        Ok(Some(elm_module))
    }

    fn parse_elm_stuff_idat(
        &self,
        project_root: &Path,
    ) -> Result<Vec<(String, ElmModule)>, Error> {
        let path = &idat_path(project_root);
        let file = std::fs::File::open(path).or_else(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                create_elm_stuff(&self.compiler, project_root)?;
                std::fs::File::open(path).map_err(|err| {
                    log::mk_err!(
                        "error opening elm-stuff/i.dat file: {:?}",
                        err
                    )
                })
            } else {
                Err(log::mk_err!(
                    "error opening elm-stuff/i.dat file: {:?}",
                    err
                ))
            }
        })?;
        let reader = BufReader::new(file);
        let modules = idat::parse(reader)?
            .into_iter()
            .filter_map(|(canonical_name, i)| {
                let idat::Name(name) = canonical_name.module;
                let module = elm_module_from_interface(i)?;
                Some((name, module))
            })
            .collect();
        Ok(modules)
    }

    fn find_elm_files_recursively(&self, path: &Path) -> Self::FilesInDir {
        DirWalker::new(path)
    }
}

impl DataflowComputation {
    pub(crate) fn new(
        compiler: Compiler,
    ) -> Result<DataflowComputation, Error> {
        Self::new_configurable(RealElmIO { compiler })
    }

    fn new_configurable<D>(elm_io: D) -> Result<DataflowComputation, Error>
    where
        D: ElmIO + 'static,
    {
        let language = tree_sitter_elm::language();
        let query_for_exports = QueryForExports::init(language)?;

        let alloc = timely::communication::allocator::thread::Thread::new();
        let projects = Rc::new(RwLock::new(HashMap::new()));
        let mut worker =
            timely::worker::Worker::new(timely::WorkerConfig::default(), alloc);

        let mut project_roots_input =
            differential_dataflow::input::InputSession::new();
        let mut filepath_events_input =
            differential_dataflow::input::InputSession::new();

        let (file_event_sender, file_event_receiver) = channel();
        let file_watcher: D::FileWatcher = notify::Watcher::new(
            file_event_sender,
            core::time::Duration::from_millis(100),
        )
        .map_err(|err| {
            log::mk_err!("failed creating file watcher: {:?}", err)
        })?;

        let probes = worker.dataflow(|scope| {
            dataflow_graph(
                elm_io,
                scope,
                query_for_exports,
                &mut project_roots_input,
                &mut filepath_events_input,
                projects.clone(),
                file_watcher,
            )
        });

        let mut computation = DataflowComputation {
            worker,
            probes,
            project_roots_input,
            filepath_events_input,
            projects,
            file_event_receiver,
            current_time: 0,
        };

        computation.advance();

        Ok(computation)
    }

    pub(crate) fn watch_project(&mut self, project_root: PathBuf) {
        self.project_roots_input.insert(project_root)
    }

    pub(crate) fn _unwatch_project(&mut self, project_root: PathBuf) {
        self.project_roots_input.remove(project_root)
    }

    pub(crate) fn advance(&mut self) {
        let DataflowComputation {
            worker,
            project_roots_input,
            filepath_events_input,
            probes,
            current_time,
            file_event_receiver,
            projects: _projects,
        } = self;
        while let Ok(event) = file_event_receiver.try_recv() {
            let mut push_event = |path: PathBuf| {
                if is_elm_file(&path) {
                    filepath_events_input.insert(path)
                }
            };
            match event {
                notify::DebouncedEvent::NoticeWrite(_) => {}
                notify::DebouncedEvent::NoticeRemove(_) => {}
                notify::DebouncedEvent::Create(path)
                | notify::DebouncedEvent::Chmod(path)
                | notify::DebouncedEvent::Write(path)
                | notify::DebouncedEvent::Remove(path) => push_event(path),
                notify::DebouncedEvent::Rename(from, to) => {
                    push_event(from);
                    push_event(to);
                }
                notify::DebouncedEvent::Rescan => {
                    // TODO: Do something smart here.
                }
                notify::DebouncedEvent::Error(err, opt_path) => {
                    log::error!(
                        "File watcher error related to file {:?}: {:?}",
                        opt_path,
                        err
                    );
                }
            }
        }

        project_roots_input.advance_to(*current_time);
        project_roots_input.flush();
        filepath_events_input.advance_to(*current_time);
        filepath_events_input.flush();

        worker.step_while(|| {
            probes.iter().any(|probe| probe.less_than(current_time))
        });
        self.current_time += 1;
    }

    pub(crate) fn with_project<F, R>(
        &self,
        root: &Path,
        f: F,
    ) -> Result<R, Error>
    where
        F: FnOnce(&ProjectInfo) -> Result<R, Error>,
    {
        let projects = self.projects.read().map_err(|err| {
            log::mk_err!("failed to obtain read lock for projects: {:?}", err)
        })?;
        let project = projects.get(root).ok_or_else(|| {
            log::mk_err!("did not find project for path {:?}", root)
        })?;
        f(project)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectInfo {
    pub modules: HashMap<String, ElmModule>,
}

#[derive(
    Abomonation, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize,
)]
pub struct ElmModule {
    pub exports: Vec<ExportedName>,
}

#[derive(
    Abomonation, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize,
)]
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

#[derive(
    Abomonation, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize,
)]
#[serde(rename_all = "kebab-case")]
struct ElmJson {
    #[serde(rename = "source-directories")]
    source_directories: Vec<PathBuf>,
}

// TODO: Clean up, removing clone's
// TODO: Introduce ProjectId type to replace `project_root` in most places.
// TODO: Introduce FileId type to replace PathBuf in most places.
// TODO: Ensure new paths are canonicalized before they're used.
fn dataflow_graph<W, G, D>(
    elm_io: D,
    scope: &mut G,
    query_for_exports: QueryForExports,
    project_roots_input: &mut DataflowInput<PathBuf>,
    filepath_events_input: &mut DataflowInput<PathBuf>,
    projects: Rc<RwLock<HashMap<PathBuf, ProjectInfo>>>,
    mut file_watcher: W,
) -> Vec<DataflowProbe>
where
    W: notify::Watcher + 'static,
    G: timely::dataflow::scopes::ScopeParent<Timestamp = Timestamp>
        + timely::dataflow::scopes::Scope,
    D: ElmIO + 'static,
{
    let project_roots = project_roots_input.to_collection(scope).distinct();

    let filepath_events = filepath_events_input.to_collection(scope);

    let elm_io2 = elm_io.clone();
    let elm_io3 = elm_io.clone();
    let elm_io4 = elm_io.clone();

    let elm_jsons = project_roots.flat_map(move |project_root: PathBuf| {
        match elm_io.parse_elm_json(&elm_json_path(&project_root)) {
            Ok(elm_json) => Some((project_root, elm_json)),
            Err(err) => {
                log::error!("Failed to load elm_json: {:?}", err);
                None
            }
        }
    });

    let source_directories_by_project = elm_jsons
        .flat_map(|(project_root, elm_json)| {
            elm_json
                .source_directories
                .into_iter()
                .map(move |dir| (project_root.clone(), project_root.join(&dir)))
        })
        .distinct();

    let source_directories = source_directories_by_project
        .map(|(_, path)| path)
        .distinct();

    // This collection can intentionally contain files multiple times.
    // A new entry should be added whenever we receive an event for a file,
    // like a modification or removal. Useful for logic that needs to rerun on
    // those occasions.
    let file_events = source_directories
        .flat_map(move |path| elm_io2.find_elm_files_recursively(&path))
        .concat(&filepath_events);

    let parsed_modules = file_events.map(|path| (path, ())).reduce(
        move |path, _input, output| match elm_io3
            .parse_elm_module(&query_for_exports, path)
        {
            Ok(Some(module)) => output.push((module, 1)),
            Ok(None) => {}
            Err(err) => {
                log::error!("Failed parsing module: {:?}", err);
            }
        },
    );

    let files = file_events.distinct();

    let project_modules = files
        // Join on `()`, i.e. create a record for every combination of
        // source path and source directory. Then later we can filter
        // that down to keep just the combinations where the path is
        // in the directory.
        .map(|path| ((), path))
        .join(&source_directories_by_project.map(|x| ((), x)))
        .flat_map(|((), (file_path, (project_root, src_dir)))| {
            if file_path.starts_with(&src_dir) {
                Some((file_path, (project_root, src_dir)))
            } else {
                None
            }
        })
        .join_map(
            &parsed_modules,
            |file_path, (project_root, src_dir), parsed_module| {
                match module_name_from_path(src_dir, file_path) {
                    Ok(module_name) => Some((
                        project_root.clone(),
                        (module_name, parsed_module.clone()),
                    )),
                    Err(err) => {
                        log::error!("Failed deriving module name: {:?}", err);
                        None
                    }
                }
            },
        )
        .flat_map(|opt| opt);

    let paths_to_watch = source_directories_by_project
        .map(|(_project_root, path)| path)
        .concat(&project_roots.map_in_place(|path| *path = elm_json_path(path)))
        .concat(&project_roots.map_in_place(|path| *path = idat_path(path)))
        .distinct()
        .inspect(move |(path, _, diff)| match std::cmp::Ord::cmp(diff, &0) {
            std::cmp::Ordering::Equal => {}
            std::cmp::Ordering::Less => {
                if let Err(err) = file_watcher.unwatch(path) {
                    log::error!(
                        "failed while remove path {:?} to watch for changes: {:?}",
                        path,
                        err
                    )
                }
            }
            std::cmp::Ordering::Greater => {
                if let Err(err) = file_watcher.watch(path, notify::RecursiveMode::Recursive) {
                    log::error!(
                        "failed while adding path {:?} to watch for changes: {:?}",
                        path,
                        err
                    )
                }
            }
        });

    let project_idats = project_roots.flat_map(move |path| {
        match elm_io4.parse_elm_stuff_idat(&path) {
            Ok(modules) => Some((path, modules)),
            Err(err) => {
                log::error!("could not read i.dat file: {:?}", err);
                None
            }
        }
    });

    let project_infos = project_modules
        .reduce(|_project_root, inputs, output| {
            let modules: Vec<(String, ElmModule)> = Vec::from_iter(
                inputs.iter().map(|(module, _count)| (*module).clone()),
            );
            output.push((modules, 1));
        })
        .join(&project_idats)
        .map(move |(project_root, (local_modules, mut idat_modules))| {
            idat_modules.extend(local_modules);
            (
                project_root,
                ProjectInfo {
                    modules: idat_modules.into_iter().collect(),
                },
            )
        })
        .inspect(move |((project_root, project_info), _, diff)| {
            match std::cmp::Ord::cmp(diff, &0) {
                std::cmp::Ordering::Equal => {}
                std::cmp::Ordering::Less => match projects.write() {
                    Ok(mut projects_write) => {
                        projects_write.remove(project_root);
                    }
                    Err(err) => {
                        log::error!("no write lock on projects: {:?}", err);
                    }
                },
                std::cmp::Ordering::Greater => match projects.write() {
                    Ok(mut projects_write) => {
                        projects_write
                            .insert(project_root.clone(), project_info.clone());
                    }
                    Err(err) => {
                        log::error!("no write lock on projects: {:?}", err);
                    }
                },
            }
        });

    vec![paths_to_watch.probe(), project_infos.probe()]
}

#[cfg(test)]
mod dataflow_tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Clone)]
    struct FakeElmIO {
        projects: Rc<Mutex<HashMap<PathBuf, ElmJson>>>,
        modules: Rc<Mutex<HashMap<PathBuf, ElmModule>>>,
        elm_jsons_parsed: Rc<Mutex<u64>>,
        elm_modules_parsed: Rc<Mutex<u64>>,
        elm_idats_parsed: Rc<Mutex<u64>>,
    }

    impl FakeElmIO {
        fn new(
            projects: Vec<(PathBuf, ElmJson)>,
            modules: Vec<(PathBuf, ElmModule)>,
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
        type FileWatcher = notify::NullWatcher;
        type FilesInDir = Vec<PathBuf>;

        fn parse_elm_json(&self, path: &Path) -> Result<ElmJson, Error> {
            if path.file_name() != Some(std::ffi::OsStr::new("elm.json")) {
                return Err(log::mk_err!("not an elm.json file: {:?}", path));
            }
            let mut elm_jsons_parsed = self.elm_jsons_parsed.lock().unwrap();
            *elm_jsons_parsed += 1;
            self.projects
                .lock()
                .unwrap()
                .get(path.parent().unwrap())
                .ok_or_else(|| log::mk_err!("did not find project {:?}", path))
                .map(ElmJson::clone)
        }

        fn parse_elm_module(
            &self,
            _query_for_exports: &QueryForExports,
            path: &Path,
        ) -> Result<Option<ElmModule>, Error> {
            let mut elm_modules_parsed =
                self.elm_modules_parsed.lock().unwrap();
            let elm_module =
                self.modules.lock().unwrap().get(path).map(ElmModule::clone);
            if elm_module.is_some() {
                *elm_modules_parsed += 1;
            }
            Ok(elm_module)
        }

        fn parse_elm_stuff_idat(
            &self,
            _project_root: &Path,
        ) -> Result<Vec<(String, ElmModule)>, Error> {
            let mut elm_idats_parsed = self.elm_idats_parsed.lock().unwrap();
            *elm_idats_parsed += 1;
            Ok(Vec::new())
        }

        fn find_elm_files_recursively(&self, dir: &Path) -> Self::FilesInDir {
            self.modules
                .lock()
                .unwrap()
                .keys()
                .filter(|path| path.starts_with(dir))
                .map(PathBuf::clone)
                .collect()
        }
    }

    fn mk_project(root: &Path, src_dirs: Vec<&str>) -> (PathBuf, ElmJson) {
        (
            root.to_owned(),
            ElmJson {
                source_directories: src_dirs
                    .into_iter()
                    .map(PathBuf::from)
                    .collect(),
            },
        )
    }

    fn mk_module(path: &str) -> (PathBuf, ElmModule) {
        (
            PathBuf::from(path),
            ElmModule {
                exports: vec![ExportedName::Value {
                    name: "bees".to_string(),
                }],
            },
        )
    }

    fn assert_modules(
        computation: &DataflowComputation,
        project_root: &Path,
        modules: &[&str],
    ) {
        computation
            .with_project(project_root, |project| {
                assert_eq!(
                    project
                        .modules
                        .keys()
                        .map(|key| key.as_str())
                        .collect::<HashSet<&str>>(),
                    modules.iter().copied().collect::<HashSet<&str>>(),
                );
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn project_elm_files_are_found() {
        let project_root = PathBuf::from("/project");
        let elm_io = FakeElmIO::new(
            vec![mk_project(&project_root, vec!["src"])],
            vec![
                mk_module("/project/src/Animals/Bat.elm"),
                mk_module("/project/src/Care/Soap.elm"),
            ],
        );
        let mut computation =
            DataflowComputation::new_configurable(elm_io).unwrap();
        computation.watch_project(project_root.clone());
        computation.advance();
        assert_modules(
            &computation,
            &project_root,
            &["Animals.Bat", "Care.Soap"],
        );
    }

    #[test]
    fn unwatched_projects_are_forgotten() {
        // Given a project with some modules
        let project_root = PathBuf::from("/project");
        let elm_io = FakeElmIO::new(
            vec![mk_project(&project_root, vec!["src"])],
            vec![
                mk_module("/project/src/Animals/Bat.elm"),
                mk_module("/project/src/Care/Soap.elm"),
            ],
        );
        let mut computation =
            DataflowComputation::new_configurable(elm_io).unwrap();
        computation.watch_project(project_root.clone());
        computation.advance();
        assert_modules(
            &computation,
            &project_root,
            &["Animals.Bat", "Care.Soap"],
        );

        // When we unwatch it
        computation._unwatch_project(project_root.clone());
        computation.advance();

        // Then it is forgotten
        if computation
            .with_project(&project_root, |_project| Ok(()))
            .is_ok()
        {
            panic!("Did not expect project to be found");
        }
    }

    #[test]
    fn elm_files_created_after_initial_parse_are_found() {
        // Given a project with an existing module...
        let project_root = PathBuf::from("/project");
        let elm_io = FakeElmIO::new(
            vec![mk_project(&project_root, vec!["src"])],
            vec![mk_module("/project/src/Animals/Bat.elm")],
        );
        let mut computation =
            DataflowComputation::new_configurable(elm_io.clone()).unwrap();
        computation.watch_project(project_root.clone());
        computation.advance();
        assert_modules(&computation, &project_root, &["Animals.Bat"]);
        assert_eq!(*elm_io.elm_modules_parsed.lock().unwrap(), 1);

        // When we add an another module file...
        elm_io
            .modules
            .lock()
            .unwrap()
            .extend(Some(mk_module("/project/src/Elements/Water.elm")));
        computation
            .filepath_events_input
            .insert(PathBuf::from("/project/src/Elements/Water.elm"));
        computation.advance();

        // Then the module file is picked up...
        assert_modules(
            &computation,
            &project_root,
            &["Elements.Water", "Animals.Bat"],
        );
        // And we only parsed the new module...
        assert_eq!(*elm_io.elm_modules_parsed.lock().unwrap(), 2);
    }

    #[test]
    fn projects_can_have_separate_files() {
        let project_root = PathBuf::from("/project");
        let project2_root = PathBuf::from("/project2");
        let elm_io = FakeElmIO::new(
            vec![
                mk_project(&project_root, vec!["src"]),
                mk_project(&project2_root, vec!["src"]),
            ],
            vec![
                mk_module("/project/src/Animals/Bat.elm"),
                mk_module("/project/src/Care/Soap.elm"),
                mk_module("/project2/src/Care/Shampoo.elm"),
            ],
        );
        let mut computation =
            DataflowComputation::new_configurable(elm_io).unwrap();
        computation.watch_project(project_root.clone());
        computation.watch_project(project2_root.clone());
        computation.advance();
        assert_modules(
            &computation,
            &project_root,
            &["Animals.Bat", "Care.Soap"],
        );
        assert_modules(&computation, &project2_root, &["Care.Shampoo"]);
    }

    #[test]
    fn elm_files_are_reparsed_if_we_send_an_event_for_them() {
        // Given a project with an existing module...
        let project_root = PathBuf::from("/project");
        let elm_io = FakeElmIO::new(
            vec![mk_project(&project_root, vec!["src"])],
            vec![mk_module("/project/src/Animals/Bat.elm")],
        );
        let mut computation =
            DataflowComputation::new_configurable(elm_io.clone()).unwrap();
        computation.watch_project(project_root.clone());
        computation.advance();
        assert_modules(&computation, &project_root, &["Animals.Bat"]);
        assert_eq!(*elm_io.elm_modules_parsed.lock().unwrap(), 1);

        // When we send an event for the file...
        computation
            .filepath_events_input
            .insert(PathBuf::from("/project/src/Animals/Bat.elm"));
        computation.advance();

        // Then the module is parsed again...
        assert_eq!(*elm_io.elm_modules_parsed.lock().unwrap(), 2);
    }

    #[test]
    fn deleting_an_elm_file_removes_it_from_a_project() {
        // Given a project with some elm modules...
        let project_root = PathBuf::from("/project");
        let elm_io = FakeElmIO::new(
            vec![mk_project(&project_root, vec!["src"])],
            vec![
                mk_module("/project/src/Animals/Bat.elm"),
                mk_module("/project/src/Care/Soap.elm"),
            ],
        );
        let mut computation =
            DataflowComputation::new_configurable(elm_io.clone()).unwrap();
        computation.watch_project(project_root.clone());
        computation.advance();
        assert_modules(
            &computation,
            &project_root,
            &["Animals.Bat", "Care.Soap"],
        );
        assert_eq!(*elm_io.elm_modules_parsed.lock().unwrap(), 2);

        // When we remove a the file...
        elm_io
            .modules
            .lock()
            .unwrap()
            .remove(&PathBuf::from("/project/src/Animals/Bat.elm"));
        computation
            .filepath_events_input
            .insert(PathBuf::from("/project/src/Animals/Bat.elm"));
        computation.advance();

        // Then the module is removed from the project...
        assert_modules(&computation, &project_root, &["Care.Soap"]);
        // And no additional parsing has taken place...
        assert_eq!(*elm_io.elm_modules_parsed.lock().unwrap(), 2);
    }

    #[test]
    fn elm_json_files_are_reparsed_if_we_send_an_event_for_them() {
        todo!()
    }

    #[test]
    fn elm_idat_files_are_reparsed_if_we_send_an_event_for_them() {
        todo!()
    }

    #[test]
    fn no_unnecessary_double_work_when_projects_share_a_source_directory() {
        todo!()
    }
}

fn is_elm_file(path: &Path) -> bool {
    path.extension() == Some(std::ffi::OsStr::new("elm"))
}

fn elm_json_path(project_root: &Path) -> PathBuf {
    project_root.join("elm.json")
}

fn idat_path(project_root: &Path) -> PathBuf {
    project_root
        .join(format!("elm-stuff/{}/i.dat", crate::elm::compiler::VERSION))
}

// This iterator finds as many files as it can and so logs rather than fails
// when it encounters an error.
struct DirWalker {
    directories: Vec<std::fs::ReadDir>,
}

impl DirWalker {
    fn new(root: &Path) -> DirWalker {
        let directories = match std::fs::read_dir(root) {
            Ok(read_dir) => vec![read_dir],
            Err(err) => {
                log::error!(
                    "error while reading contents of source directory {:?}: {:?}",
                    root,
                    err
                );
                Vec::new()
            }
        };
        DirWalker { directories }
    }
}

impl Iterator for DirWalker {
    type Item = PathBuf;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(read_dir) = self.directories.last_mut() {
            match read_dir.next() {
                None => {
                    self.directories.pop();
                }
                Some(Err(err)) => {
                    log::error!(
                        "error while reading entry of source (sub)directory: {:?}",
                        err
                    );
                }
                Some(Ok(entry)) => match entry.file_type() {
                    Err(err) => {
                        log::error!(
                            "error while reading file type of path {:?}: {:?}",
                            entry.path(),
                            err
                        );
                    }
                    Ok(file_type) => {
                        let path = entry.path();
                        if file_type.is_dir() {
                            match std::fs::read_dir(&path) {
                                Ok(inner_read_dir) => {
                                    self.directories.push(inner_read_dir)
                                }
                                Err(err) => {
                                    log::error!(
                                                    "error while reading contents of source directory {:?}: {:?}",
                                                    path,
                                                    err
                                                );
                                }
                            }
                        } else {
                            return Some(path);
                        }
                    }
                },
            }
        }
        None
    }
}

fn module_name_from_path(
    source_dir: &Path,
    path: &Path,
) -> Result<String, Error> {
    path.with_extension("")
        .strip_prefix(source_dir)
        .map_err(|err| {
            log::mk_err!(
                "error stripping source directory {:?} from elm module path {:?}: {:?}",
                path,
                source_dir,
                err
            )
        })?
        .components()
        .filter_map(|component| {
            if let std::path::Component::Normal(os_str) = component {
                Some(os_str.to_str().ok_or(os_str))
            } else {
                None
            }
        })
        .my_intersperse(Ok("."))
        .collect::<Result<String, &std::ffi::OsStr>>()
        .map_err(|os_str| {
            log::mk_err!(
                "directory segment of Elm module used in module name is not valid UTF8: {:?}",
                os_str
            )
        })
}

crate::elm::query::query!(
    QueryForExports,
    query_for_exports,
    "./queries/exports",
    exposed_all,
    exposed_value,
    exposed_type,
    value,
    type_,
    type_alias,
);

impl QueryForExports {
    fn run(
        &self,
        tree: &Tree,
        code: &[u8],
    ) -> Result<Vec<ExportedName>, Error> {
        let mut cursor = QueryCursor::new();
        let matches = cursor
            .matches(&self.query, tree.root_node(), code)
            .filter_map(|match_| {
                if let [capture, rest @ ..] = match_.captures {
                    Some((capture, rest))
                } else {
                    None
                }
            });
        let mut exposed = ExposedList::Some(HashSet::new());
        let mut exports = Vec::new();
        for (capture, rest) in matches {
            if self.exposed_all == capture.index {
                exposed = ExposedList::All;
            } else if self.exposed_value == capture.index {
                let val = Exposed::Value(code_slice(code, &capture.node)?);
                exposed = exposed.add(val);
            } else if self.exposed_type == capture.index {
                let name_node = capture.node.child(0).ok_or_else(|| {
                    log::mk_err!(
                        "could not find name node of type in exposing list"
                    )
                })?;
                let name = code_slice(code, &name_node)?;
                let val = if capture.node.child(1).is_some() {
                    Exposed::TypeWithConstructors(name)
                } else {
                    Exposed::Type(name)
                };
                exposed = exposed.add(val);
            } else if self.value == capture.index {
                let name = code_slice(code, &capture.node)?;
                if exposed.has(&Exposed::Value(name)) {
                    let export = ExportedName::Value {
                        name: name.to_owned(),
                    };
                    exports.push(export);
                }
            } else if self.type_alias == capture.index {
                let name = code_slice(code, &capture.node)?;
                if exposed.has(&Exposed::Type(name)) {
                    let aliased_type = capture
                        .node
                        .parent()
                        .and_then(|n| n.child_by_field_name("typeExpression"))
                        .and_then(|n| n.child_by_field_name("part"))
                        .map(|n| n.kind());
                    let export = if aliased_type == Some("record_type") {
                        ExportedName::RecordTypeAlias {
                            name: name.to_owned(),
                        }
                    } else {
                        ExportedName::Type {
                            name: name.to_owned(),
                            constructors: Vec::new(),
                        }
                    };
                    exports.push(export);
                }
            } else if self.type_ == capture.index {
                let name = code_slice(code, &capture.node)?;
                if exposed.has(&Exposed::TypeWithConstructors(name)) {
                    let constructors = rest
                        .iter()
                        .map(|ctor_capture| {
                            code_slice(code, &ctor_capture.node)
                                .map(std::borrow::ToOwned::to_owned)
                        })
                        .collect::<Result<Vec<String>, Error>>()?;
                    let export = ExportedName::Type {
                        name: name.to_owned(),
                        constructors,
                    };
                    exports.push(export);
                } else if exposed.has(&Exposed::Type(name)) {
                    let export = ExportedName::Type {
                        name: name.to_owned(),
                        constructors: Vec::new(),
                    };
                    exports.push(export);
                }
            }
        }
        Ok(exports)
    }
}

enum ExposedList<'a> {
    All,
    Some(HashSet<Exposed<'a>>),
}

impl<'a> ExposedList<'a> {
    fn add(mut self, item: Exposed<'a>) -> Self {
        match &mut self {
            ExposedList::All => {}
            ExposedList::Some(items) => {
                items.insert(item);
            }
        }
        self
    }

    fn has(&self, item: &Exposed) -> bool {
        match self {
            ExposedList::All => true,
            ExposedList::Some(items) => items.contains(item),
        }
    }
}

#[derive(Hash, PartialEq)]
enum Exposed<'a> {
    Type(&'a str),
    TypeWithConstructors(&'a str),
    Value(&'a str),
}

impl Eq for Exposed<'_> {}

fn code_slice<'a>(code: &'a [u8], node: &Node) -> Result<&'a str, Error> {
    std::str::from_utf8(&code[node.byte_range()]).map_err(|err| {
        log::mk_err!(
            "Failed to decode code slice for node {} as UTF8: {:?}",
            node.kind(),
            err
        )
    })
}

pub(crate) fn index_for_name(query: &Query, name: &str) -> Result<u32, Error> {
    query.capture_index_for_name(name).ok_or_else(|| {
        log::mk_err!(
            "failed to find index {} in tree-sitter query: {:?}",
            name,
            query
        )
    })
}

// Tust nightlies already contain a `intersperse` iterator. Once that lands
// in stable we should switch over.
pub(crate) trait Intersperse: Iterator {
    fn my_intersperse(self, separator: Self::Item) -> IntersperseState<Self>
    where
        Self::Item: Clone,
        Self: Sized;
}

impl<I: Iterator> Intersperse for I {
    fn my_intersperse(self, separator: Self::Item) -> IntersperseState<I> {
        IntersperseState {
            iterator: self.peekable(),
            separator,
            separator_is_next: false,
        }
    }
}

pub(crate) struct IntersperseState<I: Iterator> {
    iterator: std::iter::Peekable<I>,
    separator: I::Item,
    separator_is_next: bool,
}

impl<I: Iterator> Iterator for IntersperseState<I>
where
    I::Item: Clone,
{
    type Item = I::Item;

    fn next(&mut self) -> Option<Self::Item> {
        if self.iterator.peek().is_none() {
            None
        } else if self.separator_is_next {
            self.separator_is_next = false;
            Some(self.separator.clone())
        } else {
            self.separator_is_next = true;
            self.iterator.next()
        }
    }
}

fn create_elm_stuff(
    compiler: &Compiler,
    project_root: &Path,
) -> Result<(), Error> {
    log::info!(
        "Running `elm make` to generate elm-stuff in project: {:?}",
        project_root
    );
    // Running `elm make` will create elm-stuff. We'll pass it a valid module
    // to compile or `elm make` will return an error. `elm make` would create
    // `elm-stuff` before returning an error, but it'd be difficult to
    // distinguish that expected error from other potential unexpected ones.
    let temp_module = ropey::Rope::from_str(
        "\
        module Main exposing (..)\n\
        val : Int\n\
        val = 4\n\
        ",
    );
    let output = compiler.make(project_root, &temp_module)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(log::mk_err!(
            "failed running elm-make to generate elm-stuff:\n{:?}",
            std::string::String::from_utf8(output.stderr)
        ))
    }
}

fn elm_module_from_interface(
    dep_i: idat::DependencyInterface,
) -> Option<ElmModule> {
    if let idat::DependencyInterface::Public(interface) = dep_i {
        // TODO: add binops
        let values = interface.values.into_iter().map(elm_export_from_value);
        let unions = interface.unions.into_iter().map(elm_export_from_union);
        let aliases = interface.aliases.into_iter().map(elm_export_from_alias);
        let exports = Vec::from_iter(values.chain(unions).chain(aliases));
        Some(ElmModule { exports })
    } else {
        None
    }
}

fn elm_export_from_value(
    (idat::Name(name), _): (idat::Name, idat::CanonicalAnnotation),
) -> ExportedName {
    ExportedName::Value { name }
}

fn elm_export_from_union(
    (idat::Name(name), union): (idat::Name, idat::Union),
) -> ExportedName {
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
        // We're reading this information for use by other modules.
        // These external modules can't see private constructors,
        // so we don't need to return them here.
        idat::Union::Closed(_) => Vec::new(),
        idat::Union::Private(_) => Vec::new(),
    };
    ExportedName::Type { name, constructors }
}

fn elm_export_from_alias(
    (idat::Name(name), _): (idat::Name, idat::Alias),
) -> ExportedName {
    ExportedName::Type {
        name,
        constructors: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::included_answer_test as ia_test;

    macro_rules! exports_scanning_test {
        ($name:ident) => {
            #[test]
            fn $name() {
                let mut path = std::path::PathBuf::new();
                path.push("./tests/exports-scanning");
                let module_name = stringify!($name);
                path.push(module_name.to_owned() + ".elm");
                println!("Run simulation {:?}", &path);
                run_exports_scanning_test(&path);
            }
        };
    }

    fn run_exports_scanning_test(path: &Path) {
        match run_exports_scanning_test_helper(path) {
            Err(err) => {
                eprintln!("{:?}", err);
                panic!();
            }
            Ok(res) => ia_test::assert_eq_answer_in(&res, path),
        }
    }

    fn run_exports_scanning_test_helper(path: &Path) -> Result<String, Error> {
        let language = tree_sitter_elm::language();
        let query_for_exports = QueryForExports::init(language)?;
        let elm_io = RealElmIO {
            compiler: Compiler::new().unwrap(),
        };
        let ElmModule { exports } =
            elm_io.parse_elm_module(&query_for_exports, path)?.unwrap();
        let output = exports
            .into_iter()
            .map(|export| format!("{:?}", export))
            .my_intersperse("\n".to_owned())
            .collect();
        Ok(output)
    }

    exports_scanning_test!(exposing_all);
    exports_scanning_test!(exposing_minimal);
    exports_scanning_test!(hiding_constructors);
}
