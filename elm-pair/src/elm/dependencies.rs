use crate::elm::compiler::Compiler;
use crate::elm::io::parse_elm_module::Module;
use crate::elm::io::{ElmIO, ExportedName, RealElmIO};
use crate::elm::module_name::ModuleName;
use crate::elm::project;
use crate::lib::dataflow;
use crate::lib::log;
use crate::lib::log::Error;
use crate::lib::source_code::Buffer;
use differential_dataflow::input::Input;
use differential_dataflow::operators::arrange::ArrangeBySelf;
use differential_dataflow::operators::Join;
use differential_dataflow::operators::Reduce;
use differential_dataflow::operators::Threshold;
use differential_dataflow::trace::TraceReader;
use notify::Watcher;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use timely::dataflow::operators::Probe;

pub struct DataflowComputation {
    // The dataflow worker contains state managed by the differential-dataflow
    // library. Differential-dataflow supports having multiple workers share
    // work, but we don't make use of that.
    worker: dataflow::Worker,
    graph: DataflowGraph,
}

struct DataflowGraph {
    // These probes let us check whether the dataflow computation has processed
    // all changes made to the inputs below, i.e. whether the outputs will show
    // up-to-date information.
    probes: Vec<dataflow::Probe>,
    // An input representing the buffers we're querying.
    queried_buffers_input: dataflow::Input<Buffer>,
    // An input representing the module names we're querying.
    queried_modules_input: dataflow::Input<ModuleName>,
    // An input representing projects we're currently tracking.
    buffers_input: dataflow::Input<(Buffer, PathBuf)>,
    // An input representing events happening to files. Whether it's file
    // creation, removal, or modification, we just push a path in here to let
    // it know something's changed.
    filepath_events_input: dataflow::Input<PathBuf>,
    // A channel receiver that will receive events for changes to files in Elm
    // projects being tracked.
    file_event_receiver: Receiver<notify::DebouncedEvent>,
    // A trace containing all exports from modules we're querying for.
    exports_output: dataflow::SelfTrace<ExportedName>,
    // A trace containing all depedents on the buffer we're querying for.
    dependents_output: dataflow::SelfTrace<PathBuf>,
}

#[derive(
    Serialize,
    Deserialize,
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
)]
struct ProjectId(u8); // 256 Elm projects should be enough for everyone.

impl DataflowComputation {
    pub fn new(compiler: Compiler) -> Result<DataflowComputation, Error> {
        let alloc = timely::communication::allocator::thread::Thread::new();
        let mut worker =
            timely::worker::Worker::new(timely::WorkerConfig::default(), alloc);
        let graph = worker.dataflow(|scope| make_graph(scope, compiler))?;
        Ok(DataflowComputation { worker, graph })
    }

    pub fn track_buffer(&mut self, buffer: Buffer, path: PathBuf) {
        let canonical_path = match path.canonicalize() {
            Ok(canonical_path) => canonical_path,
            Err(err) => {
                log::error!(
                    "Failed to canonicalize path {:?}: {:?}",
                    path,
                    err
                );
                path
            }
        };
        self.graph.buffers_input.insert((buffer, canonical_path));
    }

