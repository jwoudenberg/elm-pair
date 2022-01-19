use crate::elm::compiler::Compiler;
use crate::elm::file_parsing::QueryForExports;
use crate::elm::idat;
use crate::support::dir_walker::DirWalker;
use crate::support::log;
use crate::support::log::Error;
use abomonation_derive::Abomonation;
use differential_dataflow::operators::arrange::ArrangeByKey;
use differential_dataflow::operators::Join;
use differential_dataflow::operators::Reduce;
use differential_dataflow::operators::Threshold;
use differential_dataflow::trace::{Cursor, TraceReader};
use serde::Deserialize;
use std::collections::HashMap;
use std::io::BufReader;
use std::iter::FromIterator;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use timely::dataflow::operators::Probe;
use timely::progress::frontier::AntichainRef;

pub struct DataflowComputation {
    // The dataflow worker contains state managed by the differential-dataflow
    // library. Differential-dataflow supports having multiple workers share
    // work, but we don't make use of that.
    worker: timely::worker::Worker<timely::communication::allocator::thread::Thread>,
    // These probes let us check whether the dataflow computation has processed
    // all changes made to the inputs below, i.e. whether the outputs will show
    // up-to-date information.
    probes: Vec<DataflowProbe>,
    // Changes made to the dataflow inputs below will receive this timestamp.
    current_time: Timestamp,
    // An input representing projects we're currently tracking.
    project_roots_input: DataflowInput<(ProjectId, PathBuf)>,
    // An input representing events happening to files. Whether it's file
    // creation, removal, or modification, we just push a path in here to let
    // it know something's changed.
    filepath_events_input: DataflowInput<PathBuf>,
    // A channel receiver that will receive events for changes to files in Elm
    // projects being tracked.
    file_event_receiver: Receiver<notify::DebouncedEvent>,
    // A trace containing information about all Elm modules in projects
    // currently tracked.
    modules_by_project: DataflowTrace<ProjectId, (String, ElmModule)>,
    // Generated project id's indexed by the project's root path.
    project_ids: HashMap<PathBuf, ProjectId>,
}

type Timestamp = u32;

type Diff = isize;

type Allocator = timely::communication::allocator::Thread;

type DataflowInput<A> = differential_dataflow::input::InputSession<Timestamp, A, Diff>;

type DataflowCollection<'a, A> =
    differential_dataflow::collection::Collection<DataflowScope<'a>, A, Diff>;

type DataflowScope<'a> =
    timely::dataflow::scopes::child::Child<'a, timely::worker::Worker<Allocator>, Timestamp>;

type DataflowProbe = timely::dataflow::operators::probe::Handle<Timestamp>;

type DataflowTrace<K, V> = differential_dataflow::operators::arrange::TraceAgent<
    differential_dataflow::trace::implementations::spine_fueled::Spine<
        K,
        V,
        Timestamp,
        Diff,
        std::rc::Rc<
            differential_dataflow::trace::implementations::ord::OrdValBatch<K, V, Timestamp, Diff>,
        >,
    >,
>;

#[derive(Abomonation, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProjectId(u8); // 256 Elm Project should be enough for everyone.

// This trait exists to allow dependency injection of side-effecty functions
// that read and write files into pure dataflow computation logic. The goal is
// to allow the dataflow logic to be tested in isolation.
trait ElmIO: Clone {
    type FileWatcher: notify::Watcher + 'static;
    type FilesInDir: IntoIterator<Item = PathBuf>;

    fn parse_elm_json(&self, path: &Path) -> Result<ElmJson, Error>;
    fn parse_elm_module(
        &self,
        query_for_exports: &QueryForExports,
        path: &Path,
    ) -> Result<Option<ElmModule>, Error>;
    fn parse_elm_stuff_idat(&self, path: &Path) -> Result<Vec<(String, ElmModule)>, Error>;
    fn find_files_recursively(&self, path: &Path) -> Self::FilesInDir;
}

#[derive(Clone)]
struct RealElmIO {
    compiler: Compiler,
}

