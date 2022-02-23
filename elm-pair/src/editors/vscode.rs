use crate::analysis_thread as analysis;
use crate::editor_listener_thread::{BufferChange, Editor, EditorEvent};
use crate::lib::bytes;
use crate::lib::log;
use crate::lib::log::Error;
use crate::lib::source_code::{Buffer, Edit, SourceFileSnapshot};
use std::collections::HashMap;
use std::io::{BufReader, BufWriter, Read, Write};
use std::ops::DerefMut;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tree_sitter::{InputEdit, Point};

const NEW_FILE_MSG: u8 = 0;
const FILE_CHANGED_MSG: u8 = 1;

pub struct VsCode<R, W> {
    editor_id: u32,
    read: R,
    write: Arc<Mutex<W>>,
    buffer_paths: Arc<Mutex<HashMap<Buffer, PathBuf>>>,
}

impl VsCode<BufReader<UnixStream>, BufWriter<UnixStream>> {
    pub fn from_unix_socket(
        socket: UnixStream,
        editor_id: u32,
    ) -> Result<Self, crate::Error> {
        let write = socket.try_clone().map_err(|err| {
            log::mk_err!("failed cloning vscode socket: {:?}", err)
        })?;
        let vscode = VsCode {
            editor_id,
            read: BufReader::new(socket),
            write: Arc::new(Mutex::new(BufWriter::new(write))),
            buffer_paths: Arc::new(Mutex::new(HashMap::new())),
        };
        Ok(vscode)
    }
}

impl<R: Read, W: 'static + Write + Send> Editor for VsCode<R, W> {
    type Driver = VsCodeDriver<W>;
    type Event = VsCodeEvent<R>;

    fn driver(&self) -> VsCodeDriver<W> {
        VsCodeDriver {
            write: self.write.clone(),
            buffer_paths: self.buffer_paths.clone(),
        }
    }

    fn name(&self) -> &'static str {
        "vs-code"
    }

    fn listen<F>(self, mut on_event: F) -> Result<(), crate::Error>
    where
        F: FnMut(Buffer, &mut Self::Event) -> Result<(), crate::Error>,
    {
        let mut read = self.read;
        loop {
            let mut u32_buffer = [0; 4];
            let buffer_id = match read.read_exact(&mut u32_buffer) {
                Ok(()) => std::primitive::u32::from_be_bytes(u32_buffer),
                Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Ok(());
                }
                Err(err) => {
                    return Err(log::mk_err!("could not read u8: {:?}", err));
                }
            };
            let buffer = Buffer {
                buffer_id,
                editor_id: self.editor_id,
            };
            let mut event = VsCodeEvent {
                buffer,
                read,
                // TODO: Avoid this clone by parsing the file path here.
                buffer_paths: self.buffer_paths.clone(),
            };
            on_event(buffer, &mut event).unwrap();
            read = event.read;
        }
    }
}

pub struct VsCodeDriver<W> {
    write: Arc<Mutex<W>>,
    buffer_paths: Arc<Mutex<HashMap<Buffer, PathBuf>>>,
}

impl<W> analysis::EditorDriver for VsCodeDriver<W>
where
    W: 'static + Write + Send,
{
    fn apply_edits(&self, refactor: Vec<Edit>) -> bool {
        let mut write_guard = crate::lock(&self.write);
        let mut write = write_guard.deref_mut();
        let buffer_paths = crate::lock(&self.buffer_paths);
        match write_refactor(&mut write, &buffer_paths, refactor) {
            Ok(()) => true,
            Err(err) => {
                log::error!("failed to write refactor to vscode: {:?}", err);
                false
            }
        }
    }
}

fn write_refactor<W: Write>(
    write: &mut W,
    buffer_paths: &HashMap<Buffer, PathBuf>,
    refactor: Vec<Edit>,
) -> Result<(), Error> {
    bytes::write_u32(write, refactor.len() as u32)?;
    for edit in refactor {
        let path = buffer_paths.get(&edit.buffer).unwrap();
        let path_bytes = path.as_os_str().as_bytes();
        let InputEdit {
            start_position,
            old_end_position,
            ..
        } = edit.input_edit;
        bytes::write_u32(write, path_bytes.len() as u32)?;
        write.write_all(path_bytes).map_err(|err| {
            log::mk_err!("failed writing path to vscode: {:?}", err)
        })?;
        bytes::write_u32(write, start_position.row as u32)?;
        bytes::write_u32(write, start_position.column as u32)?;
        bytes::write_u32(write, old_end_position.row as u32)?;
        bytes::write_u32(write, old_end_position.column as u32)?;
        bytes::write_u32(write, edit.new_bytes.len() as u32)?;
        write.write_all(edit.new_bytes.as_bytes()).map_err(|err| {
            log::mk_err!("failed writing change to vscode: {:?}", err)
        })?;
        write.flush().map_err(|err| {
            log::mk_err!("failed flushing refactor to vscode: {:?}", err)
        })?;
    }
    Ok(())
}

