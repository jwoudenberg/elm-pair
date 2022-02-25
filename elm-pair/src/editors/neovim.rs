use crate::analysis_thread as analysis;
use crate::editor_listener_thread::{BufferChange, Editor, EditorEvent};
use crate::lib::bytes;
use crate::lib::bytes::read_chunks;
use crate::lib::log;
use crate::lib::log::Error;
use crate::lib::source_code::{
    byte_to_point, Buffer, Edit, RefactorAllowed, SourceFileSnapshot,
};
use byteorder::ReadBytesExt;
use messagepack::read_tuple;
use ropey::{Rope, RopeBuilder};
use std::collections::HashMap;
use std::io::{BufReader, BufWriter, Read, Write};
use std::ops::DerefMut;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tree_sitter::InputEdit;

pub struct Neovim<R, W> {
    editor_id: u32,
    read: R,
    write: Arc<Mutex<W>>,
}

impl Neovim<BufReader<UnixStream>, BufWriter<UnixStream>> {
    pub fn from_unix_socket(
        socket: UnixStream,
        editor_id: u32,
    ) -> Result<Self, crate::Error> {
        let write = socket.try_clone().map_err(|err| {
            log::mk_err!("failed cloning neovim socket: {:?}", err)
        })?;
        let neovim = Neovim {
            editor_id,
            read: BufReader::new(socket),
            write: Arc::new(Mutex::new(BufWriter::new(write))),
        };
        Ok(neovim)
    }
}

impl<R: Read, W: 'static + Write + Send> Editor for Neovim<R, W> {
    type Driver = NeovimDriver<W>;
    type Event = NeovimEvent<R>;

    fn driver(&self) -> NeovimDriver<W> {
        NeovimDriver {
            write: self.write.clone(),
        }
    }

    fn name(&self) -> &'static str {
        "neovim"
    }

    fn listen<F>(self, on_event: F) -> Result<(), crate::Error>
    where
        F: FnMut(Buffer, &mut Self::Event) -> Result<(), crate::Error>,
    {
        let mut listener = NeovimListener {
            editor_id: self.editor_id,
            read: self.read,
            write: self.write,
            on_event,
            paths_for_new_buffers: HashMap::new(),
        };
        while let Some(new_listener) = listener.parse_msg()? {
            listener = new_listener;
        }
        Ok(())
    }
}

struct NeovimListener<R, W, F> {
    editor_id: u32,
    read: R,
    write: Arc<Mutex<W>>,
    on_event: F,
    paths_for_new_buffers: HashMap<Buffer, PathBuf>,
}

