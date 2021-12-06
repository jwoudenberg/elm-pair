use crate::analysis_thread;
use crate::compilation_thread;
use crate::editors::neovim;
use crate::{Buffer, MVar, SourceFileSnapshot};
use ropey::Rope;
use std::io::BufReader;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::mpsc::{SendError, Sender};
use std::sync::Arc;
use tree_sitter::{InputEdit, Tree};

#[derive(Debug)]
pub(crate) enum Error {
    SocketCreationFailed(std::io::Error),
    AcceptingIncomingSocketConnectionFailed(std::io::Error),
    CloningSocketFailed(std::io::Error),
    EditorRequestedNonExistingLocalCopy,
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
    latest_code_var: Arc<MVar<SourceFileSnapshot>>,
    compilation_sender: Sender<compilation_thread::Msg>,
    analysis_sender: Sender<analysis_thread::Msg>,
}

pub(crate) fn run(
    latest_code_var: Arc<MVar<SourceFileSnapshot>>,
    compilation_sender: Sender<compilation_thread::Msg>,
    analysis_sender: Sender<analysis_thread::Msg>,
) -> Result<(), Error> {
    EditorListenerLoop {
        latest_code_var,
        compilation_sender,
        analysis_sender,
    }
    .start()
}

impl EditorListenerLoop {
    fn start(self) -> Result<(), Error> {
        let socket_path = "/tmp/elm-pair.sock";
        let listener = UnixListener::bind(socket_path)
            .map_err(Error::SocketCreationFailed)?;
        // TODO: Figure out how to deal with multiple connections.
        let editor_id = 0;
        let socket = listener.incoming().into_iter().next().unwrap();
        let read_socket =
            socket.map_err(Error::AcceptingIncomingSocketConnectionFailed)?;
        let write_socket = read_socket
            .try_clone()
            .map_err(Error::CloningSocketFailed)?;
        let neovim = neovim::Neovim::new(
            BufReader::new(read_socket),
            write_socket,
            editor_id,
        );
        self.listen_to_editor(neovim)
    }

    fn listen_to_editor<E>(&self, editor: E) -> Result<(), Error>
    where
        E: Editor,
    {
        let driver = editor.driver();
        let boxed = Box::new(driver);
        let mut revision_of_last_compilation_candidate = None;
        self.analysis_sender
            .send(analysis_thread::Msg::EditorConnected(boxed))?;
        editor.listen(
            |_buf| {
                if let Some(code) = self.latest_code_var.try_take() {
                    Ok(code)
                } else {
                    // TODO: let the editor handle this error (so it can request
                    // a refresh).
                    Err(Error::EditorRequestedNonExistingLocalCopy)
                }
            },
            |event| {
                let code = match event {
                    EditorEvent::ModifiedSourceFile { mut code, edit } => {
                        apply_source_file_edit(&mut code, edit)?;
                        code
                    }
                    EditorEvent::OpenedNewSourceFile {
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
                    && Some(code.revision)
                        > revision_of_last_compilation_candidate
                {
                    revision_of_last_compilation_candidate =
                        Some(code.revision);
                    self.compilation_sender.send(
                        compilation_thread::Msg::CompilationRequested(
                            code.clone(),
                        ),
                    )?;
                }
                self.latest_code_var.write(code);
                self.analysis_sender
                    .send(analysis_thread::Msg::SourceCodeModified)?;
                Ok(())
            },
        )?;
        self.analysis_sender
            .send(analysis_thread::Msg::AllEditorsDisconnected)?;
        Ok(())
    }
}

// An API for communicatating with an editor.
pub(crate) trait Editor {
    type Driver: analysis_thread::EditorDriver;

    // Listen for changes to source files happening in the editor.
    fn listen<F, G>(
        self,
        load_code_copy: F,
        store_new_code: G,
    ) -> Result<(), Error>
    where
        F: FnMut(Buffer) -> Result<SourceFileSnapshot, Error>,
        G: FnMut(EditorEvent) -> Result<(), Error>;

    // Obtain an EditorDriver for sending commands to the editor.
    fn driver(&self) -> Self::Driver;
}

pub(crate) enum EditorEvent {
    OpenedNewSourceFile {
        buffer: Buffer,
        path: PathBuf,
        bytes: Rope,
    },
    ModifiedSourceFile {
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
