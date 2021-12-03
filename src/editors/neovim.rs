use crate::analysis_thread as analysis;
use crate::editor_listener_thread as editor_listener;
use crate::editor_listener_thread::{Editor, EditorEvent};
use crate::{Edit, InputEdit, SourceFileSnapshot};
use byteorder::ReadBytesExt;
use messagepack::{read_tuple, DecodingError};
use ropey::{Rope, RopeBuilder};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::ops::DerefMut;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub(crate) struct Neovim<R, W> {
    read: R,
    write: Arc<Mutex<W>>,
}

impl<R: Read, W: Write> Neovim<R, W> {
    pub fn new(read: R, write: W) -> Self
    where
        R: Read,
    {
        Neovim {
            read,
            write: Arc::new(Mutex::new(write)),
        }
    }
}

impl<R: Read, W: 'static + Write + Send> Editor for Neovim<R, W> {
    type Driver = NeovimDriver<W>;

    fn driver(&self) -> NeovimDriver<W> {
        NeovimDriver {
            write: self.write.clone(),
        }
    }

    fn listen<F, G>(
        self,
        load_code_copy: F,
        store_new_code: G,
    ) -> Result<(), editor_listener::Error>
    where
        F: FnMut(usize) -> Result<SourceFileSnapshot, editor_listener::Error>,
        G: FnMut(EditorEvent) -> Result<(), editor_listener::Error>,
    {
        let mut listener = NeovimListener {
            read: self.read,
            write: self.write,
            load_code_copy,
            store_new_code,
            paths_for_new_buffers: HashMap::new(),
        };
        while listener.parse_msg()? {}
        Ok(())
    }
}

struct NeovimListener<R, W, F, G> {
    read: R,
    write: Arc<Mutex<W>>,
    load_code_copy: F,
    store_new_code: G,
    paths_for_new_buffers: HashMap<usize, PathBuf>,
}

