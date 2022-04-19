use crate::editors;
use crate::lib::bytes;
use crate::lib::log;
use crate::lib::log::Error;
use crate::lib::source_code::{
    Buffer, Edit, RefactorAllowed, SourceFileSnapshot,
};
use std::collections::HashMap;
use std::io::{BufReader, BufWriter, Read, Write};
use std::ops::DerefMut;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tree_sitter::{InputEdit, Point};

const MSG_NEW_FILE: u8 = 0;
const MSG_FILE_CHANGED: u8 = 1;

const CMD_REFACTOR: u8 = 0;
const CMD_OPEN_FILES: u8 = 1;

pub struct VsCode<R, W> {
    editor_id: editors::Id,
    read: R,
    write: Arc<Mutex<W>>,
    buffers: HashMap<Buffer, SourceFileSnapshot>,
    buffer_paths: Arc<Mutex<HashMap<Buffer, PathBuf>>>,
}

impl VsCode<BufReader<UnixStream>, BufWriter<UnixStream>> {
    pub fn from_unix_socket(
        socket: UnixStream,
        editor_id: editors::Id,
    ) -> Result<Self, crate::Error> {
        let write = socket.try_clone().map_err(|err| {
            log::mk_err!("failed cloning vscode socket: {:?}", err)
        })?;
        let vscode = VsCode {
            editor_id,
            read: BufReader::new(socket),
            write: Arc::new(Mutex::new(BufWriter::new(write))),
            buffers: HashMap::new(),
            buffer_paths: Arc::new(Mutex::new(HashMap::new())),
        };
        Ok(vscode)
    }
}

impl<R: Read, W: 'static + Write + Send> editors::Editor for VsCode<R, W> {
    type Driver = VsCodeDriver<W>;

    fn driver(&self) -> VsCodeDriver<W> {
        VsCodeDriver {
            write: self.write.clone(),
            buffer_paths: self.buffer_paths.clone(),
        }
    }

    fn kind(&self) -> editors::Kind {
        editors::Kind::VsCode
    }

    fn listen<F>(mut self, mut on_event: F) -> Result<(), crate::Error>
    where
        F: FnMut(editors::Event) -> Result<(), crate::Error>,
    {
        loop {
            let mut u32_buffer = [0; 4];
            let buffer_id = match self.read.read_exact(&mut u32_buffer) {
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

            let opt_code = self.buffers.remove(&buffer);

            let event = match bytes::read_u8(&mut self.read)? {
                MSG_NEW_FILE => parse_new_file_msg(
                    &mut self.read,
                    &mut crate::lock(&self.buffer_paths),
                    buffer,
                ),
                MSG_FILE_CHANGED => {
                    if let Some(code) = opt_code {
                        parse_file_changed_msg(&mut self.read, code)
                    } else {
                        Err(log::mk_err!(
                            "vscode MSG_FILE_CHANGED for unknown buffer"
                        ))
                    }
                }
                other => Err(log::mk_err!("unknown vscode msg type {}", other)),
            }?;

            self.buffers.insert(
                buffer,
                match &event {
                    editors::Event::ModifiedBuffer { code, .. } => code.clone(),
                    editors::Event::OpenedNewBuffer { code, .. } => {
                        code.clone()
                    }
                },
            );

            on_event(event)?;
        }
    }
}

pub struct VsCodeDriver<W> {
    write: Arc<Mutex<W>>,
    buffer_paths: Arc<Mutex<HashMap<Buffer, PathBuf>>>,
}

impl<W> editors::Driver for VsCodeDriver<W>
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
    fn open_files(&self, files: Vec<PathBuf>) -> bool {
        let mut write_guard = crate::lock(&self.write);
        let mut write = write_guard.deref_mut();
        match write_open_files(&mut write, files) {
            Ok(()) => true,
            Err(err) => {
                log::error!("failed to write open files to vscode: {:?}", err);
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
    bytes::write_u8(write, CMD_REFACTOR)?;
    bytes::write_u32(write, refactor.len() as u32)?; //no. of edits in refactor
    for edit in refactor {
        let path = buffer_paths.get(&edit.buffer).unwrap();
        write_path(write, path)?;
        let InputEdit {
            start_position,
            old_end_position,
            ..
        } = edit.input_edit;
        bytes::write_u32(write, start_position.row as u32)?;
        bytes::write_u32(write, start_position.column as u32)?;
        bytes::write_u32(write, old_end_position.row as u32)?;
        bytes::write_u32(write, old_end_position.column as u32)?;
        bytes::write_u32(write, edit.new_bytes.len() as u32)?;
        write.write_all(edit.new_bytes.as_bytes()).map_err(|err| {
            log::mk_err!("failed writing change to vscode: {:?}", err)
        })?;
    }
    write.flush().map_err(|err| {
        log::mk_err!("failed flushing refactor to vscode: {:?}", err)
    })
}

fn write_open_files<W: Write>(
    write: &mut W,
    files: Vec<PathBuf>,
) -> Result<(), Error> {
    bytes::write_u8(write, CMD_OPEN_FILES)?;
    bytes::write_u32(write, files.len() as u32)?;
    for path in files {
        write_path(write, &path)?;
    }
    write.flush().map_err(|err| {
        log::mk_err!("failed flushing open files cmd to vscode: {:?}", err)
    })
}

fn write_path<W: Write>(write: &mut W, path: &Path) -> Result<(), Error> {
    let path_bytes = path.as_os_str().as_bytes();
    bytes::write_u32(write, path_bytes.len() as u32)?;
    write
        .write_all(path_bytes)
        .map_err(|err| log::mk_err!("failed writing path to vscode: {:?}", err))
}

fn parse_new_file_msg<R: Read>(
    read: &mut R,
    buffer_paths: &mut HashMap<Buffer, PathBuf>,
    buffer: Buffer,
) -> Result<editors::Event, Error> {
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
    let change = editors::Event::OpenedNewBuffer {
        code: SourceFileSnapshot::new(buffer, bytes_builder.finish())?,
        path,
    };
    Ok(change)
}

fn parse_file_changed_msg<R: Read>(
    read: &mut R,
    mut code: SourceFileSnapshot,
) -> Result<editors::Event, Error> {
    let refactor_allowed = if bytes::read_u8(read)? == 0 {
        RefactorAllowed::No
    } else {
        RefactorAllowed::Yes
    };
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
    code.apply_edit(InputEdit {
        start_byte,
        old_end_byte,
        new_end_byte,
        start_position,
        old_end_position,
        new_end_position,
    })?;
    let change = editors::Event::ModifiedBuffer {
        code,
        refactor_allowed,
    };
    Ok(change)
}
