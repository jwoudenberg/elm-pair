use crate::analysis_thread;
use crate::compilation_thread;
use crate::editors::neovim;
use crate::editors::vscode;
use crate::lib::log;
use crate::lib::source_code::{EditorId, RefactorAllowed, SourceFileSnapshot};
use crate::Error;
use std::io::Read;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc::Sender;

struct EditorListenerLoop {
    compilation_sender: Sender<compilation_thread::Msg>,
    analysis_sender: Sender<analysis_thread::Msg>,
}

pub fn run(
    listener: UnixListener,
    compilation_sender: Sender<compilation_thread::Msg>,
    analysis_sender: Sender<analysis_thread::Msg>,
) -> Result<(), Error> {
    for (editor_id, socket) in listener.incoming().into_iter().enumerate() {
        match socket {
            Err(err) => {
                log::error!("failed to accept editor connection: {:?}", err,);
                continue;
            }
            Ok(accepted_socket) => spawn_editor_thread(
                compilation_sender.clone(),
                analysis_sender.clone(),
                EditorId::new(editor_id as u32),
                accepted_socket,
            ),
        };
    }
    Ok(())
}

#[derive(Debug)]
pub enum EditorKind {
    Neovim,
    VsCode,
}

fn read_editor_kind<R: Read>(read: &mut R) -> Result<EditorKind, Error> {
    // We use 4 bytes to identify the editor because this is the smallest
    // payload size Neovim is able to send, being limited to messages that are
    // valid msgpack-rpc payloads.
    let mut buf = [0; 4];
    read.read_exact(&mut buf)
        .map_err(|err| log::mk_err!("could not read editor kind: {:?}", err))?;
    match buf {
        // The 4-byte identifier for Neovim is an empty msgpack-rpc notify msg.
        // 147 (10010011): Marks an upcoming 3-element array.
        // 2: Fist element is the notify msg kind, which is always 2.
        // 160 (10100000): An empty string (notify method being called).
        // 144 (10010000): Empty array (arguments passed to notify method).
        [147, 2, 160, 144] => Ok(EditorKind::Neovim),
        [0, 0, 0, 0] => Ok(EditorKind::VsCode),
        other => Err(log::mk_err!("unknown editor identifier {:?}", other)),
    }
}

pub fn spawn_editor_thread(
    compilation_sender: Sender<compilation_thread::Msg>,
    analysis_sender: Sender<analysis_thread::Msg>,
    editor_id: EditorId,
    mut socket: UnixStream,
) {
    let editor_kind = match read_editor_kind(&mut socket) {
        Ok(kind) => kind,
        Err(err) => {
            log::error!("Failed to start editor thread: {:?}", err);
            return;
        }
    };
    std::thread::spawn(move || {
        let mut listener_loop = EditorListenerLoop {
            compilation_sender,
            analysis_sender,
        };
        let res = match editor_kind {
            EditorKind::Neovim => {
                neovim::Neovim::from_unix_socket(socket, editor_id)
                    .and_then(|editor| listener_loop.start(editor_id, editor))
            }
            EditorKind::VsCode => {
                vscode::VsCode::from_unix_socket(socket, editor_id)
                    .and_then(|editor| listener_loop.start(editor_id, editor))
            }
        };
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
        editor_id: EditorId,
        editor: E,
    ) -> Result<(), Error> {
        log::info!(
            "editor {} connected and given id {:?}",
            editor.name(),
            editor_id
        );
        let driver = editor.driver();
        let boxed = Box::new(driver);
        self.analysis_sender
            .send(analysis_thread::Msg::EditorConnected(editor_id, boxed))?;
        editor.listen(|event| {
            let new_code = match event {
                EditorEvent::ModifiedBuffer {
                    code,
                    refactor_allowed,
                } => {
                    self.analysis_sender.send(
                        analysis_thread::Msg::SourceCodeModified {
                            code: code.clone(),
                            refactor: refactor_allowed,
                        },
                    )?;
                    code
                }
                EditorEvent::OpenedNewBuffer { code, path } => {
                    log::info!("new buffer opened: {:?}", code.buffer);
                    self.compilation_sender.send(
                        compilation_thread::Msg::OpenedNewSourceFile {
                            buffer: code.buffer,
                            path: path.clone(),
                        },
                    )?;
                    self.analysis_sender.send(
                        analysis_thread::Msg::OpenedNewSourceFile {
                            path,
                            code: code.clone(),
                        },
                    )?;
                    code
                }
            };
            if !new_code.tree.root_node().has_error() {
                self.compilation_sender.send(
                    compilation_thread::Msg::CompilationRequested(new_code),
                )?;
            }
            Ok(())
        })?;
        self.analysis_sender
            .send(analysis_thread::Msg::EditorDisconnected(editor_id))?;
        log::info!("editor with id {:?} disconnected", editor_id);
        Ok(())
    }
}

// An API for communicatating with an editor.
pub trait Editor {
    type Driver: analysis_thread::EditorDriver;

    // Listen for changes to source files happening in the editor.
    fn listen<F>(self, on_event: F) -> Result<(), Error>
    where
        F: FnMut(EditorEvent) -> Result<(), Error>;

    // Obtain an EditorDriver for sending commands to the editor.
    fn driver(&self) -> Self::Driver;

    fn name(&self) -> &'static str;
}

pub enum EditorEvent {
    OpenedNewBuffer {
        code: SourceFileSnapshot,
        path: PathBuf,
    },
    ModifiedBuffer {
        code: SourceFileSnapshot,
        refactor_allowed: RefactorAllowed,
    },
}