impl<R, W, F> NeovimListener<R, W, F>
where
    R: Read,
    W: Write,
    F: FnMut(Buffer, &mut NeovimEvent<R>) -> Result<(), crate::Error>,
{
    // Messages we receive from neovim's webpack-rpc API:
    // neovim api:  https://neovim.io/doc/user/api.html
    // webpack-rpc: https://github.com/msgpack-rpc/msgpack-rpc/blob/master/spec.md
    //
    // TODO handle neovim API versions
    fn parse_msg(mut self) -> Result<Option<Self>, Error> {
        let array_len_res = rmp::decode::read_array_len(&mut self.read);
        // There's currently no way to check if there's more to read save by
        // trying to read some bytes and seeing what happens. That's what we
        // do here. If we run into an EOF on the first byte(s) of a new message,
        // then the EOF is on a message boundary, so we'll shut down gracefully.
        let array_len = match &array_len_res {
            Err(rmp::decode::ValueReadError::InvalidMarkerRead(io_err)) => {
                if io_err.kind() == std::io::ErrorKind::UnexpectedEof {
                    return Ok(None);
                } else {
                    array_len_res?
                }
            }
            _ => array_len_res?,
        };
        let type_: i32 = rmp::decode::read_int(&mut self.read)?;
        if array_len == 3 && type_ == 2 {
            let new_self = self.parse_notification_msg()?;
            Ok(Some(new_self))
        } else {
            Err(log::mk_err!(
                "received unknown msgpack-rpc message with length {:?} and type {:?}",
                array_len,
                type_
            ))
        }
    }

    fn parse_notification_msg(mut self) -> Result<Self, Error> {
        let mut buffer = [0u8; 30];
        let len = rmp::decode::read_str_len(&mut self.read)? as usize;
        if len > buffer.len() {
            return Err(log::mk_err!(
                "name of received msgpack-rpc message length {:?} exceeds max length {:?}",
                len,
                buffer.len()
            ));
        }
        self.read.read_exact(&mut buffer[0..len]).map_err(|err| {
            log::mk_err!("failed reading msgpack-rpc message name: {:?}", err)
        })?;
        match &buffer[0..len] {
            b"nvim_error_event" => self.parse_error_event()?,
            b"nvim_buf_lines_event" => return self.parse_buf_lines_event(),
            b"nvim_buf_changedtick_event" => {
                self.parse_buf_changedtick_event()?
            }
            b"nvim_buf_detach_event" => self.parse_buf_detach_event()?,
            b"buffer_opened" => self.parse_buffer_opened()?,
            method => {
                return Err(log::mk_err!(
                    "received neovim message with unknown name: {:?}",
                    from_utf8(method)
                ))
            }
        };
        Ok(self)
    }

    fn parse_error_event(&mut self) -> Result<(), Error> {
        read_tuple!(
            &mut self.read,
            type_ = rmp::decode::read_int(&mut self.read)?,
            msg = {
                let len = rmp::decode::read_str_len(&mut self.read)?;
                let mut buffer = vec![0; len as usize];
                self.read.read_exact(&mut buffer).map_err(|err| {
                    log::mk_err!(
                        "failed reading error out of neovim message: {:?}",
                        err
                    )
                })?;
                from_utf8(&buffer)?.to_owned()
            }
        );
        let type_: u64 = type_; // for type inference.
        Err(log::mk_err!(
            "received error from neovim: {:?} {}",
            type_,
            msg
        ))
    }

    fn parse_buffer_opened(&mut self) -> Result<(), Error> {
        read_tuple!(
            &mut self.read,
            buf = Buffer {
                editor_id: self.editor_id,
                buffer_id: rmp::decode::read_int(&mut self.read)?
            },
            path = {
                let len = rmp::decode::read_str_len(&mut self.read)?;
                let mut buffer = vec![0; len as usize];
                self.read.read_exact(&mut buffer).map_err(|err| {
                    log::mk_err!("failed reading msgpack-rpc string: {:?}", err)
                })?;
                Path::new(from_utf8(&buffer)?).to_owned()
            }
        );
        self.paths_for_new_buffers.insert(buf, path);
        self.nvim_buf_attach(buf)
    }

    fn parse_buf_lines_event(mut self) -> Result<Self, Error> {
        read_tuple!(
            &mut self.read,
            buffer = Buffer {
                editor_id: self.editor_id,
                buffer_id: read_buf(&mut self.read)?,
            },
            _changedtick = skip_objects(&mut self.read, 1)?,
            firstline = rmp::decode::read_int(&mut self.read)?,
            lastline = rmp::decode::read_int(&mut self.read)?,
            _linedata = {
                let mut update = NeovimEvent {
                    editor_id: self.editor_id,
                    paths_for_new_buffers: self.paths_for_new_buffers,
                    read: self.read,
                    firstline,
                    lastline,
                    buffer,
                };
                (self.on_event)(buffer, &mut update)?;
                self = NeovimListener {
                    editor_id: update.editor_id,
                    paths_for_new_buffers: update.paths_for_new_buffers,
                    read: update.read,
                    write: self.write,
                    on_event: self.on_event,
                }
            }
        );
        Ok(self)
    }

    fn parse_buf_changedtick_event(&mut self) -> Result<(), Error> {
        // We're not interested in these events, so we skip them.
        read_tuple!(&mut self.read);
        Ok(())
    }

    fn parse_buf_detach_event(&mut self) -> Result<(), Error> {
        // Re-attach this buffer
        // TODO: consider when we might not want to reattach.
        read_tuple!(&mut self.read, buffer_id = read_buf(&mut self.read)?);
        self.nvim_buf_attach(Buffer {
            editor_id: self.editor_id,
            buffer_id,
        })
    }

    fn nvim_buf_attach(&self, buf: Buffer) -> Result<(), Error> {
        let mut write_guard = crate::lock(&self.write);
        let write = write_guard.deref_mut();
        rmp::encode::write_array_len(write, 3)?;
        rmp::encode::write_i8(write, 2)?;
        write_str(write, "nvim_buf_attach")?;
        // nvim_buf_attach arguments
        rmp::encode::write_array_len(write, 3)?;
        rmp::encode::write_u32(write, buf.buffer_id)?; //buf
        rmp::encode::write_bool(write, true).map_err(|err| {
            log::mk_err!("failed writing to neovim: {:?}", err)
        })?; // send_buffer
        rmp::encode::write_map_len(write, 0)?; // opts
        write.flush().map_err(|err| {
            log::mk_err!("failed writing to neovim: {:?}", err)
        })?; // send_buff
        Ok(())
    }
}

