use crate::analysis_thread;
use crate::compilation_thread;
use crate::editors::neovim;
use crate::{Buffer, MVar, SourceFileSnapshot};
use ropey::Rope;
use std::collections::HashMap;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc::{SendError, Sender};
use std::sync::Arc;
use tree_sitter::{InputEdit, Tree};

#[derive(Debug)]
pub(crate) enum Error {
    SocketCreationFailed(std::io::Error),
    AcceptingIncomingSocketConnectionFailed(std::io::Error),
    TreeSitterParsingFailed,
    TreeSitterSettingLanguageFailed(tree_sitter::LanguageError),
    NeovimMessageDecodingFailed(neovim::Error),
    FailedToSendMessage,
}

impl<T> From<SendError<T>> for Error {
    fn from(_err: SendError<T>) -> Error {
        Error::FailedToSendMessage
    }
}

struct EditorListenerLoop {
    active_buffer: Arc<MVar<SourceFileSnapshot>>,
    inactive_buffers: HashMap<Buffer, SourceFileSnapshot>,
    compilation_sender: Sender<compilation_thread::Msg>,
    analysis_sender: Sender<analysis_thread::Msg>,
}

pub(crate) fn run(
    active_buffer: Arc<MVar<SourceFileSnapshot>>,
    compilation_sender: Sender<compilation_thread::Msg>,
    analysis_sender: Sender<analysis_thread::Msg>,
) -> Result<(), Error> {
    let socket_path = "/tmp/elm-pair.sock";
    let listener =
        UnixListener::bind(socket_path).map_err(Error::SocketCreationFailed)?;
    for (editor_id, socket) in listener.incoming().into_iter().enumerate() {
        spawn_editor_thread(
            active_buffer.clone(),
            compilation_sender.clone(),
            analysis_sender.clone(),
            editor_id as u32,
            socket.map_err(Error::AcceptingIncomingSocketConnectionFailed)?,
        );
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
    crate::spawn_thread(analysis_sender.clone(), move || {
        let neovim =
            neovim::Neovim::from_unix_socket(socket, editor_id as u32)?;
        EditorListenerLoop {
            active_buffer,
            compilation_sender,
            analysis_sender,
            inactive_buffers: HashMap::new(),
        }
        .start(editor_id as u32, neovim)
    });
}

impl EditorListenerLoop {
    fn start<E: Editor>(
        &mut self,
        editor_id: u32,
        editor: E,
    ) -> Result<(), Error> {
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
                    apply_source_file_edit(&mut code, edit)?;
                    code
                }
                BufferChange::OpenedNewBuffer {
                    bytes,
                    path,
                    buffer,
                } => {
                    self.compilation_sender.send(
                        compilation_thread::Msg::OpenedNewSourceFile {
                            buffer,
                            path,
                        },
                    )?;
                    init_source_file_snapshot(buffer, bytes)?
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

pub(crate) fn init_source_file_snapshot(
    buffer: Buffer,
    bytes: Rope,
) -> Result<SourceFileSnapshot, Error> {
    let snapshot = SourceFileSnapshot {
        buffer,
        tree: parse(None, &bytes)?,
        bytes,
        revision: 0,
    };
    Ok(snapshot)
}

pub(crate) fn apply_source_file_edit(
    code: &mut SourceFileSnapshot,
    edit: InputEdit,
) -> Result<(), Error> {
    code.revision += 1;
    code.tree.edit(&edit);
    reparse_tree(code)?;
    Ok(())
}

fn reparse_tree(code: &mut SourceFileSnapshot) -> Result<(), Error> {
    let new_tree = parse(Some(&code.tree), &code.bytes)?;
    code.tree = new_tree;
    Ok(())
}

// TODO: reuse parser.
fn parse(prev_tree: Option<&Tree>, code: &Rope) -> Result<Tree, Error> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(tree_sitter_elm::language())
        .map_err(Error::TreeSitterSettingLanguageFailed)?;
    match parser.parse(code.bytes().collect::<Vec<u8>>(), prev_tree) {
        None => Err(Error::TreeSitterParsingFailed),
        Some(tree) => Ok(tree),
    }
}