    pub fn advance(&mut self) {
        let DataflowComputation {
            worker,
            graph:
                DataflowGraph {
                    queried_buffers_input,
                    queried_modules_input,
                    buffers_input,
                    filepath_events_input,
                    probes,
                    file_event_receiver,
                    exports_output,
                    dependents_output,
                },
        } = self;
        while let Ok(event) = file_event_receiver.try_recv() {
            let mut push_event = |path: PathBuf| {
                if project::is_elm_file(&path) {
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

        dataflow::Advancable::advance(
            &mut (
                queried_buffers_input,
                queried_modules_input,
                buffers_input,
                filepath_events_input,
                exports_output,
                dependents_output,
                probes,
            ),
            worker,
        );
    }

    // TODO: Make it so exports_cursor does not need to take the module arg.
    pub fn exports_cursor(
        &mut self,
        buffer: Buffer,
        module: ModuleName,
    ) -> dataflow::Cursor<dataflow::SelfTrace<ExportedName>> {
        self.graph.queried_buffers_input.insert(buffer);
        self.graph.queried_modules_input.insert(module.clone());
        self.advance();
        // Remove the existing query as to not affect future queries.
        // This change will take effect the next time we `advance()`.
        self.graph.queried_buffers_input.remove(buffer);
        self.graph.queried_modules_input.remove(module);

        let (cursor, storage) = self.graph.exports_output.cursor();
        dataflow::Cursor { cursor, storage }
    }

    pub fn dependent_modules_cursor(
        &mut self,
        buffer: Buffer,
    ) -> dataflow::Cursor<dataflow::SelfTrace<PathBuf>> {
        self.graph.queried_buffers_input.insert(buffer);
        self.advance();
        // Remove the existing query as to not affect future queries.
        // This change will take effect the next time we `advance()`.
        self.graph.queried_buffers_input.remove(buffer);
        let (cursor, storage) = self.graph.dependents_output.cursor();
        dataflow::Cursor { cursor, storage }
    }
}

// TODO: clarify difference between this function and dataflow_graph.
fn make_graph(
    scope: &mut dataflow::Scope,
    compiler: Compiler,
) -> Result<DataflowGraph, Error> {
    let mut project_ids = HashMap::new();
    let elm_io = RealElmIO::new(compiler)?;
    let (file_event_sender, file_event_receiver) = channel();
    let mut file_watcher = notify::watcher(
        file_event_sender,
        core::time::Duration::from_millis(100),
    )
    .map_err(|err| log::mk_err!("failed creating file watcher: {:?}", err))?;

    let (queried_buffers_input, queried_buffers) = scope.new_collection();
    let (queried_modules_input, queried_modules) = scope.new_collection();
    let (buffers_input, buffers) = scope.new_collection();
    let (filepath_events_input, filepath_events) = scope.new_collection();

    let buffer_projects =
        buffers.flat_map(move |(buffer, path): (Buffer, PathBuf)| {
            match project::root(&path) {
                Ok(root) => {
                    let next_project_id = ProjectId(project_ids.len() as u8);
                    let project_id = project_ids
                        .entry(root.to_owned())
                        .or_insert(next_project_id);
                    Some((buffer, *project_id, root.to_owned()))
                }
                Err(err) => {
                    log::error!(
                        "Can't find Elm project root for path {:?}: {:?}",
                        path,
                        err,
                    );
                    None
                }
            }
        });

    let project_roots = buffer_projects
        .map(|(_, project, root)| (project, root))
        .distinct();

    let (exports_by_project, paths_to_watch, dependent_modules) =
        dataflow_graph(elm_io, project_roots, filepath_events);

    let watched_paths =
        paths_to_watch.inspect(
            move |(path, _, diff)| match std::cmp::Ord::cmp(diff, &0) {
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
                    if let Err(err) = file_watcher
                        .watch(path, notify::RecursiveMode::Recursive)
                    {
                        log::error!(
                "failed while adding path {:?} to watch for changes: {:?}",
                                path,
                                err
                            )
                    }
                }
            },
        );

    let queried_projects = buffer_projects
        .map(|(buffer, project, _)| (buffer, project))
        .semijoin(&queried_buffers)
        .map(|(_, project)| project);

    let exports_output = exports_by_project
        .semijoin(&queried_projects)
        .map(|(_, x)| x)
        .semijoin(&queried_modules)
        .map(|(_, export)| export)
        .arrange_by_self();

    let queried_paths: dataflow::Collection<(ProjectId, PathBuf)> =
        buffer_projects
            .map(|(buffer, project, _)| (buffer, project))
            .semijoin(&queried_buffers)
            .join_map(&buffers, |_buffer, project, path| {
                (*project, path.clone())
            });

    let dependents_output: dataflow::Collection<PathBuf> = dependent_modules
        .semijoin(&queried_paths)
        .map(|(_, path)| path);

    let dependents_output_arr = dependents_output.arrange_by_self();

    let probes = vec![
        watched_paths.probe(),
        exports_output.stream.probe(),
        dependents_output_arr.stream.probe(),
    ];
    let graph = DataflowGraph {
        probes,
        queried_buffers_input,
        queried_modules_input,
        buffers_input,
        filepath_events_input,
        file_event_receiver,
        exports_output: exports_output.trace,
        dependents_output: dependents_output_arr.trace,
    };
    Ok(graph)
}

#[allow(clippy::type_complexity)]
fn dataflow_graph<'a, D>(
    elm_io: D,
    project_roots: dataflow::Collection<'a, (ProjectId, PathBuf)>,
    filepath_events: dataflow::Collection<'a, PathBuf>,
) -> (
    dataflow::Collection<'a, (ProjectId, (ModuleName, ExportedName))>,
    dataflow::Collection<'a, PathBuf>,
    dataflow::Collection<'a, ((ProjectId, PathBuf), PathBuf)>,
)
where
    D: ElmIO + 'static,
{
    let elm_io2 = elm_io.clone();
    let elm_io3 = elm_io.clone();
    let elm_io4 = elm_io.clone();

    let elm_json_files: dataflow::Collection<(PathBuf, ProjectId)> =
        project_roots.map(move |(project_id, project_root)| {
            (project::elm_json_path(&project_root), project_id)
        });

    let elm_json_file_events: dataflow::Collection<(PathBuf, ProjectId)> =
        elm_json_files
            .semijoin(&filepath_events)
            .concat(&elm_json_files);

    let source_directories_by_project: dataflow::Collection<(
        ProjectId,
        PathBuf,
    )> = elm_json_file_events
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

    let source_directories: dataflow::Collection<PathBuf> =
        source_directories_by_project
            .map(|(_, path)| path)
            .distinct();

    // This collection can intentionally contain files multiple times.
    // A new entry should be added whenever we receive an event for a file,
    // like a modification or removal. Useful for logic that needs to rerun on
    // those occasions.
    let module_events: dataflow::Collection<PathBuf> = source_directories
        .flat_map(move |path| {
            elm_io2
                .find_files_recursively(&path)
                .into_iter()
                .filter(|path| project::is_elm_file(path))
        })
        .concat(&filepath_events.filter(|path| project::is_elm_file(path)));

    let parsed_modules: dataflow::Collection<(PathBuf, Module)> =
        module_events.map(|path| (path, ())).reduce(
            move |path, _input, output| match elm_io3.parse_elm_module(path) {
                Ok(parsed) => output.push((parsed, 1)),
                Err(err) => {
                    log::error!("Failed parsing module: {:?}", err);
                }
            },
        );

    let exported_names: dataflow::Collection<(PathBuf, ExportedName)> =
        parsed_modules.flat_map(|(path, (exports, _))| {
            exports
                .into_iter()
                .map(move |export| (path.clone(), export))
        });

    let modules_dependent_on_path: dataflow::Collection<(ModuleName, PathBuf)> =
        parsed_modules.flat_map(|(path, (_, imports))| {
            imports
                .into_iter()
                .map(move |import| (import, path.clone()))
        });

    let module_paths: dataflow::Collection<PathBuf> = module_events.distinct();

    let project_modules: dataflow::Collection<(
        PathBuf,
        (ProjectId, ModuleName),
    )> = module_paths
        // Join on `()`, i.e. create a record for every combination of
        // source path and source directory. Then later we can filter
        // that down to keep just the combinations where the path is
        // in the directory.
        .map(|path| ((), path))
        .join(&source_directories_by_project.map(|x| ((), x)))
        .flat_map(|((), (file_path, (project_id, src_dir)))| {
            if file_path.starts_with(&src_dir) {
                match crate::elm::module_name::from_path(&src_dir, &file_path) {
                    Ok(module_name) => {
                        Some((file_path, (project_id, module_name)))
                    }
                    Err(err) => {
                        log::error!("Failed deriving module name: {:?}", err);
                        None
                    }
                }
            } else {
                None
            }
        });

    let exports_by_project: dataflow::Collection<(
        ProjectId,
        (ModuleName, ExportedName),
    )> = project_modules.join_map(
        &exported_names,
        |_file_path, (project_id, module_name), parsed_module| {
            (*project_id, (module_name.clone(), parsed_module.clone()))
        },
    );

    let paths_to_watch: dataflow::Collection<PathBuf> =
        source_directories_by_project
            .map(|(_, path)| path)
            .concat(
                &project_roots.map(|(_, path)| project::elm_json_path(&path)),
            )
            .concat(&project_roots.map(|(_, path)| project::idat_path(&path)))
            .distinct();

    let idat_files: dataflow::Collection<(PathBuf, ProjectId)> = project_roots
        .map(|(project_id, project_root)| {
            (project::idat_path(&project_root), project_id)
        });

    let idat_file_events: dataflow::Collection<(PathBuf, ProjectId)> =
        idat_files.semijoin(&filepath_events).concat(&idat_files);

    let idat_modules: dataflow::Collection<(
        ProjectId,
        (ModuleName, ExportedName),
    )> = idat_file_events
        .map(|(idat_path, project)| (project, idat_path))
        .reduce(move |_project_id, input, output| {
            let idat_path = input[0].0;
            match elm_io4.parse_elm_stuff_idat(idat_path) {
                Ok(modules) => output.extend(modules.map(|module| (module, 1))),
                Err(err) => {
                    log::error!("could not read i.dat file: {:?}", err);
                }
            }
        });

    let dependent_modules: dataflow::Collection<(
        (ProjectId, PathBuf),
        PathBuf,
    )> = project_modules
        .map(|(imported_path, (project_id, imported_name))| {
            (imported_name, (project_id, imported_path))
        })
        .join_map(
            &modules_dependent_on_path,
            |_imported_name, (project_id, imported_path), dependent_path| {
                ((*project_id, imported_path.clone()), dependent_path.clone())
            },
        );

    (
        exports_by_project.concat(&idat_modules),
        paths_to_watch,
        dependent_modules,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::elm::io::mock::{
        mk_module, mk_module_with_imports, mk_project, FakeElmIO,
    };
    use differential_dataflow::operators::arrange::ArrangeByKey;
    use differential_dataflow::trace::cursor::CursorDebug;
    use std::collections::HashSet;
    use std::iter::FromIterator;
    use std::path::Path;

    struct DependenciesCalculation {
        worker: dataflow::Worker,
        probes: Vec<dataflow::Probe>,
        project_roots_input: dataflow::Input<(ProjectId, PathBuf)>,
        filepath_events_input: dataflow::Input<PathBuf>,
        exports_by_project:
            dataflow::KeyTrace<ProjectId, (ModuleName, ExportedName)>,
        paths_to_watch: dataflow::SelfTrace<PathBuf>,
        dependent_modules: dataflow::KeyTrace<(ProjectId, PathBuf), PathBuf>,
    }

    impl DependenciesCalculation {
        fn new(elm_io: &FakeElmIO) -> DependenciesCalculation {
            let alloc = timely::communication::allocator::thread::Thread::new();
            let mut worker = timely::worker::Worker::new(
                timely::WorkerConfig::default(),
                alloc,
            );

            let mut project_roots_input =
                differential_dataflow::input::InputSession::new();
            let mut filepath_events_input =
                differential_dataflow::input::InputSession::new();

            let (exports_by_project, paths_to_watch, dependent_modules, probes) =
                worker.dataflow(|scope| {
                    let project_roots =
                        project_roots_input.to_collection(scope);
                    let filepath_events =
                        filepath_events_input.to_collection(scope);
                    let (exports_by_project, paths_to_watch, dependent_modules) =
                        dataflow_graph(
                            elm_io.clone(),
                            project_roots,
                            filepath_events,
                        );

                    let exports_by_project_arr =
                        exports_by_project.arrange_by_key();

                    let paths_to_watch_arr = paths_to_watch.arrange_by_self();

                    let imports_by_module_arr = dependent_modules.arrange_by_key();

                    (
                        exports_by_project_arr.trace,
                        paths_to_watch_arr.trace,
                        imports_by_module_arr.trace,
                        vec![
                            paths_to_watch_arr.stream.probe(),
                            exports_by_project_arr.stream.probe(),
                        ],
                    )
                });

            DependenciesCalculation {
                worker,
                probes,
                project_roots_input,
                filepath_events_input,
                exports_by_project,
                paths_to_watch,
                dependent_modules,
            }
        }

        fn advance(&mut self) {
            dataflow::Advancable::advance(
                &mut (
                    &mut self.project_roots_input,
                    &mut self.filepath_events_input,
                    &mut self.exports_by_project,
                    &mut self.dependent_modules,
                    &mut self.probes,
                ),
                &mut self.worker,
            );
        }

        fn paths_to_watch(&mut self) -> HashSet<PathBuf> {
            let (mut cursor, storage) = self.paths_to_watch.cursor();
            cursor
                .to_vec(&storage)
                .into_iter()
                .filter_map(|((path, _), counts)| {
                    let total: isize =
                        counts.into_iter().map(|(_, count)| count).sum();
                    if total > 0 {
                        Some(path)
                    } else {
                        None
                    }
                })
                .collect()
        }

        fn project(&mut self, project: ProjectId) -> HashSet<ModuleName> {
            let (mut cursor, storage) = self.exports_by_project.cursor();
            cursor
                .to_vec(&storage)
                .into_iter()
                .filter_map(|((project_, (name, _contents)), counts)| {
                    let total: isize =
                        counts.into_iter().map(|(_, count)| count).sum();
                    if total > 0 && project_ == project {
                        Some(name)
                    } else {
                        None
                    }
                })
                .collect()
        }

        fn dependent_modules(
            &mut self,
            project: ProjectId,
            module: &Path,
        ) -> HashSet<PathBuf> {
            let (mut cursor, storage) = self.dependent_modules.cursor();
            cursor
                .to_vec(&storage)
                .into_iter()
                .filter_map(
                    |(((project_, module_), imported_module), counts)| {
                        let total: isize =
                            counts.into_iter().map(|(_, count)| count).sum();
                        if total > 0 && project_ == project && module_ == module
                        {
                            Some(imported_module)
                        } else {
                            None
                        }
                    },
                )
                .collect()
        }
    }

    #[test]
    fn project_elm_files_are_found() {
        // Given an Elm project with some files...
        let project_id = ProjectId(0);
        let project_root = PathBuf::from("/project");
        let elm_io = FakeElmIO::new(
            vec![mk_project(&project_root, vec!["/project/src"], vec![])],
            vec![
                mk_module("/project/src/Animals/Bat.elm"),
                mk_module_with_imports(
                    "/project/src/Care/Soap.elm",
                    vec![ModuleName::from_str("Animals.Bat")],
                ),
            ],
        );
        let mut computation = DependenciesCalculation::new(&elm_io);

        // When we start tracking the project...
        computation
            .project_roots_input
            .insert((project_id, project_root));
        computation.advance();

        // Then all its modules are discovered...
        assert_eq!(
            computation.project(project_id),
            HashSet::from_iter([
                ModuleName::from_str("Animals.Bat"),
                ModuleName::from_str("Care.Soap")
            ]),
        );
        // And the right paths are tracked...
        assert_eq!(
            computation.paths_to_watch(),
            HashSet::from_iter([
                PathBuf::from("/project/elm.json"),
                PathBuf::from("/project/elm-stuff/0.19.1/i.dat"),
                PathBuf::from("/project/src"),
            ]),
        );
        // And imports are found...
        assert_eq!(
            computation.dependent_modules(
                project_id,
                Path::new("/project/src/Animals/Bat.elm"),
            ),
            HashSet::from_iter([PathBuf::from("/project/src/Care/Soap.elm")]),
        );
    }

    #[test]
    fn unwatched_projects_are_forgotten() {
        let project_id = ProjectId(0);
        // Given a project with some modules
        let project_root = PathBuf::from("/project");
        let elm_io = FakeElmIO::new(
            vec![mk_project(&project_root, vec!["/project/src"], vec![])],
            vec![
                mk_module("/project/src/Animals/Bat.elm"),
                mk_module("/project/src/Care/Soap.elm"),
            ],
        );
        let mut computation = DependenciesCalculation::new(&elm_io);
        computation
            .project_roots_input
            .insert((project_id, project_root.clone()));
        computation.advance();
        assert_eq!(
            computation.project(project_id),
            HashSet::from_iter([
                ModuleName::from_str("Animals.Bat"),
                ModuleName::from_str("Care.Soap")
            ]),
        );
        // And the right paths are tracked...
        assert_eq!(
            computation.paths_to_watch(),
            HashSet::from_iter([
                PathBuf::from("/project/elm.json"),
                PathBuf::from("/project/elm-stuff/0.19.1/i.dat"),
                PathBuf::from("/project/src"),
            ]),
        );

        // When we unwatch it
        computation
            .project_roots_input
            .remove((project_id, project_root));
        computation.advance();

        // Then it is forgotten
        assert_eq!(computation.project(project_id), HashSet::new(),);
        // And the projects paths are no longer tracked...
        assert_eq!(computation.paths_to_watch(), HashSet::new(),);
    }

    #[test]
    fn elm_files_created_after_initial_parse_are_found() {
        // Given a project with an existing module...
        let project_id = ProjectId(0);
        let project_root = PathBuf::from("/project");
        let elm_io = FakeElmIO::new(
            vec![mk_project(&project_root, vec!["/project/src"], vec![])],
            vec![mk_module("/project/src/Animals/Bat.elm")],
        );
        let mut computation = DependenciesCalculation::new(&elm_io);
        computation
            .project_roots_input
            .insert((project_id, project_root));
        computation.advance();
        assert_eq!(
            computation.project(project_id),
            HashSet::from_iter([ModuleName::from_str("Animals.Bat")]),
        );
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
        assert_eq!(
            computation.project(project_id),
            HashSet::from_iter([
                ModuleName::from_str("Elements.Water"),
                ModuleName::from_str("Animals.Bat")
            ]),
        );
        // And we only parsed the new module...
        assert_eq!(*elm_io.elm_modules_parsed.lock().unwrap(), 2);
    }

    #[test]
    fn projects_can_have_separate_files() {
        let project_id = ProjectId(0);
        let project2_id = ProjectId(1);
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
        let mut computation = DependenciesCalculation::new(&elm_io);
        computation
            .project_roots_input
            .insert((project_id, project_root));
        computation
            .project_roots_input
            .insert((project2_id, project2_root));
        computation.advance();
        assert_eq!(
            computation.project(project_id),
            HashSet::from_iter([
                ModuleName::from_str("Animals.Bat"),
                ModuleName::from_str("Care.Soap")
            ]),
        );
        assert_eq!(
            computation.project(project2_id),
            HashSet::from_iter([ModuleName::from_str("Care.Shampoo")]),
        );
    }

    #[test]
    fn elm_files_are_reparsed_if_we_send_an_event_for_them() {
        // Given a project with an existing module...
        let project_id = ProjectId(0);
        let project_root = PathBuf::from("/project");
        let elm_io = FakeElmIO::new(
            vec![mk_project(&project_root, vec!["/project/src"], vec![])],
            vec![mk_module("/project/src/Animals/Bat.elm")],
        );
        let mut computation = DependenciesCalculation::new(&elm_io);
        computation
            .project_roots_input
            .insert((project_id, project_root));
        computation.advance();
        assert_eq!(
            computation.project(project_id),
            HashSet::from_iter([ModuleName::from_str("Animals.Bat")]),
        );
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
        let project_id = ProjectId(0);
        let project_root = PathBuf::from("/project");
        let elm_io = FakeElmIO::new(
            vec![mk_project(&project_root, vec!["/project/src"], vec![])],
            vec![
                mk_module("/project/src/Animals/Bat.elm"),
                mk_module("/project/src/Care/Soap.elm"),
            ],
        );
        let mut computation = DependenciesCalculation::new(&elm_io);
        computation
            .project_roots_input
            .insert((project_id, project_root));
        computation.advance();
        assert_eq!(
            computation.project(project_id),
            HashSet::from_iter([
                ModuleName::from_str("Animals.Bat"),
                ModuleName::from_str("Care.Soap")
            ]),
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
        assert_eq!(
            computation.project(project_id),
            HashSet::from_iter([ModuleName::from_str("Care.Soap")]),
        );
        // And no additional parsing has taken place...
        assert_eq!(*elm_io.elm_modules_parsed.lock().unwrap(), 2);
    }

    #[test]
    fn elm_json_files_are_reparsed_if_we_send_an_event_for_them() {
        let project_id = ProjectId(0);
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
        let mut computation = DependenciesCalculation::new(&elm_io);
        computation
            .project_roots_input
            .insert((project_id, project_root.clone()));
        computation.advance();
        assert_eq!(
            computation.project(project_id),
            HashSet::from_iter([
                ModuleName::from_str("Json.Decode"),
                ModuleName::from_str("Animals.Bat")
            ]),
        );
        assert_eq!(*elm_io.elm_modules_parsed.lock().unwrap(), 1);
        assert_eq!(*elm_io.elm_jsons_parsed.lock().unwrap(), 1);

        // When we change the elm.json file and remove its source directories...
        elm_io.projects.lock().unwrap().extend(vec![mk_project(
            &project_root,
            vec![],
            vec![],
        )]);
        computation
            .filepath_events_input
            .insert(PathBuf::from("/project/elm.json"));
        computation.advance();

        // Then the elm.json is reparsed...
        assert_eq!(*elm_io.elm_jsons_parsed.lock().unwrap(), 2);
        // And the project's only contains dependency modules...
        assert_eq!(
            computation.project(project_id),
            HashSet::from_iter([ModuleName::from_str("Json.Decode")]),
        );
        assert_eq!(*elm_io.elm_modules_parsed.lock().unwrap(), 1);
    }

    #[test]
    fn elm_idat_files_are_reparsed_if_we_send_an_event_for_them() {
        let project_id = ProjectId(0);
        // Given a project with a dependency module...
        let project_root = PathBuf::from("/project");
        let elm_io = FakeElmIO::new(
            vec![mk_project(&project_root, vec![], vec!["Json.Decode"])],
            vec![],
        );
        let mut computation = DependenciesCalculation::new(&elm_io);
        computation
            .project_roots_input
            .insert((project_id, project_root.clone()));
        computation.advance();
        assert_eq!(
            computation.project(project_id),
            HashSet::from_iter([ModuleName::from_str("Json.Decode")]),
        );
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
        assert_eq!(
            computation.project(project_id),
            HashSet::from_iter([ModuleName::from_str("Time")]),
        );
    }

    #[test]
    fn no_unnecessary_double_work_when_projects_share_a_source_directory() {
        // Given two projects that share a source directory...
        let project_id = ProjectId(0);
        let project2_id = ProjectId(1);
        let project_root = PathBuf::from("/project");
        let project2_root = PathBuf::from("/project2");
        let elm_io = FakeElmIO::new(
            vec![
                mk_project(&project_root, vec!["/shared/src"], vec![]),
                mk_project(&project2_root, vec!["/shared/src"], vec![]),
            ],
            vec![mk_module("/shared/src/Care/Soap.elm")],
        );
        let mut computation = DependenciesCalculation::new(&elm_io);
        computation
            .project_roots_input
            .insert((project_id, project_root));
        computation
            .project_roots_input
            .insert((project2_id, project2_root));
        computation.advance();

        // Then both projects list the modules in the shared source directory...
        assert_eq!(
            computation.project(project_id),
            HashSet::from_iter([ModuleName::from_str("Care.Soap")]),
        );
        assert_eq!(
            computation.project(project2_id),
            HashSet::from_iter([ModuleName::from_str("Care.Soap")]),
        );

        // And each module has only been parsed once...
        assert_eq!(*elm_io.elm_modules_parsed.lock().unwrap(), 1);
    }

    #[test]
    fn duplicate_source_directories_dont_cause_extra_parses() {
        // Given an elm.json that lists the same source directory twice...
        let project_id = ProjectId(0);
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
        let mut computation = DependenciesCalculation::new(&elm_io);

        // When we parse the elm.json...
        computation
            .project_roots_input
            .insert((project_id, project_root));
        computation.advance();

        // Then the resulting project contains the expected modules...
        assert_eq!(
            computation.project(project_id),
            HashSet::from_iter([
                ModuleName::from_str("Animals.Bat"),
                ModuleName::from_str("Care.Soap")
            ]),
        );
        // And each module has only been parsed once...
        assert_eq!(*elm_io.elm_modules_parsed.lock().unwrap(), 2);
    }
}