impl ElmIO for RealElmIO {
    type FileWatcher = notify::RecommendedWatcher;
    type FilesInDir = DirWalker;

    fn parse_elm_json(&self, path: &Path) -> Result<ElmJson, Error> {
        let file = std::fs::File::open(path)
            .map_err(|err| log::mk_err!("error while reading elm.json: {:?}", err))?;
        let reader = BufReader::new(file);
        let mut elm_json: ElmJson = serde_json::from_reader(reader)
            .map_err(|err| log::mk_err!("error while parsing elm.json: {:?}", err))?;
        let project_root = project_root_from_elm_json_path(path)?;
        for dir in elm_json.source_directories.as_mut_slice() {
            let abs_path = project_root.join(&dir);
            // If we cannot canonicalize the path, likely because it doesn't
            // exist, we still want to keep listing the directory in case it is
            // created in the future.
            *dir = abs_path.canonicalize().unwrap_or(abs_path);
        }
        Ok(elm_json)
    }

    fn parse_elm_module(
        &self,
        query_for_exports: &QueryForExports,
        path: &Path,
    ) -> Result<Option<ElmModule>, Error> {
        crate::elm::file_parsing::parse(query_for_exports, path)
    }

    fn parse_elm_stuff_idat(&self, path: &Path) -> Result<Vec<(String, ElmModule)>, Error> {
        let file = std::fs::File::open(path).or_else(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                let project_root = project_root_from_idat_path(path)?;
                create_elm_stuff(&self.compiler, project_root)?;
                std::fs::File::open(path)
                    .map_err(|err| log::mk_err!("error opening elm-stuff/i.dat file: {:?}", err))
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

    fn find_files_recursively(&self, path: &Path) -> Self::FilesInDir {
        DirWalker::new(path)
    }
}

impl DataflowComputation {
    pub fn new(compiler: Compiler) -> Result<DataflowComputation, Error> {
        Self::new_configurable(RealElmIO { compiler })
    }

    fn new_configurable<D>(elm_io: D) -> Result<DataflowComputation, Error>
    where
        D: ElmIO + 'static,
    {
        let language = tree_sitter_elm::language();
        let query_for_exports = QueryForExports::init(language)?;

        let alloc = timely::communication::allocator::thread::Thread::new();
        let mut worker = timely::worker::Worker::new(timely::WorkerConfig::default(), alloc);

        let mut project_roots_input = differential_dataflow::input::InputSession::new();
        let mut filepath_events_input = differential_dataflow::input::InputSession::new();

        let (file_event_sender, file_event_receiver) = channel();
        let file_watcher: D::FileWatcher =
            notify::Watcher::new(file_event_sender, core::time::Duration::from_millis(100))
                .map_err(|err| log::mk_err!("failed creating file watcher: {:?}", err))?;

        let (modules_by_project, probes) = worker.dataflow(|scope| {
            let project_roots = project_roots_input.to_collection(scope);
            let filepath_events = filepath_events_input.to_collection(scope);
            dataflow_graph(
                elm_io,
                query_for_exports,
                project_roots,
                filepath_events,
                file_watcher,
            )
        });

        let mut computation = DataflowComputation {
            worker,
            probes,
            project_roots_input,
            filepath_events_input,
            file_event_receiver,
            current_time: 0,
            modules_by_project,
            project_ids: HashMap::new(),
        };

        computation.advance();

        Ok(computation)
    }

    pub fn watch_project(&mut self, project_root: PathBuf) -> ProjectId {
        let next_project_id = ProjectId(self.project_ids.len() as u8);
        let project_id = *self
            .project_ids
            .entry(project_root.clone())
            .or_insert(next_project_id);
        self.project_roots_input.insert((project_id, project_root));
        project_id
    }

    pub fn _unwatch_project(&mut self, project_id: ProjectId) {
        let opt_project_root =
            self.project_ids.iter().find_map(
                |(root, id)| {
                    if *id == project_id {
                        Some(root)
                    } else {
                        None
                    }
                },
            );
        if let Some(project_root) = opt_project_root {
            self.project_roots_input
                .remove((project_id, project_root.clone()))
        }
    }

    pub fn advance(&mut self) {
        self.current_time += 1;
        let DataflowComputation {
            worker,
            project_roots_input,
            filepath_events_input,
            probes,
            current_time,
            file_event_receiver,
            modules_by_project,
            project_ids: _,
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

        modules_by_project.set_logical_compaction(AntichainRef::new(&[*current_time]));
        modules_by_project.set_physical_compaction(AntichainRef::new(&[*current_time]));

        worker.step_while(|| probes.iter().any(|probe| probe.less_than(current_time)));
    }

    pub fn project_cursor(
        &mut self,
    ) -> ProjectCursor<DataflowTrace<ProjectId, (String, ElmModule)>> {
        let (cursor, storage) = self.modules_by_project.cursor();
        ProjectCursor { cursor, storage }
    }
}

#[allow(clippy::type_complexity)]
pub struct ProjectCursor<T: TraceReader> {
    cursor: T::Cursor,
    storage: <T::Cursor as Cursor<T::Key, T::Val, T::Time, T::R>>::Storage,
}

impl ProjectCursor<DataflowTrace<ProjectId, (String, ElmModule)>> {
    pub fn get_project(&mut self, project: &ProjectId) -> Result<ProjectInfo, Error> {
        let mut modules = HashMap::new();
        self.cursor.rewind_keys(&self.storage);
        self.cursor.rewind_vals(&self.storage);
        self.cursor.seek_key(&self.storage, project);
        while let Some((module_name, module)) = self.cursor.get_val(&self.storage) {
            let mut times = 0;
            self.cursor.map_times(&self.storage, |_, r| times += r);
            if times > 0 {
                modules.insert(module_name.as_str(), module);
            }
            self.cursor.step_val(&self.storage);
        }
        if modules.is_empty() {
            return Err(log::mk_err!("did not find project with id {:?}", project));
        } else {
            Ok(modules)
        }
    }
}

pub type ProjectInfo<'a> = HashMap<&'a str, &'a ElmModule>;

#[derive(Abomonation, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ElmModule {
    pub exports: Vec<ExportedName>,
}

#[derive(Abomonation, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
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

#[derive(Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct ElmJson {
    #[serde(rename = "source-directories")]
    source_directories: Vec<PathBuf>,
}

fn dataflow_graph<'a, W, D>(
    elm_io: D,
    query_for_exports: QueryForExports,
    project_roots: DataflowCollection<'a, (ProjectId, PathBuf)>,
    filepath_events: DataflowCollection<'a, PathBuf>,
    mut file_watcher: W,
) -> (
    DataflowTrace<ProjectId, (String, ElmModule)>,
    Vec<DataflowProbe>,
)
where
    W: notify::Watcher + 'static,
    D: ElmIO + 'static,
{
    let elm_io2 = elm_io.clone();
    let elm_io3 = elm_io.clone();
    let elm_io4 = elm_io.clone();

    let project_roots = project_roots.distinct();

    let elm_json_files = project_roots
        .map(move |(project_id, project_root)| (elm_json_path(&project_root), project_id));

    let elm_json_file_events = elm_json_files
        .semijoin(&filepath_events)
        .concat(&elm_json_files);

    let source_directories_by_project = elm_json_file_events
        .map(|(elm_json_path, project_id)| (project_id, elm_json_path))
        .reduce(move |_, _input, output| {
            let mut elm_json = match elm_io.parse_elm_json(_input[0].0) {
                Ok(elm_json) => elm_json,
                Err(err) => {
                    log::error!("Failed to load elm_json: {:?}", err);
                    return;
                }
            };
            elm_json.source_directories.sort();
            elm_json.source_directories.dedup();
            for dir in elm_json.source_directories.into_iter() {
                output.push((dir, 1));
            }
        });

    let source_directories = source_directories_by_project
        .map(|(_, path)| path)
        .distinct();

    // This collection can intentionally contain files multiple times.
    // A new entry should be added whenever we receive an event for a file,
    // like a modification or removal. Useful for logic that needs to rerun on
    // those occasions.
    let module_events = source_directories
        .flat_map(move |path| {
            elm_io2
                .find_files_recursively(&path)
                .into_iter()
                .filter(|path| is_elm_file(path))
        })
        .concat(&filepath_events.filter(|path| is_elm_file(path)));

    let parsed_modules =
        module_events
            .map(|path| (path, ()))
            .reduce(move |path, _input, output| {
                match elm_io3.parse_elm_module(&query_for_exports, path) {
                    Ok(Some(module)) => output.push((module, 1)),
                    Ok(None) => {}
                    Err(err) => {
                        log::error!("Failed parsing module: {:?}", err);
                    }
                }
            });

    let module_paths = module_events.distinct();

    let project_modules = module_paths
        // Join on `()`, i.e. create a record for every combination of
        // source path and source directory. Then later we can filter
        // that down to keep just the combinations where the path is
        // in the directory.
        .map(|path| ((), path))
        .join(&source_directories_by_project.map(|x| ((), x)))
        .flat_map(|((), (file_path, (project_id, src_dir)))| {
            if file_path.starts_with(&src_dir) {
                Some((file_path, (project_id, src_dir)))
            } else {
                None
            }
        })
        .join_map(
            &parsed_modules,
            |file_path, (project_id, src_dir), parsed_module| {
                match crate::elm::module_name::from_path(src_dir, file_path) {
                    Ok(module_name) => Some((*project_id, (module_name, parsed_module.clone()))),
                    Err(err) => {
                        log::error!("Failed deriving module name: {:?}", err);
                        None
                    }
                }
            },
        )
        .flat_map(|opt| opt);

    let paths_to_watch = source_directories_by_project
        .map(|(_, path)| path)
        .concat(&project_roots.map(|(_, path)| elm_json_path(&path)))
        .concat(&project_roots.map(|(_, path)| idat_path(&path)))
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

    let idat_files =
        project_roots.map(|(project_id, project_root)| (idat_path(&project_root), project_id));

    let idat_file_events = idat_files.semijoin(&filepath_events).concat(&idat_files);

    let idat_modules = idat_file_events
        .map(|(idat_path, project)| (project, idat_path))
        .reduce(move |_project_id, input, output| {
            let idat_path = input[0].0;
            match elm_io4.parse_elm_stuff_idat(idat_path) {
                Ok(modules) => output.extend(modules.into_iter().map(|module| (module, 1))),
                Err(err) => {
                    log::error!("could not read i.dat file: {:?}", err);
                }
            }
        });

    let modules_by_project = project_modules.concat(&idat_modules).arrange_by_key();

    (
        modules_by_project.trace,
        vec![paths_to_watch.probe(), modules_by_project.stream.probe()],
    )
}

