use crate::elm::compiler::Compiler;
use crate::elm::io::{ElmIO, ExportedName, RealElmIO};
use crate::elm::project;
use crate::lib::dataflow;
use crate::lib::log;
use crate::lib::log::Error;
use crate::lib::source_code::Buffer;
use abomonation_derive::Abomonation;
use differential_dataflow::operators::arrange::ArrangeBySelf;
use differential_dataflow::operators::Join;
use differential_dataflow::operators::Reduce;
use differential_dataflow::operators::Threshold;
use differential_dataflow::trace::TraceReader;
use notify::Watcher;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use timely::dataflow::operators::Probe;

pub struct DataflowComputation {
    // The dataflow worker contains state managed by the differential-dataflow
    // library. Differential-dataflow supports having multiple workers share
    // work, but we don't make use of that.
    worker: dataflow::Worker,
    // These probes let us check whether the dataflow computation has processed
    // all changes made to the inputs below, i.e. whether the outputs will show
    // up-to-date information.
    probes: Vec<dataflow::Probe>,
    // An input representing the buffers we're querying.
    queried_buffers_input: dataflow::Input<Buffer>,
    // An input representing the module names we're querying.
    queried_modules_input: dataflow::Input<String>,
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
}

#[derive(
    Abomonation, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord,
)]
struct ProjectId(u8); // 256 Elm Project should be enough for everyone.

impl DataflowComputation {
    pub fn new(compiler: Compiler) -> Result<DataflowComputation, Error> {
        let mut project_ids = HashMap::new();
        let elm_io = RealElmIO::new(compiler)?;

        let alloc = timely::communication::allocator::thread::Thread::new();
        let mut worker =
            timely::worker::Worker::new(timely::WorkerConfig::default(), alloc);

        let mut queried_buffers_input =
            differential_dataflow::input::InputSession::new();
        let mut queried_modules_input =
            differential_dataflow::input::InputSession::new();
        let mut buffers_input =
            differential_dataflow::input::InputSession::new();
        let mut filepath_events_input =
            differential_dataflow::input::InputSession::new();

        let (file_event_sender, file_event_receiver) = channel();
        let mut file_watcher = notify::watcher(
            file_event_sender,
            core::time::Duration::from_millis(100),
        )
        .map_err(|err| {
            log::mk_err!("failed creating file watcher: {:?}", err)
        })?;

        let (exports_output, probes) = worker.dataflow(|scope| {
            let queried_buffers = queried_buffers_input.to_collection(scope);
            let queried_modules = queried_modules_input.to_collection(scope);
            let buffers = buffers_input.to_collection(scope);
            let filepath_events = filepath_events_input.to_collection(scope);

            let buffer_projects = buffers.flat_map(move |(buffer, path): (Buffer, PathBuf)| {
                match project::root(&path) {
                    Ok(root) => {
                        let next_project_id = ProjectId(project_ids.len() as u8);
                        let project_id = project_ids
                            .entry(root.to_owned())
                            .or_insert(next_project_id);
                        Some((buffer, *project_id, root.to_owned()))
                    }
                    Err(err) => {
                        log::error!("Can't find Elm project root for path {:?}: {:?}", path, err,);
                        None
                    }
                }
            });

            let project_roots = buffer_projects
                .map(|(_, project, root)| (project, root))
                .distinct();

            let (exports, paths_to_watch) = dataflow_graph(elm_io, project_roots, filepath_events);

            let watched_paths =
                paths_to_watch.inspect(move |(path, _, diff)| match std::cmp::Ord::cmp(diff, &0) {
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
                        if let Err(err) = file_watcher.watch(path, notify::RecursiveMode::Recursive)
                        {
                            log::error!(
                                "failed while adding path {:?} to watch for changes: {:?}",
                                path,
                                err
                            )
                        }
                    }
                });

            let queried_projects = buffer_projects
                .map(|(buffer, project, _)| (buffer, project))
                .semijoin(&queried_buffers)
                .map(|(_, project)| project);

            let exports_output = exports
                .semijoin(&queried_projects)
                .map(|(_, x)| x)
                .semijoin(&queried_modules)
                .map(|(_, export)| export)
                .arrange_by_self();

            (
                exports_output.trace,
                vec![watched_paths.probe(), exports_output.stream.probe()],
            )
        });

        let computation = DataflowComputation {
            worker,
            probes,
            queried_buffers_input,
            queried_modules_input,
            buffers_input,
            filepath_events_input,
            file_event_receiver,
            exports_output,
        };

        Ok(computation)
    }

    pub fn track_buffer(&mut self, buffer: Buffer, path: PathBuf) {
        self.buffers_input.insert((buffer, path));
    }

    pub fn advance(&mut self) {
        let DataflowComputation {
            worker,
            queried_buffers_input,
            queried_modules_input,
            buffers_input,
            filepath_events_input,
            probes,
            file_event_receiver,
            exports_output,
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
                probes,
            ),
            worker,
        );
    }

    pub fn exports_cursor(
        &mut self,
        buffer: Buffer,
        module: String,
    ) -> dataflow::Cursor<dataflow::SelfTrace<ExportedName>> {
        self.queried_buffers_input.insert(buffer);
        self.queried_modules_input.insert(module.clone());
        self.advance();
        // Remove the existing query as to not affect future queries.
        // This change will take effect the next time we `advance()`.
        self.queried_buffers_input.remove(buffer);
        self.queried_modules_input.remove(module);

        let (cursor, storage) = self.exports_output.cursor();
        dataflow::Cursor { cursor, storage }
    }
}