pub struct NeovimEvent<R> {
    editor_id: u32,
    read: R,
    paths_for_new_buffers: HashMap<Buffer, PathBuf>,
    firstline: i64,
    lastline: i64,
    buffer: Buffer,
}

impl<R: Read> EditorEvent for NeovimEvent<R> {
    fn apply_to_buffer(
        &mut self,
        opt_code: Option<SourceFileSnapshot>,
    ) -> Result<BufferChange, crate::Error> {
        let contains_entire_buffer = self.lastline == -1;
        if contains_entire_buffer {
            let rope = self.read_rope()?;
            Ok(BufferChange::OpenedNewBuffer {
                buffer: self.buffer,
                bytes: rope,
                path: self
                    .paths_for_new_buffers
                    .remove(&self.buffer)
                    .ok_or_else(|| {
                        log::mk_err!(
                            "received neovim lines event for unkonwn buffer: {:?}",
                            self.buffer,
                        )
                    })?,
            })
        } else if let Some(mut code) = opt_code {
            let edit = self.apply_change(
                self.firstline,
                self.lastline,
                &mut code.bytes,
            )?;
            Ok(BufferChange::ModifiedBuffer {
                code,
                edit,
                refactor_allowed: RefactorAllowed::Yes,
            })
        } else {
            log::error!(
                "received incremental buffer update before full update"
            );
            // TODO: re-attach buffer to get initial lines event.
            Ok(BufferChange::NoChanges)
        }
    }
}

impl<R: Read> NeovimEvent<R> {
    fn apply_change(
        &mut self,
        firstline: i64,
        lastline: i64,
        code: &mut Rope,
    ) -> Result<InputEdit, Error> {
        let start_line = firstline as usize;
        let old_end_line = lastline as usize;
        let start_char = code.line_to_char(start_line);
        let start_byte = code.line_to_byte(start_line);
        let old_end_char = code.line_to_char(old_end_line);
        let old_end_byte = code.line_to_byte(old_end_line);
        let mut new_end_byte = start_byte;
        let old_end_position = byte_to_point(code, old_end_byte);
        code.remove(start_char..old_end_char);
        let mut remaining_lines = rmp::decode::read_array_len(&mut self.read)?;
        while remaining_lines > 0 {
            remaining_lines -= 1;
            let len = rmp::decode::read_str_len(&mut self.read)?;
            read_chunks(
                &mut self.read,
                len as usize,
                |err| {
                    log::mk_err!(
                        "failed reading string from msgpack-rpc message: {:?}",
                        err
                    )
                },
                |chunk| {
                    code.insert(code.byte_to_char(new_end_byte), chunk);
                    new_end_byte += chunk.len();
                    Ok(())
                },
            )?;
            code.insert_char(code.byte_to_char(new_end_byte), '\n');
            new_end_byte += 1;
        }
        Ok(InputEdit {
            start_byte,
            old_end_byte,
            new_end_byte,
            start_position: byte_to_point(code, start_byte),
            old_end_position,
            new_end_position: byte_to_point(code, new_end_byte),
        })
    }

    fn read_rope(&mut self) -> Result<Rope, Error> {
        let mut builder = RopeBuilder::new();
        let mut remaining_lines = rmp::decode::read_array_len(&mut self.read)?;
        while remaining_lines > 0 {
            remaining_lines -= 1;
            let len = rmp::decode::read_str_len(&mut self.read)?;
            read_chunks(
                &mut self.read,
                len as usize,
                |err| {
                    log::mk_err!(
                        "failed reading string from msgpack-rpc message: {:?}",
                        err
                    )
                },
                |chunk| {
                    builder.append(chunk);
                    Ok(())
                },
            )?;
            builder.append("\n");
        }
        Ok(builder.finish())
    }
}

// Skip `count` messagepack options. If one of these objects is an array or
// map, skip its contents too.
fn skip_objects<R>(read: &mut R, count: u32) -> Result<(), Error>
where
    R: Read,
{
    let mut count = count;
    while count > 0 {
        count -= 1;
        let marker = rmp::decode::read_marker(read)?;
        count += skip_one_object(read, marker).map_err(|err| {
            log::mk_err!(
                "failed skipping data in msgpack-rpc stream: {:?}",
                err
            )
        })?;
    }
    Ok(())
}

