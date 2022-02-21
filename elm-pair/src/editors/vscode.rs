use crate::analysis_thread as analysis;
use crate::editor_listener_thread::{BufferChange, Editor, EditorEvent};
use crate::lib::bytes;
use crate::lib::log;
use crate::lib::log::Error;
use crate::lib::source_code::{Buffer, Edit, SourceFileSnapshot};
use std::io::{BufReader, BufWriter, Read, Write};
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
        };
        Ok(vscode)
    }
}

impl<R: Read, W: 'static + Write + Send> Editor for VsCode<R, W> {
    type Driver = VsCodeDriver<W>;
    type Event = VsCodeEvent<R>;

    fn driver(&self) -> VsCodeDriver<W> {
        VsCodeDriver {
            _write: self.write.clone(),
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
            // TODO: check for EOF.
            let buffer = Buffer {
                buffer_id: bytes::read_u32(&mut read)?,
                editor_id: self.editor_id,
            };
            let mut event = VsCodeEvent { buffer, read };
            on_event(buffer, &mut event).unwrap();
            read = event.read;
        }
    }
}

pub struct VsCodeDriver<W> {
    _write: Arc<Mutex<W>>,
}

impl<W> analysis::EditorDriver for VsCodeDriver<W>
where
    W: 'static + Write + Send,
{
    fn apply_edits(&self, _refactor: Vec<Edit>) -> bool {
        println!("TODO: refactor here");
        true
    }
}

pub struct VsCodeEvent<R> {
    read: R,
    buffer: Buffer,
}

impl<R: Read> EditorEvent for VsCodeEvent<R> {
    fn apply_to_buffer(
        &mut self,
        opt_code: Option<SourceFileSnapshot>,
    ) -> Result<BufferChange, crate::Error> {
        match bytes::read_u8(&mut self.read)? {
            NEW_FILE_MSG => parse_new_file_msg(&mut self.read, self.buffer),
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
    buffer: Buffer,
) -> Result<BufferChange, Error> {
    let path_len = bytes::read_u32(read)?;
    let path_string = bytes::read_string(read, path_len as usize)?;
    let path = PathBuf::from(path_string);
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