pub struct VsCodeEvent<R> {
    read: R,
    buffer: Buffer,
    buffer_paths: Arc<Mutex<HashMap<Buffer, PathBuf>>>,
}

impl<R: Read> EditorEvent for VsCodeEvent<R> {
    fn apply_to_buffer(
        &mut self,
        opt_code: Option<SourceFileSnapshot>,
    ) -> Result<BufferChange, crate::Error> {
        match bytes::read_u8(&mut self.read)? {
            NEW_FILE_MSG => parse_new_file_msg(
                &mut self.read,
                &mut crate::lock(&self.buffer_paths),
                self.buffer,
            ),
            FILE_CHANGED_MSG => {
                if let Some(code) = opt_code {
                    parse_file_changed_msg(&mut self.read, code)
                } else {
                    Err(log::mk_err!(
                        "vscode FILE_CHANGED_MSG for unknown buffer"
                    ))
                }
            }
            other => Err(log::mk_err!("unknown vscode msg type {}", other)),
        }
    }
}

fn parse_new_file_msg<R: Read>(
    read: &mut R,
    buffer_paths: &mut HashMap<Buffer, PathBuf>,
    buffer: Buffer,
) -> Result<BufferChange, Error> {
    let path_len = bytes::read_u32(read)?;
    let path_string = bytes::read_string(read, path_len as usize)?;
    let path = PathBuf::from(path_string);
    buffer_paths.insert(buffer, path.clone());
    let bytes_len = bytes::read_u32(read)?;
    let mut bytes_builder = ropey::RopeBuilder::new();
    bytes::read_chunks(
        read,
        bytes_len as usize,
        |err| log::mk_err!("failed reading code from vscode: {:?}", err),
        |str| {
            bytes_builder.append(str);
            Ok(())
        },
    )?;
    let change = BufferChange::OpenedNewBuffer {
        buffer,
        path,
        bytes: bytes_builder.finish(),
    };
    Ok(change)
}

fn parse_file_changed_msg<R: Read>(
    read: &mut R,
    mut code: SourceFileSnapshot,
) -> Result<BufferChange, Error> {
    let start_line = bytes::read_u32(read)?;
    let start_char = bytes::read_u32(read)?;
    let end_line = bytes::read_u32(read)?;
    let end_char = bytes::read_u32(read)?;
    let mut start_idx =
        code.bytes.line_to_char(start_line as usize) + start_char as usize;
    let start_byte = code.bytes.char_to_byte(start_idx);
    let start_position = Point {
        row: start_line as usize,
        column: start_char as usize,
    };
    let end_idx =
        code.bytes.line_to_char(end_line as usize) + end_char as usize;
    let old_end_byte = code.bytes.char_to_byte(end_idx);
    let old_end_position = Point {
        row: end_line as usize,
        column: end_char as usize,
    };
    code.bytes.remove(start_idx..end_idx);
    let new_code_len = bytes::read_u32(read)?;
    bytes::read_chunks(
        read,
        new_code_len as usize,
        |err| log::mk_err!("failed reading snippet from vscode: {:?}", err),
        |str| {
            code.bytes.insert(start_idx, str);
            start_idx += str.len();
            Ok(())
        },
    )?;
    let new_end_byte = code.bytes.char_to_byte(start_idx);
    let new_end_row = code.bytes.char_to_line(start_idx);
    let new_end_position = Point {
        row: new_end_row,
        column: start_idx - new_end_row,
    };
    let change = BufferChange::ModifiedBuffer {
        code,
        edit: InputEdit {
            start_byte,
            old_end_byte,
            new_end_byte,
            start_position,
            old_end_position,
            new_end_position,
        },
    };
    Ok(change)
}