fn skip_one_object<R>(
    read: &mut R,
    marker: rmp::Marker,
) -> Result<u32, std::io::Error>
where
    R: Read,
{
    match marker {
        rmp::Marker::FixPos(_) => {}
        rmp::Marker::FixNeg(_) => {}
        rmp::Marker::Null => {}
        rmp::Marker::True => {}
        rmp::Marker::False => {}
        rmp::Marker::U8 => bytes::skip(read, 1)?,
        rmp::Marker::U16 => bytes::skip(read, 2)?,
        rmp::Marker::U32 => bytes::skip(read, 4)?,
        rmp::Marker::U64 => bytes::skip(read, 8)?,
        rmp::Marker::I8 => bytes::skip(read, 1)?,
        rmp::Marker::I16 => bytes::skip(read, 2)?,
        rmp::Marker::I32 => bytes::skip(read, 4)?,
        rmp::Marker::I64 => bytes::skip(read, 8)?,
        rmp::Marker::F32 => bytes::skip(read, 4)?,
        rmp::Marker::F64 => bytes::skip(read, 8)?,
        rmp::Marker::FixStr(bytes) => bytes::skip(read, bytes as u64)?,
        rmp::Marker::Str8 => {
            let bytes = read.read_u8()?;
            bytes::skip(read, bytes as u64)?;
        }
        rmp::Marker::Str16 => {
            let bytes = read.read_u16::<byteorder::BigEndian>()?;
            bytes::skip(read, bytes as u64)?
        }
        rmp::Marker::Str32 => {
            let bytes = read.read_u32::<byteorder::BigEndian>()?;
            bytes::skip(read, bytes as u64)?
        }
        rmp::Marker::Bin8 => {
            let bytes = read.read_u8()?;
            bytes::skip(read, bytes as u64)?
        }
        rmp::Marker::Bin16 => {
            let bytes = read.read_u16::<byteorder::BigEndian>()?;
            bytes::skip(read, bytes as u64)?
        }
        rmp::Marker::Bin32 => {
            let bytes = read.read_u32::<byteorder::BigEndian>()?;
            bytes::skip(read, bytes as u64)?
        }
        rmp::Marker::FixArray(objects) => {
            return Ok(objects as u32);
        }
        rmp::Marker::Array16 => {
            let objects = read.read_u16::<byteorder::BigEndian>()?;
            return Ok(objects as u32);
        }
        rmp::Marker::Array32 => {
            let objects = read.read_u32::<byteorder::BigEndian>()?;
            return Ok(objects);
        }
        rmp::Marker::FixMap(entries) => {
            return Ok(2 * entries as u32);
        }
        rmp::Marker::Map16 => {
            let entries = read.read_u16::<byteorder::BigEndian>()?;
            return Ok(2 * entries as u32);
        }
        rmp::Marker::Map32 => {
            let entries = read.read_u32::<byteorder::BigEndian>()?;
            return Ok(2 * entries);
        }
        rmp::Marker::FixExt1 => bytes::skip(read, 2)?,
        rmp::Marker::FixExt2 => bytes::skip(read, 3)?,
        rmp::Marker::FixExt4 => bytes::skip(read, 5)?,
        rmp::Marker::FixExt8 => bytes::skip(read, 9)?,
        rmp::Marker::FixExt16 => bytes::skip(read, 17)?,
        rmp::Marker::Ext8 => {
            let bytes = read.read_u8()?;
            bytes::skip(read, 1 + bytes as u64)?
        }
        rmp::Marker::Ext16 => {
            let bytes = read.read_u16::<byteorder::BigEndian>()?;
            bytes::skip(read, 1 + bytes as u64)?
        }
        rmp::Marker::Ext32 => {
            let bytes = read.read_u32::<byteorder::BigEndian>()?;
            bytes::skip(read, 1 + bytes as u64)?
        }
        rmp::Marker::Reserved => {}
    }
    Ok(0)
}

fn from_utf8(buffer: &[u8]) -> Result<&str, Error> {
    let str = std::str::from_utf8(buffer).map_err(|err| {
        log::mk_err!(
            "failed decoding string from msgpack-rpc message as utf8: {:?}",
            err
        )
    })?;
    Ok(str)
}