impl<R, W, F, G> NeovimListener<R, W, F, G>
where
    R: Read,
    W: Write,
    F: FnMut(usize) -> Result<SourceFileSnapshot, editor_listener::Error>,
    G: FnMut(EditorEvent) -> Result<(), editor_listener::Error>,
{
    // Messages we receive from neovim's webpack-rpc API:
    // neovim api:  https://neovim.io/doc/user/api.html
    // webpack-rpc: https://github.com/msgpack-rpc/msgpack-rpc/blob/master/spec.md
    //
    // TODO handle neovim API versions
    fn parse_msg(&mut self) -> Result<bool, Error> {
        let array_len_res = rmp::decode::read_array_len(&mut self.read);
        // There's currently no way to check if there's more to read save by
        // trying to read some bytes and seeing what happens. That's what we
        // do here. If we run into an EOF on the first byte(s) of a new message,
        // then the EOF is on a message boundary, so we'll shut down gracefully.
        let array_len = match &array_len_res {
            Err(rmp::decode::ValueReadError::InvalidMarkerRead(io_err)) => {
                if io_err.kind() == std::io::ErrorKind::UnexpectedEof {
                    return Ok(false);
                } else {
                    array_len_res?
                }
            }
            _ => array_len_res?,
        };
        let type_ = rmp::decode::read_int(&mut self.read)?;
        if array_len == 3 && type_ == 2 {
            self.parse_notification_msg()?;
            Ok(true)
        } else {
            Err(Error::UnknownMessageType(array_len, type_))
        }
    }

    fn parse_notification_msg(&mut self) -> Result<(), Error> {
        let mut buffer = [0u8; 30];
        let len = rmp::decode::read_str_len(&mut self.read)? as usize;
        if len > buffer.len() {
            return Err(
                DecodingError::BufferCannotHoldString(len as u32).into()
            );
        }
        self.read
            .read_exact(&mut buffer[0..len])
            .map_err(DecodingError::ReadingString)?;
        match &buffer[0..len] {
            b"nvim_error_event" => self.parse_error_event(),
            b"nvim_buf_lines_event" => self.parse_buf_lines_event(),
            b"nvim_buf_changedtick_event" => self.parse_buf_changedtick_event(),
            b"nvim_buf_detach_event" => self.parse_buf_detach_event(),
            b"buffer_opened" => self.parse_buffer_opened(),
            method => {
                Err(Error::UnknownEventMethod(to_utf8(method)?.to_owned()))
            }
        }
    }

    fn parse_error_event(&mut self) -> Result<(), Error> {
        read_tuple!(
            &mut self.read,
            type_ = rmp::decode::read_int(&mut self.read)?,
            msg = {
                let len = rmp::decode::read_str_len(&mut self.read)?;
                let mut buffer = vec![0; len as usize];
                self.read
                    .read_exact(&mut buffer)
                    .map_err(DecodingError::ReadingString)?;
                to_utf8(&buffer)?.to_owned()
            }
        );
        Err(Error::ReceivedErrorEvent(type_, msg))
    }

    fn parse_buffer_opened(&mut self) -> Result<(), Error> {
        read_tuple!(
            &mut self.read,
            buf = rmp::decode::read_int(&mut self.read)?,
            path = {
                let len = rmp::decode::read_str_len(&mut self.read)?;
                let mut buffer = vec![0; len as usize];
                self.read
                    .read_exact(&mut buffer)
                    .map_err(DecodingError::ReadingString)?;
                Path::new(to_utf8(&buffer)?).to_owned()
            }
        );
        self.paths_for_new_buffers.insert(buf, path);
        self.nvim_buf_attach(buf)
    }

    fn parse_buf_lines_event(&mut self) -> Result<(), Error> {
        read_tuple!(
            &mut self.read,
            buffer = read_buf(&mut self.read)?,
            _changedtick = skip_objects(&mut self.read, 1)?,
            firstline = rmp::decode::read_int(&mut self.read)?,
            lastline = rmp::decode::read_int(&mut self.read)?,
            _linedata =
                {
                    if lastline == -1 {
                        let rope = self.read_rope()?;
                        (self.store_new_code)(EditorEvent::OpenedNewSourceFile {
                        bytes: rope,
                        buffer,
                        path: self.paths_for_new_buffers.remove(&buffer).ok_or(
                            Error::ReceivedLinesEventForUnknownBuffer(buffer),
                        )?,
                    })?;
                    } else {
                        let mut code = (self.load_code_copy)(buffer)?;
                        let edit = self.apply_change(
                            firstline,
                            lastline,
                            &mut code.bytes,
                        )?;
                        (self.store_new_code)(
                            EditorEvent::ModifiedSourceFile {
                                _buffer: buffer,
                                code,
                                edit,
                            },
                        )?;
                    }
                }
        );
        Ok(())
    }

    fn parse_buf_changedtick_event(&mut self) -> Result<(), Error> {
        // We're not interested in these events, so we skip them.
        read_tuple!(&mut self.read);
        Ok(())
    }

    fn parse_buf_detach_event(&mut self) -> Result<(), Error> {
        // Re-attach this buffer
        // TODO: consider when we might not want to reattach.
        read_tuple!(&mut self.read, buf = read_buf(&mut self.read)?);
        self.nvim_buf_attach(buf)
    }

    fn nvim_buf_attach(&self, buf: usize) -> Result<(), Error> {
        let mut write_guard = crate::lock(&self.write);
        let write = write_guard.deref_mut();
        rmp::encode::write_array_len(write, 3)?;
        rmp::encode::write_i8(write, 2)?;
        write_str(write, "nvim_buf_attach")?;
        // nvim_buf_attach arguments
        rmp::encode::write_array_len(write, 3)?;
        rmp::encode::write_u32(write, buf as u32)?; //buf
        rmp::encode::write_bool(write, true)
            .map_err(Error::EncodingFailedWhileWritingData)?; // send_buffer
        rmp::encode::write_map_len(write, 0)?; // opts
        Ok(())
    }

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
        let old_end_position = crate::byte_to_point(code, old_end_byte);
        code.remove(start_char..old_end_char);
        let mut remaining_lines = rmp::decode::read_array_len(&mut self.read)?;
        while remaining_lines > 0 {
            remaining_lines -= 1;
            let len = rmp::decode::read_str_len(&mut self.read)?;
            read_chunks(
                &mut self.read,
                len as usize,
                DecodingError::ReadingString,
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
            start_position: crate::byte_to_point(code, start_byte),
            old_end_position,
            new_end_position: crate::byte_to_point(code, new_end_byte),
        })
    }

    fn read_rope(&mut self) -> Result<Rope, DecodingError> {
        let mut builder = RopeBuilder::new();
        let mut remaining_lines = rmp::decode::read_array_len(&mut self.read)?;
        while remaining_lines > 0 {
            remaining_lines -= 1;
            let len = rmp::decode::read_str_len(&mut self.read)?;
            read_chunks(
                &mut self.read,
                len as usize,
                DecodingError::ReadingString,
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

#[derive(Debug)]
pub(crate) enum Error {
    DecodingFailed(DecodingError),
    EncodingFailedWhileWritingMarker(std::io::Error),
    EncodingFailedWhileWritingData(std::io::Error),
    EncodingFailedWhileWritingString(std::io::Error),
    UnknownMessageType(u32, u8),
    UnknownEventMethod(String),
    ReceivedErrorEvent(u64, String),
    FailedWhileProcessingBufChange(Box<editor_listener::Error>),
    ReceivedLinesEventForUnknownBuffer(usize),
}

impl From<Error> for editor_listener::Error {
    fn from(err: Error) -> editor_listener::Error {
        if let Error::FailedWhileProcessingBufChange(original) = err {
            *original
        } else {
            editor_listener::Error::NeovimMessageDecodingFailed(err)
        }
    }
}

impl From<editor_listener::Error> for Error {
    fn from(err: editor_listener::Error) -> Error {
        Error::FailedWhileProcessingBufChange(Box::new(err))
    }
}

impl From<DecodingError> for Error {
    fn from(err: DecodingError) -> Error {
        Error::DecodingFailed(err)
    }
}

impl From<rmp::encode::ValueWriteError> for Error {
    fn from(error: rmp::encode::ValueWriteError) -> Error {
        match error {
            rmp::encode::ValueWriteError::InvalidMarkerWrite(sub_error) => {
                Error::EncodingFailedWhileWritingMarker(sub_error)
            }
            rmp::encode::ValueWriteError::InvalidDataWrite(sub_error) => {
                Error::EncodingFailedWhileWritingData(sub_error)
            }
        }
    }
}

impl From<rmp::decode::ValueReadError> for Error {
    fn from(error: rmp::decode::ValueReadError) -> Error {
        Error::DecodingFailed(error.into())
    }
}

impl From<rmp::decode::NumValueReadError> for Error {
    fn from(error: rmp::decode::NumValueReadError) -> Error {
        Error::DecodingFailed(error.into())
    }
}

impl From<rmp::decode::DecodeStringError<'_>> for Error {
    fn from(error: rmp::decode::DecodeStringError) -> Error {
        Error::DecodingFailed(error.into())
    }
}

impl From<rmp::decode::MarkerReadError> for Error {
    fn from(error: rmp::decode::MarkerReadError) -> Error {
        Error::DecodingFailed(error.into())
    }
}

// Skip `count` messagepack options. If one of these objects is an array or
// map, skip its contents too.
fn skip_objects<R>(read: &mut R, count: u32) -> Result<(), DecodingError>
where
    R: Read,
{
    let mut count = count;
    while count > 0 {
        count -= 1;
        let marker = rmp::decode::read_marker(read)?;
        count += skip_one_object(read, marker)
            .map_err(DecodingError::SkippingData)?;
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
        rmp::Marker::U8 => skip_bytes(read, 1)?,
        rmp::Marker::U16 => skip_bytes(read, 2)?,
        rmp::Marker::U32 => skip_bytes(read, 4)?,
        rmp::Marker::U64 => skip_bytes(read, 8)?,
        rmp::Marker::I8 => skip_bytes(read, 1)?,
        rmp::Marker::I16 => skip_bytes(read, 2)?,
        rmp::Marker::I32 => skip_bytes(read, 4)?,
        rmp::Marker::I64 => skip_bytes(read, 8)?,
        rmp::Marker::F32 => skip_bytes(read, 4)?,
        rmp::Marker::F64 => skip_bytes(read, 8)?,
        rmp::Marker::FixStr(bytes) => skip_bytes(read, bytes as u64)?,
        rmp::Marker::Str8 => {
            let bytes = read.read_u8()?;
            skip_bytes(read, bytes as u64)?;
        }
        rmp::Marker::Str16 => {
            let bytes = read.read_u16::<byteorder::BigEndian>()?;
            skip_bytes(read, bytes as u64)?
        }
        rmp::Marker::Str32 => {
            let bytes = read.read_u32::<byteorder::BigEndian>()?;
            skip_bytes(read, bytes as u64)?
        }
        rmp::Marker::Bin8 => {
            let bytes = read.read_u8()?;
            skip_bytes(read, bytes as u64)?
        }
        rmp::Marker::Bin16 => {
            let bytes = read.read_u16::<byteorder::BigEndian>()?;
            skip_bytes(read, bytes as u64)?
        }
        rmp::Marker::Bin32 => {
            let bytes = read.read_u32::<byteorder::BigEndian>()?;
            skip_bytes(read, bytes as u64)?
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
        rmp::Marker::FixExt1 => skip_bytes(read, 2)?,
        rmp::Marker::FixExt2 => skip_bytes(read, 3)?,
        rmp::Marker::FixExt4 => skip_bytes(read, 5)?,
        rmp::Marker::FixExt8 => skip_bytes(read, 9)?,
        rmp::Marker::FixExt16 => skip_bytes(read, 17)?,
        rmp::Marker::Ext8 => {
            let bytes = read.read_u8()?;
            skip_bytes(read, 1 + bytes as u64)?
        }
        rmp::Marker::Ext16 => {
            let bytes = read.read_u16::<byteorder::BigEndian>()?;
            skip_bytes(read, 1 + bytes as u64)?
        }
        rmp::Marker::Ext32 => {
            let bytes = read.read_u32::<byteorder::BigEndian>()?;
            skip_bytes(read, 1 + bytes as u64)?
        }
        rmp::Marker::Reserved => {}
    }
    Ok(0)
}

fn skip_bytes<R>(read: &mut R, count: u64) -> Result<(), std::io::Error>
where
    R: Read,
{
    std::io::copy(&mut read.take(count), &mut std::io::sink())?;
    Ok(())
}

fn to_utf8(buffer: &[u8]) -> Result<&str, DecodingError> {
    let str =
        std::str::from_utf8(buffer).map_err(DecodingError::InvalidUtf8)?;
    Ok(str)
}

// Reads chunks of string slices of a reader. Used to copy bits of a reader
// somewhere else without needing intermediate heap allocation.
fn read_chunks<R, F, G, E>(
    mut read: R,
    len: usize,
    on_error: G,
    mut on_chunk: F,
) -> Result<(), E>
where
    R: Read,
    F: FnMut(&str) -> Result<(), E>,
    G: Fn(std::io::Error) -> E,
{
    let mut bytes_remaining = len;
    let mut buffer_offset = 0;
    // The size of the buffer is small as to avoid overflowing the stack, but
    // large enough to contain a single line of code (our typical read load).
    // That way most typical payloads are moved in one iteration.
    let mut buffer = [0u8; 100];
    while bytes_remaining > 0 {
        let chunk_size = std::cmp::min(buffer.len(), bytes_remaining);
        let write_slice = &mut buffer[buffer_offset..chunk_size];
        read.read_exact(write_slice).map_err(&on_error)?;
        let str = match std::str::from_utf8_mut(&mut buffer[0..chunk_size]) {
            Ok(str) => str,
            Err(utf8_error) => {
                let good_bytes = utf8_error.valid_up_to();
                unsafe {
                    std::str::from_utf8_unchecked_mut(
                        &mut buffer[0..good_bytes],
                    )
                }
            }
        };
        let actual_chunk_size = str.len();
        bytes_remaining -= actual_chunk_size;
        on_chunk(str)?;
        let bad_bytes = actual_chunk_size - chunk_size;
        buffer_offset = 0;
        while buffer_offset < bad_bytes {
            buffer[buffer_offset] = buffer[actual_chunk_size + buffer_offset];
            buffer_offset += 1;
        }
    }
    Ok(())
}

fn read_buf<R>(read: &mut R) -> Result<usize, DecodingError>
where
    R: Read,
{
    let (_, buf) = rmp::decode::read_fixext1(read)?;
    Ok(buf as usize)
}

pub(crate) struct NeovimDriver<W> {
    write: Arc<Mutex<W>>,
}

impl<W> analysis::EditorDriver for NeovimDriver<W>
where
    W: 'static + Write + Send,
{
    fn apply_edits(&self, refactor: Vec<Edit>) -> bool {
        println!("REFACTOR: {:?}", refactor);
        let mut write_guard = crate::lock(&self.write);
        let write = write_guard.deref_mut();
        match write_refactor(write, refactor) {
            Ok(()) => true,
            Err(err) => {
                eprintln!("Ran into non-fatal error while attempting to send edits to neovim: {:?}", err );
                false
            }
        }
    }
}

fn write_refactor<W>(write: &mut W, refactor: Vec<Edit>) -> Result<(), Error>
where
    W: Write,
{
    rmp::encode::write_array_len(write, 3)?; // msgpack envelope
    rmp::encode::write_i8(write, 2)?;
    write_str(write, "nvim_call_atomic")?;

    rmp::encode::write_array_len(write, 1)?; // nvim_call_atomic args

    rmp::encode::write_array_len(write, refactor.len() as u32)?; // calls array
    let buf = 0; // TODO: use a real value here.
    for edit in refactor {
        let start = edit.input_edit.start_position;
        let end = edit.input_edit.old_end_position;

        rmp::encode::write_array_len(write, 2)?; // call tuple
        write_str(write, "nvim_buf_set_text")?;

        rmp::encode::write_array_len(write, 6)?; // nvim_buf_set_text args
        rmp::encode::write_u8(write, buf)?;
        rmp::encode::write_u64(write, start.row as u64)?;
        rmp::encode::write_u64(write, start.column as u64)?;
        rmp::encode::write_u64(write, end.row as u64)?;
        rmp::encode::write_u64(write, end.column as u64)?;

        rmp::encode::write_array_len(write, 1)?; // array of lines
        write_str(write, &edit.new_bytes)?;
    }
    Ok(())
}

fn write_str<W>(write: &mut W, str: &str) -> Result<(), Error>
where
    W: Write,
{
    let bytes = str.as_bytes();
    rmp::encode::write_str_len(write, bytes.len() as u32)?;
    write
        .write_all(bytes)
        .map_err(Error::EncodingFailedWhileWritingString)?;
    Ok(())
}

mod messagepack {
    #[derive(Debug)]
    pub(crate) enum DecodingError {
        ReadingMarker(std::io::Error),
        ReadingData(std::io::Error),
        SkippingData(std::io::Error),
        ReadingString(std::io::Error),
        TypeMismatch(rmp::Marker),
        OutOfRange,
        InvalidUtf8(core::str::Utf8Error),
        BufferCannotHoldString(u32),
        NotEnoughArrayElements { expected: u32, actual: u32 },
    }

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
                    DecodingError::NotEnoughArrayElements {
                        actual: array_len,
                        expected: expected_len
                    }.into()
                )
            }
            $( let $name = $x; )*
            skip_objects($read, array_len - expected_len)?;
        };
    }
    pub use read_tuple;

    impl From<rmp::decode::MarkerReadError> for DecodingError {
        fn from(
            rmp::decode::MarkerReadError(error): rmp::decode::MarkerReadError,
        ) -> DecodingError {
            DecodingError::ReadingMarker(error)
        }
    }

    impl From<rmp::decode::ValueReadError> for DecodingError {
        fn from(error: rmp::decode::ValueReadError) -> DecodingError {
            match error {
                rmp::decode::ValueReadError::InvalidMarkerRead(sub_error) => {
                    DecodingError::ReadingMarker(sub_error)
                }
                rmp::decode::ValueReadError::InvalidDataRead(sub_error) => {
                    DecodingError::ReadingData(sub_error)
                }
                rmp::decode::ValueReadError::TypeMismatch(sub_error) => {
                    DecodingError::TypeMismatch(sub_error)
                }
            }
        }
    }

    impl From<rmp::decode::NumValueReadError> for DecodingError {
        fn from(error: rmp::decode::NumValueReadError) -> DecodingError {
            match error {
                rmp::decode::NumValueReadError::InvalidMarkerRead(
                    sub_error,
                ) => DecodingError::ReadingMarker(sub_error),
                rmp::decode::NumValueReadError::InvalidDataRead(sub_error) => {
                    DecodingError::ReadingData(sub_error)
                }
                rmp::decode::NumValueReadError::TypeMismatch(sub_error) => {
                    DecodingError::TypeMismatch(sub_error)
                }
                rmp::decode::NumValueReadError::OutOfRange => {
                    DecodingError::OutOfRange
                }
            }
        }
    }

    impl From<rmp::decode::DecodeStringError<'_>> for DecodingError {
        fn from(error: rmp::decode::DecodeStringError) -> DecodingError {
            match error {
                rmp::decode::DecodeStringError::InvalidMarkerRead(
                    sub_error,
                ) => DecodingError::ReadingMarker(sub_error),
                rmp::decode::DecodeStringError::InvalidDataRead(sub_error) => {
                    DecodingError::ReadingData(sub_error)
                }
                rmp::decode::DecodeStringError::TypeMismatch(sub_error) => {
                    DecodingError::TypeMismatch(sub_error)
                }
                rmp::decode::DecodeStringError::BufferSizeTooSmall(
                    sub_error,
                ) => DecodingError::BufferCannotHoldString(sub_error),
                rmp::decode::DecodeStringError::InvalidUtf8(_, sub_error) => {
                    DecodingError::InvalidUtf8(sub_error)
                }
            }
        }
    }
}
