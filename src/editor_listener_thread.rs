use crate::analysis_thread;
use crate::compilation_thread;
use crate::editors::neovim;
use crate::support::log;
use crate::support::source_code::{Buffer, SourceFileSnapshot};
use crate::{Error, MVar};
use ropey::Rope;
use std::collections::HashMap;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use tree_sitter::InputEdit;

struct EditorListenerLoop {
    active_buffer: Arc<MVar<SourceFileSnapshot>>,
    inactive_buffers: HashMap<Buffer, SourceFileSnapshot>,
    compilation_sender: Sender<compilation_thread::Msg>,
    analysis_sender: Sender<analysis_thread::Msg>,
}

pub(crate) fn run(
    socket_path: PathBuf,
    active_buffer: Arc<MVar<SourceFileSnapshot>>,
    compilation_sender: Sender<compilation_thread::Msg>,
    analysis_sender: Sender<analysis_thread::Msg>,
) -> Result<(), Error> {
    // Delete the socket file in case a previous run left it behind.
    std::fs::remove_file(&socket_path).unwrap_or(());
    let listener = UnixListener::bind(&socket_path).map_err(|err| {
        log::mk_err!("error while creating socket {:?}: {:?}", socket_path, err)
    })?;
    for (editor_id, socket) in listener.incoming().into_iter().enumerate() {
        match socket {
            Err(err) => {
                log::error!("failed to accept editor connection: {:?}", err,);
                continue;
            }
            Ok(accepted_socket) => spawn_editor_thread(
                active_buffer.clone(),
                compilation_sender.clone(),
                analysis_sender.clone(),
                editor_id as u32,
                accepted_socket,
            ),
        };
    }
    Ok(())
}

pub(crate) fn spawn_editor_thread(
    active_buffer: Arc<MVar<SourceFileSnapshot>>,
    compilation_sender: Sender<compilation_thread::Msg>,
    analysis_sender: Sender<analysis_thread::Msg>,
    editor_id: u32,
    socket: UnixStream,
) {
    std::thread::spawn(move || {
        let res = neovim::Neovim::from_unix_socket(socket, editor_id).and_then(
            |neovim| {
                EditorListenerLoop {
                    active_buffer,
                    compilation_sender,
                    analysis_sender,
                    inactive_buffers: HashMap::new(),
                }
                .start(editor_id as u32, neovim)
            },
        );
        match res {
            Ok(()) => {}
            Err(err) => {
                log::error!(
                    "thread for editor {:?} failed with error: {:?}",
                    editor_id,
                    err
                )
            }
        }
    });
}

impl EditorListenerLoop {
    fn start<E: Editor>(
        &mut self,
        editor_id: u32,
        editor: E,
    ) -> Result<(), Error> {
        log::info!(
            "editor {} connected and given id {:?}",
            editor.name(),
            editor_id
        );
        let driver = editor.driver();
        let boxed = Box::new(driver);
        let mut last_compiled_candidates = HashMap::new();
        self.analysis_sender
            .send(analysis_thread::Msg::EditorConnected(editor_id, boxed))?;
        editor.listen(|buffer, update| {
            let event = update.apply_to_buffer(self.take_buffer(buffer))?;
            let code = match event {
                BufferChange::NoChanges => return Ok(()),
                BufferChange::ModifiedBuffer { mut code, edit } => {
                    code.apply_edit(edit)?;
                    code
                }
                BufferChange::OpenedNewBuffer {
                    bytes,
                    path,
                    buffer,
                } => {
                    log::info!("new buffer opened: {:?}", buffer);
                    self.compilation_sender.send(
                        compilation_thread::Msg::OpenedNewSourceFile {
                            buffer,
                            path: path.clone(),
                        },
                    )?;
                    self.analysis_sender.send(
                        analysis_thread::Msg::OpenedNewSourceFile {
                            buffer,
                            path,
                        },
                    )?;
                    SourceFileSnapshot::new(buffer, bytes)?
                }
            };
            if !code.tree.root_node().has_error()
                && Some(&code.revision) > last_compiled_candidates.get(&buffer)
            {
                last_compiled_candidates.insert(buffer, code.revision);
                self.compilation_sender.send(
                    compilation_thread::Msg::CompilationRequested(code.clone()),
                )?;
            }
            self.put_active_buffer(code);
            self.analysis_sender
                .send(analysis_thread::Msg::SourceCodeModified)?;
            Ok(())
        })?;
        self.analysis_sender
            .send(analysis_thread::Msg::EditorDisconnected(editor_id))?;
        log::info!("editor with id {:?} disconnected", editor_id);
        Ok(())
    }

    fn take_buffer(&mut self, buffer: Buffer) -> Option<SourceFileSnapshot> {
        if let Some(code) = self.active_buffer.try_take() {
            if code.buffer == buffer {
                return Some(code);
            } else {
                self.inactive_buffers.insert(code.buffer, code);
            }
        }
        self.inactive_buffers.remove(&buffer)
    }

    fn put_active_buffer(&mut self, code: SourceFileSnapshot) {
        if let Some(prev_active) = self.active_buffer.replace(code) {
            self.inactive_buffers
                .insert(prev_active.buffer, prev_active);
        }
    }
}

// An API for communicatating with an editor.
pub(crate) trait Editor {
    type Driver: analysis_thread::EditorDriver;
    type Event: EditorEvent;

    // Listen for changes to source files happening in the editor.
    fn listen<F>(self, on_event: F) -> Result<(), Error>
    where
        F: FnMut(Buffer, &mut Self::Event) -> Result<(), Error>;

    // Obtain an EditorDriver for sending commands to the editor.
    fn driver(&self) -> Self::Driver;

    fn name(&self) -> &'static str;
}

// A notification of an editor change. To get to the actual change we have to
// pass the existing source code for this file to `apply_to_buffer`. This allows
// the editor integration to copy new source code directly into the existing
// code.
pub(crate) trait EditorEvent {
    fn apply_to_buffer(
        &mut self,
        code: Option<SourceFileSnapshot>,
    ) -> Result<BufferChange, Error>;
}

pub(crate) enum BufferChange {
    NoChanges,
    OpenedNewBuffer {
        buffer: Buffer,
        path: PathBuf,
        bytes: Rope,
    },
    ModifiedBuffer {
        code: SourceFileSnapshot,
        edit: InputEdit,
    },
}