fn read_buf<R>(read: &mut R) -> Result<u32, Error>
where
    R: Read,
{
    let (_, buf) = rmp::decode::read_fixext1(read)?;
    Ok(buf as u32)
}

pub struct NeovimDriver<W> {
    write: Arc<Mutex<W>>,
}

impl<W> analysis::EditorDriver for NeovimDriver<W>
where
    W: 'static + Write + Send,
{
    fn apply_edits(&self, refactor: Vec<Edit>) -> bool {
        match self.write_refactor(refactor) {
            Ok(()) => true,
            Err(err) => {
                log::error!("failed sending refactor to neovim: {:?}", err);
                false
            }
        }
    }
}

impl<W> NeovimDriver<W>
where
    W: Write,
{
    // TODO: Send back changedtick, and let Neovim apply update only when it
    // hasn't changed.
    fn write_refactor(&self, refactor: Vec<Edit>) -> Result<(), Error> {
        let mut write_guard = crate::lock(&self.write);
        let write = write_guard.deref_mut();
        rmp::encode::write_array_len(write, 3)?; // msgpack envelope
        rmp::encode::write_i8(write, 2)?;
        write_str(write, "nvim_call_atomic")?;

        rmp::encode::write_array_len(write, 1)?; // nvim_call_atomic args

        rmp::encode::write_array_len(write, refactor.len() as u32)?; // calls array
        for edit in refactor {
            let start = edit.input_edit.start_position;
            let end = edit.input_edit.old_end_position;

            rmp::encode::write_array_len(write, 2)?; // call tuple
            write_str(write, "nvim_buf_set_text")?;

            rmp::encode::write_array_len(write, 6)?; // nvim_buf_set_text args
            rmp::encode::write_u32(write, edit.buffer.buffer_id)?;
            rmp::encode::write_u64(write, start.row as u64)?;
            rmp::encode::write_u64(write, start.column as u64)?;
            rmp::encode::write_u64(write, end.row as u64)?;
            rmp::encode::write_u64(write, end.column as u64)?;

            // Not using the `lines()` function here, because it will drop
            // a trailing newline resulting in newlines disappearing in Neovim.
            let lines = edit.new_bytes.split('\n');
            rmp::encode::write_array_len(write, lines.clone().count() as u32)?; // array of lines
            for line in lines {
                write_str(write, line)?;
            }
        }
        write.flush().map_err(|err| {
            log::mk_err!("failed writing to neovim: {:?}", err)
        })?;
        Ok(())
    }
}

fn write_str<W>(write: &mut W, str: &str) -> Result<(), Error>
where
    W: Write,
{
    let bytes = str.as_bytes();
    rmp::encode::write_str_len(write, bytes.len() as u32)?;
    write
        .write_all(bytes)
        .map_err(|err| log::mk_err!("failed writing to neovim: {:?}", err))?;
    Ok(())
}

mod messagepack {
    // Helper macro that counts the numver of arguments passed to it.
    // Taken from: https://stackoverflow.com/questions/34304593/counting-length-of-repetition-in-macro
    #[macro_export]
    macro_rules! count {
        () => (0usize);
        ( $x:tt $($xs:tt)* ) => (1usize + messagepack::count!($($xs)*));
    }
    pub use count;

    // A macro for safely reading an messagepack array. The macro takes care of
    // checking we get at least the expected amount of items, and skips over extra
    // elements we're not interested in.
    //
    //     read_tuple!(
    //         read,
    //         watts = read_int8(read)?,
    //         defrost = read_bool(read)?,
    //     )
    //     println!("The microwave is set to {:?} Watts", watts);
    //
    #[macro_export]
    macro_rules! read_tuple {
        ($read:expr) => { {
            let array_len = rmp::decode::read_array_len($read)?;
            skip_objects($read, array_len)?;
        } };
        ($read:expr, $( $name:ident = $x:expr ),* ) => {
            let array_len = rmp::decode::read_array_len($read)?;
            let expected_len = messagepack::count!($($x)*) as u32;
            if array_len  < expected_len {
                return Err(
                    log::mk_err!(
                        "messagepack array contains {:?} elements, while I expected at least {:?}",
                        array_len,
                        expected_len,
                    )
                )
            }
            $( let $name = $x; )*
            skip_objects($read, array_len - expected_len)?;
        };
    }
    pub use read_tuple;
}