#[allow(clippy::type_complexity)]
fn dataflow_graph<'a, D>(
    elm_io: D,
    project_roots: dataflow::Collection<'a, (ProjectId, PathBuf)>,
    filepath_events: dataflow::Collection<'a, PathBuf>,
) -> (
    dataflow::Collection<'a, (ProjectId, (String, ExportedName))>,
    dataflow::Collection<'a, PathBuf>,
)
where
    D: ElmIO + 'static,
{
    let elm_io2 = elm_io.clone();
    let elm_io3 = elm_io.clone();
    let elm_io4 = elm_io.clone();

    let project_roots = project_roots.distinct();

    let elm_json_files =
        project_roots.map(move |(project_id, project_root)| {
            (project::elm_json_path(&project_root), project_id)
        });

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
                .filter(|path| project::is_elm_file(path))
        })
        .concat(&filepath_events.filter(|path| project::is_elm_file(path)));

    let parsed_modules = module_events.map(|path| (path, ())).reduce(
        move |path, _input, output| match elm_io3.parse_elm_module(path) {
            Ok(exports) => {
                output.extend(exports.into_iter().map(|export| (export, 1)))
            }
            Err(err) => {
                log::error!("Failed parsing module: {:?}", err);
            }
        },
    );

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
                    Ok(module_name) => Some((
                        *project_id,
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
        .map(|(_, path)| path)
        .concat(&project_roots.map(|(_, path)| project::elm_json_path(&path)))
        .concat(&project_roots.map(|(_, path)| project::idat_path(&path)))
        .distinct();

    let idat_files = project_roots.map(|(project_id, project_root)| {
        (project::idat_path(&project_root), project_id)
    });

    let idat_file_events =
        idat_files.semijoin(&filepath_events).concat(&idat_files);

    let idat_modules = idat_file_events
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

    (project_modules.concat(&idat_modules), paths_to_watch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::elm::io::mock::{mk_module, mk_project, FakeElmIO};
    use differential_dataflow::operators::arrange::ArrangeByKey;
    use differential_dataflow::trace::cursor::CursorDebug;
    use std::collections::HashSet;
    use std::iter::FromIterator;

    struct DependenciesCalculation {
        worker: dataflow::Worker,
        probes: Vec<dataflow::Probe>,
        project_roots_input: dataflow::Input<(ProjectId, PathBuf)>,
        filepath_events_input: dataflow::Input<PathBuf>,
        exports_by_project:
            dataflow::KeyTrace<ProjectId, (String, ExportedName)>,
        watched_paths: dataflow::SelfTrace<PathBuf>,
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

            let (exports_by_project, watched_paths, probes) =
                worker.dataflow(|scope| {
                    let project_roots =
                        project_roots_input.to_collection(scope);
                    let filepath_events =
                        filepath_events_input.to_collection(scope);
                    let (modules, paths_to_watch) = dataflow_graph(
                        elm_io.clone(),
                        project_roots,
                        filepath_events,
                    );

                    let exports_by_project = modules.arrange_by_key();

                    let watched_paths = paths_to_watch.arrange_by_self();

                    (
                        exports_by_project.trace,
                        watched_paths.trace,
                        vec![
                            watched_paths.stream.probe(),
                            exports_by_project.stream.probe(),
                        ],
                    )
                });

            DependenciesCalculation {
                worker,
                probes,
                project_roots_input,
                filepath_events_input,
                exports_by_project,
                watched_paths,
            }
        }

        fn advance(&mut self) {
            dataflow::Advancable::advance(
                &mut (
                    &mut self.project_roots_input,
                    &mut self.filepath_events_input,
                    &mut self.exports_by_project,
                    &mut self.probes,
                ),
                &mut self.worker,
            );
        }

        fn watched_paths(&mut self) -> HashSet<PathBuf> {
            let (mut cursor, storage) = self.watched_paths.cursor();
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

        fn project(&mut self, project: ProjectId) -> HashSet<String> {
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
                mk_module("/project/src/Care/Soap.elm"),
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
                "Animals.Bat".to_string(),
                "Care.Soap".to_string()
            ]),
        );
        // And the right paths are tracked...
        assert_eq!(
            computation.watched_paths(),
            HashSet::from_iter([
                PathBuf::from("/project/elm.json"),
                PathBuf::from("/project/elm-stuff/0.19.1/i.dat"),
                PathBuf::from("/project/src"),
            ]),
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
                "Animals.Bat".to_string(),
                "Care.Soap".to_string()
            ]),
        );
        // And the right paths are tracked...
        assert_eq!(
            computation.watched_paths(),
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
        assert_eq!(computation.watched_paths(), HashSet::new(),);
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
            HashSet::from_iter(["Animals.Bat".to_string()]),
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
                "Elements.Water".to_string(),
                "Animals.Bat".to_string()
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
                "Animals.Bat".to_string(),
                "Care.Soap".to_string()
            ]),
        );
        assert_eq!(
            computation.project(project2_id),
            HashSet::from_iter(["Care.Shampoo".to_string()]),
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
            HashSet::from_iter(["Animals.Bat".to_string()]),
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
                "Animals.Bat".to_string(),
                "Care.Soap".to_string()
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
            HashSet::from_iter(["Care.Soap".to_string()]),
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
                "Json.Decode".to_string(),
                "Animals.Bat".to_string()
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
            HashSet::from_iter(["Json.Decode".to_string()]),
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
            HashSet::from_iter(["Json.Decode".to_string()]),
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
            HashSet::from_iter(["Time".to_string()]),
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
            HashSet::from_iter(["Care.Soap".to_string()]),
        );
        assert_eq!(
            computation.project(project2_id),
            HashSet::from_iter(["Care.Soap".to_string()]),
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
                "Animals.Bat".to_string(),
                "Care.Soap".to_string()
            ]),
        );
        // And each module has only been parsed once...
        assert_eq!(*elm_io.elm_modules_parsed.lock().unwrap(), 2);
    }
}