fn is_elm_file(path: &Path) -> bool {
    path.extension() == Some(std::ffi::OsStr::new("elm"))
}

fn elm_json_path(project_root: &Path) -> PathBuf {
    project_root.join("elm.json")
}

fn project_root_from_elm_json_path(elm_json: &Path) -> Result<&Path, Error> {
    elm_json.parent().ok_or_else(|| {
        log::mk_err!("couldn't navigate from elm.json file to project root directory")
    })
}

fn idat_path(project_root: &Path) -> PathBuf {
    project_root.join(format!("elm-stuff/{}/i.dat", crate::elm::compiler::VERSION))
}

fn project_root_from_idat_path(idat: &Path) -> Result<&Path, Error> {
    idat.parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .ok_or_else(|| log::mk_err!("couldn't navigate from i.dat file to project root directory"))
}

fn create_elm_stuff(compiler: &Compiler, project_root: &Path) -> Result<(), Error> {
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

fn elm_module_from_interface(dep_i: idat::DependencyInterface) -> Option<ElmModule> {
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

fn elm_export_from_union((idat::Name(name), union): (idat::Name, idat::Union)) -> ExportedName {
    let constructor_names = |canonical_union: idat::CanonicalUnion| {
        let iter = canonical_union
            .alts
            .into_iter()
            .map(|idat::Ctor(idat::Name(name), _, _, _)| name);
        Vec::from_iter(iter)
    };
    let constructors = match union {
        idat::Union::Open(canonical_union) => constructor_names(canonical_union),
        // We're reading this information for use by other modules.
        // These external modules can't see private constructors,
        // so we don't need to return them here.
        idat::Union::Closed(_) => Vec::new(),
        idat::Union::Private(_) => Vec::new(),
    };
    ExportedName::Type { name, constructors }
}

fn elm_export_from_alias((idat::Name(name), _): (idat::Name, idat::Alias)) -> ExportedName {
    ExportedName::Type {
        name,
        constructors: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::rc::Rc;
    use std::sync::Mutex;

    #[derive(Clone)]
    struct FakeElmIO {
        projects: Rc<Mutex<HashMap<PathBuf, FakeElmProject>>>,
        modules: Rc<Mutex<HashMap<PathBuf, ElmModule>>>,
        elm_jsons_parsed: Rc<Mutex<u64>>,
        elm_modules_parsed: Rc<Mutex<u64>>,
        elm_idats_parsed: Rc<Mutex<u64>>,
    }

    #[derive(Clone)]
    struct FakeElmProject {
        elm_json: ElmJson,
        dependencies: Vec<(String, ElmModule)>,
    }

    impl FakeElmIO {
        fn new(
            projects: Vec<(PathBuf, FakeElmProject)>,
            modules: Vec<(PathBuf, ElmModule)>,
        ) -> FakeElmIO {
            FakeElmIO {
                projects: Rc::new(Mutex::new(HashMap::from_iter(projects.into_iter()))),
                modules: Rc::new(Mutex::new(HashMap::from_iter(modules.into_iter()))),
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
            let project_root = project_root_from_elm_json_path(path)?;
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
            _query_for_exports: &QueryForExports,
            path: &Path,
        ) -> Result<Option<ElmModule>, Error> {
            let mut elm_modules_parsed = self.elm_modules_parsed.lock().unwrap();
            let elm_module = self.modules.lock().unwrap().get(path).map(ElmModule::clone);
            if elm_module.is_some() {
                *elm_modules_parsed += 1;
            }
            Ok(elm_module)
        }

        fn parse_elm_stuff_idat(&self, path: &Path) -> Result<Vec<(String, ElmModule)>, Error> {
            let projects = self.projects.lock().unwrap();
            let project_root = project_root_from_idat_path(path)?;
            let project = projects
                .get(project_root)
                .ok_or_else(|| log::mk_err!("did not find project {:?}", project_root))?;
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

    fn mk_project(
        root: &Path,
        src_dirs: Vec<&str>,
        dep_mods: Vec<&str>,
    ) -> (PathBuf, FakeElmProject) {
        (
            root.to_owned(),
            FakeElmProject {
                elm_json: ElmJson {
                    source_directories: src_dirs.into_iter().map(PathBuf::from).collect(),
                },
                dependencies: dep_mods
                    .into_iter()
                    .map(|name| {
                        (
                            name.to_string(),
                            ElmModule {
                                exports: vec![ExportedName::Value {
                                    name: "ants".to_string(),
                                }],
                            },
                        )
                    })
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
        computation: &mut DataflowComputation,
        project: &ProjectId,
        modules: &[&str],
    ) {
        let mut cursor = computation.project_cursor();
        let project = cursor.get_project(project).unwrap();
        assert_eq!(
            project.keys().copied().collect::<HashSet<&str>>(),
            modules.iter().copied().collect::<HashSet<&str>>(),
        );
    }

    #[test]
    fn project_elm_files_are_found() {
        let project_root = PathBuf::from("/project");
        let elm_io = FakeElmIO::new(
            vec![mk_project(&project_root, vec!["/project/src"], vec![])],
            vec![
                mk_module("/project/src/Animals/Bat.elm"),
                mk_module("/project/src/Care/Soap.elm"),
            ],
        );
        let mut computation = DataflowComputation::new_configurable(elm_io).unwrap();
        let project_id = computation.watch_project(project_root);
        computation.advance();
        assert_modules(&mut computation, &project_id, &["Animals.Bat", "Care.Soap"]);
    }

    #[test]
    fn unwatched_projects_are_forgotten() {
        // Given a project with some modules
        let project_root = PathBuf::from("/project");
        let elm_io = FakeElmIO::new(
            vec![mk_project(&project_root, vec!["/project/src"], vec![])],
            vec![
                mk_module("/project/src/Animals/Bat.elm"),
                mk_module("/project/src/Care/Soap.elm"),
            ],
        );
        let mut computation = DataflowComputation::new_configurable(elm_io).unwrap();
        let project_id = computation.watch_project(project_root);
        computation.advance();
        assert_modules(&mut computation, &project_id, &["Animals.Bat", "Care.Soap"]);

        // When we unwatch it
        computation._unwatch_project(project_id);
        computation.advance();

        // Then it is forgotten
        if computation
            .project_cursor()
            .get_project(&project_id)
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
            vec![mk_project(&project_root, vec!["/project/src"], vec![])],
            vec![mk_module("/project/src/Animals/Bat.elm")],
        );
        let mut computation = DataflowComputation::new_configurable(elm_io.clone()).unwrap();
        let project_id = computation.watch_project(project_root);
        computation.advance();
        assert_modules(&mut computation, &project_id, &["Animals.Bat"]);
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
            &mut computation,
            &project_id,
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
                mk_project(&project_root, vec!["/project/src"], vec![]),
                mk_project(&project2_root, vec!["/project2/src"], vec![]),
            ],
            vec![
                mk_module("/project/src/Animals/Bat.elm"),
                mk_module("/project/src/Care/Soap.elm"),
                mk_module("/project2/src/Care/Shampoo.elm"),
            ],
        );
        let mut computation = DataflowComputation::new_configurable(elm_io).unwrap();
        let project_id = computation.watch_project(project_root);
        let project2_id = computation.watch_project(project2_root);
        computation.advance();
        assert_modules(&mut computation, &project_id, &["Animals.Bat", "Care.Soap"]);
        assert_modules(&mut computation, &project2_id, &["Care.Shampoo"]);
    }

    #[test]
    fn elm_files_are_reparsed_if_we_send_an_event_for_them() {
        // Given a project with an existing module...
        let project_root = PathBuf::from("/project");
        let elm_io = FakeElmIO::new(
            vec![mk_project(&project_root, vec!["/project/src"], vec![])],
            vec![mk_module("/project/src/Animals/Bat.elm")],
        );
        let mut computation = DataflowComputation::new_configurable(elm_io.clone()).unwrap();
        let project_id = computation.watch_project(project_root);
        computation.advance();
        assert_modules(&mut computation, &project_id, &["Animals.Bat"]);
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
            vec![mk_project(&project_root, vec!["/project/src"], vec![])],
            vec![
                mk_module("/project/src/Animals/Bat.elm"),
                mk_module("/project/src/Care/Soap.elm"),
            ],
        );
        let mut computation = DataflowComputation::new_configurable(elm_io.clone()).unwrap();
        let project_id = computation.watch_project(project_root);
        computation.advance();
        assert_modules(&mut computation, &project_id, &["Animals.Bat", "Care.Soap"]);
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
        assert_modules(&mut computation, &project_id, &["Care.Soap"]);
        // And no additional parsing has taken place...
        assert_eq!(*elm_io.elm_modules_parsed.lock().unwrap(), 2);
    }

    #[test]
    fn elm_json_files_are_reparsed_if_we_send_an_event_for_them() {
        // Given a project with a module and a dependency...
        let project_root = PathBuf::from("/project");
        let elm_io = FakeElmIO::new(
            vec![mk_project(
                &project_root,
                vec!["/project/src"],
                vec!["Json.Decode"],
            )],
            vec![mk_module("/project/src/Animals/Bat.elm")],
        );
        let mut computation = DataflowComputation::new_configurable(elm_io.clone()).unwrap();
        let project_id = computation.watch_project(project_root.clone());
        computation.advance();
        assert_modules(
            &mut computation,
            &project_id,
            &["Json.Decode", "Animals.Bat"],
        );
        assert_eq!(*elm_io.elm_modules_parsed.lock().unwrap(), 1);
        assert_eq!(*elm_io.elm_jsons_parsed.lock().unwrap(), 1);

        // When we change the elm.json file and remove its source directories...
        elm_io
            .projects
            .lock()
            .unwrap()
            .extend(vec![mk_project(&project_root, vec![], vec![])]);
        computation
            .filepath_events_input
            .insert(PathBuf::from("/project/elm.json"));
        computation.advance();

        // Then the elm.json is reparsed...
        assert_eq!(*elm_io.elm_jsons_parsed.lock().unwrap(), 2);
        // And the project's only contains dependency modules...
        assert_modules(&mut computation, &project_id, &["Json.Decode"]);
        assert_eq!(*elm_io.elm_modules_parsed.lock().unwrap(), 1);
    }

    #[test]
    fn elm_idat_files_are_reparsed_if_we_send_an_event_for_them() {
        // Given a project with a dependency module...
        let project_root = PathBuf::from("/project");
        let elm_io = FakeElmIO::new(
            vec![mk_project(&project_root, vec![], vec!["Json.Decode"])],
            vec![],
        );
        let mut computation = DataflowComputation::new_configurable(elm_io.clone()).unwrap();
        let project_id = computation.watch_project(project_root.clone());
        computation.advance();
        assert_modules(&mut computation, &project_id, &["Json.Decode"]);
        assert_eq!(*elm_io.elm_idats_parsed.lock().unwrap(), 1);

        // When we change the i.dat file to list other dependencies...
        elm_io.projects.lock().unwrap().extend(vec![mk_project(
            &project_root,
            vec![],
            vec!["Time"],
        )]);
        computation
            .filepath_events_input
            .insert(PathBuf::from("/project/elm-stuff/0.19.1/i.dat"));
        computation.advance();

        // Then the i.dat file is reparsed...
        assert_eq!(*elm_io.elm_idats_parsed.lock().unwrap(), 2);
        // And the project's dependency modules have chanaged
        assert_modules(&mut computation, &project_id, &["Time"]);
    }

    #[test]
    fn no_unnecessary_double_work_when_projects_share_a_source_directory() {
        // Given two projects that share a source directory...
        let project_root = PathBuf::from("/project");
        let project2_root = PathBuf::from("/project2");
        let elm_io = FakeElmIO::new(
            vec![
                mk_project(&project_root, vec!["/shared/src"], vec![]),
                mk_project(&project2_root, vec!["/shared/src"], vec![]),
            ],
            vec![mk_module("/shared/src/Care/Soap.elm")],
        );
        let mut computation = DataflowComputation::new_configurable(elm_io.clone()).unwrap();
        let project_id = computation.watch_project(project_root);
        let project2_id = computation.watch_project(project2_root);
        computation.advance();

        // Then both projects list the modules in the shared source directory...
        assert_modules(&mut computation, &project_id, &["Care.Soap"]);
        assert_modules(&mut computation, &project2_id, &["Care.Soap"]);

        // And each module has only been parsed once...
        assert_eq!(*elm_io.elm_modules_parsed.lock().unwrap(), 1);
    }

    #[test]
    fn duplicate_source_directories_dont_cause_extra_parses() {
        // Given an elm.json that lists the same source directory twice...
        let project_root = PathBuf::from("/project");
        let elm_io = FakeElmIO::new(
            vec![mk_project(
                &project_root,
                vec!["/project/src", "/project/src"],
                vec![],
            )],
            vec![
                mk_module("/project/src/Animals/Bat.elm"),
                mk_module("/project/src/Care/Soap.elm"),
            ],
        );
        let mut computation = DataflowComputation::new_configurable(elm_io.clone()).unwrap();

        // When we parse the elm.json...
        let project_id = computation.watch_project(project_root);
        computation.advance();

        // Then the resulting project contains the expected modules...
        assert_modules(&mut computation, &project_id, &["Animals.Bat", "Care.Soap"]);
        // And each module has only been parsed once...
        assert_eq!(*elm_io.elm_modules_parsed.lock().unwrap(), 2);
    }
}
