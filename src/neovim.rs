use crate::{Edit, EditorSourceChange, InputEdit};
use byteorder::ReadBytesExt;
use ropey::{Rope, RopeBuilder};
use std::io::{BufReader, Read, Write};
use std::ops::DerefMut;
use std::sync::{Arc, Mutex};

pub(crate) struct Neovim<R, W> {
    read: BufReader<R>,
    write: Arc<Mutex<W>>,
}

impl<R: Read, W: Write> Neovim<R, W> {
    pub fn new(read: R, write: W) -> Self
    where
        R: Read,
    {
        Neovim {
            read: BufReader::new(read),
            write: Arc::new(Mutex::new(write)),
        }
    }
}

impl<R: Read, W: Write> crate::Editor for Neovim<R, W> {
    type Driver = NeovimDriver<W>;
    type SourceChange = BufChange;

    fn driver(&self) -> NeovimDriver<W> {
        NeovimDriver {
            write: self.write.clone(),
        }
    }

    fn listen<P>(self, on_buf_change: P) -> Result<(), crate::Error>
    where
        P: FnMut(BufChange) -> Result<(), crate::Error>,
    {
        let mut listener = NeovimListener {
            read: self.read,
            write: self.write,
            on_buf_change,
        };
        while listener.parse_msg()? {}
        Ok(())
    }
}

struct NeovimListener<R, W, P> {
    read: BufReader<R>,
    write: Arc<Mutex<W>>,
    on_buf_change: P,
}

impl<R: Read, W: Write, P: FnMut(BufChange) -> Result<(), crate::Error>>
    NeovimListener<R, W, P>
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
        // TODO: Don't parse to UTF8 here. Compare bytestrings instead.
        let method = rmp::decode::read_str(&mut self.read, &mut buffer)?;
        match method {
            "nvim_error_event" => self.parse_error_event(),
            "nvim_buf_lines_event" => self.parse_buf_lines_event(),
            "nvim_buf_changedtick_event" => self.parse_buf_changedtick_event(),
            "nvim_buf_detach_event" => self.parse_buf_detach_event(),
            "buffer_opened" => self.parse_buffer_opened(),
            method => Err(Error::UnknownEventMethod(method.to_owned())),
        }
    }

    fn parse_error_event(&mut self) -> Result<(), Error> {
        let array_len = rmp::decode::read_array_len(&mut self.read)?;
        if array_len < 2 {
            return Err(Error::NotEnoughArgsInBufLinesEvent(array_len));
        }
        let type_ = rmp::decode::read_int(&mut self.read)?;
        let msg = read_string(&mut self.read)?;
        skip_objects(&mut self.read, array_len - 2)?;

        Err(Error::ReceivedErrorEvent(type_, msg))
    }

    fn parse_buffer_opened(&mut self) -> Result<(), Error> {
        let array_len = rmp::decode::read_array_len(&mut self.read)?;
        if array_len < 2 {
            return Err(Error::NotEnoughArgsInBufLinesEvent(array_len));
        }
        let buf = rmp::decode::read_int(&mut self.read)?;
        skip_objects(&mut self.read, array_len - 1)?;
        self.nvim_buf_attach(buf)
    }

    fn parse_buf_lines_event(&mut self) -> Result<(), Error> {
        let array_len = rmp::decode::read_array_len(&mut self.read)?;
        if array_len < 6 {
            return Err(Error::NotEnoughArgsInBufLinesEvent(array_len));
        }
        let buf = read_buf(&mut self.read)?;
        let changedtick = rmp::decode::read_int(&mut self.read)?;
        let firstline = rmp::decode::read_int(&mut self.read)?;
        let lastline = rmp::decode::read_int(&mut self.read)?;
        let mut line_count = rmp::decode::read_array_len(&mut self.read)?;
        let mut linedata = Vec::with_capacity(line_count as usize);
        while line_count > 0 {
            line_count -= 1;
            linedata.push(read_string(&mut self.read)?);
        }
        // I'm not using the `more` argument for anything.
        let _more = rmp::decode::read_bool(&mut self.read)?;
        let extra_args = array_len - 6;
        skip_objects(&mut self.read, extra_args)?;
        (self.on_buf_change)(BufChange {
            _buf: buf as u64,
            _changedtick: changedtick,
            firstline,
            lastline,
            linedata,
        })
        .map_err(|err| Error::FailedWhileProcessingBufChange(Box::new(err)))
    }

    fn parse_buf_changedtick_event(&mut self) -> Result<(), Error> {
        // We're not interested in these events, so we skip them.
        let array_len = rmp::decode::read_array_len(&mut self.read)?;
        skip_objects(&mut self.read, array_len)?;
        Ok(())
    }

    fn parse_buf_detach_event(&mut self) -> Result<(), Error> {
        // Re-attach this buffer
        // TODO: consider when we might not want to reattach.
        let array_len = rmp::decode::read_array_len(&mut self.read)?;
        if array_len < 1 {
            return Err(Error::NotEnoughArgsInBufLinesEvent(array_len));
        }
        let buf = read_buf(&mut self.read)?;
        skip_objects(&mut self.read, array_len - 1)?;
        self.nvim_buf_attach(buf)
    }

    fn nvim_buf_attach(&self, buf: u8) -> Result<(), Error> {
        let mut write_guard = crate::lock(&self.write);
        let write = write_guard.deref_mut();
        rmp::encode::write_array_len(write, 3)?;
        rmp::encode::write_i8(write, 2)?;
        write_str(write, "nvim_buf_attach")?;
        // nvim_buf_attach arguments
        rmp::encode::write_array_len(write, 3)?;
        rmp::encode::write_u8(write, buf)?; //buf
        rmp::encode::write_bool(write, true)
            .map_err(Error::EncodingFailedWhileWritingData)?; // send_buffer
        rmp::encode::write_map_len(write, 0)?; // opts
        Ok(())
    }
}

pub struct BufChange {
    _buf: u64,
    _changedtick: u64,
    firstline: i64,
    lastline: i64,
    linedata: Vec<String>,
}

impl EditorSourceChange for BufChange {
    fn apply_first(&self) -> Result<Option<Rope>, crate::Error> {
        if self.lastline == -1 {
            Ok(Some(rope_from_lines(&self.linedata)))
        } else {
            Err(Error::GotIncrementalUpdateBeforeFullUpdate.into())
        }
    }

    fn apply(
        &self,
        code: &mut Rope,
    ) -> Result<Option<InputEdit>, crate::Error> {
        if self.lastline == -1 {
            let old_end_byte = code.len_bytes();
            *code = rope_from_lines(&self.linedata);
            let start_byte = 0;
            let new_end_byte = code.len_bytes();
            Ok(Some(InputEdit {
                start_byte,
                old_end_byte,
                new_end_byte,
                start_position: crate::byte_to_point(code, start_byte),
                old_end_position: crate::byte_to_point(code, old_end_byte),
                new_end_position: crate::byte_to_point(code, new_end_byte),
            }))
        } else {
            let start_line = self.firstline as usize;
            let old_end_line = self.lastline as usize;
            let start_char = code.line_to_char(start_line);
            let start_byte = code.line_to_byte(start_line);
            let old_end_char = code.line_to_char(old_end_line);
            let old_end_byte = code.line_to_byte(old_end_line);
            let mut new_end_byte = start_byte;
            let old_end_position = crate::byte_to_point(code, old_end_byte);
            code.remove(start_char..old_end_char);
            for line in &self.linedata {
                code.insert(start_char, line.as_str());
                new_end_byte += line.len();
                let new_end_char = code.byte_to_char(new_end_byte);
                code.insert_char(new_end_char, '\n');
                new_end_byte += 1;
            }
            Ok(Some(InputEdit {
                start_byte,
                old_end_byte,
                new_end_byte,
                start_position: crate::byte_to_point(code, start_byte),
                old_end_position,
                new_end_position: crate::byte_to_point(code, new_end_byte),
            }))
        }
    }
}

fn rope_from_lines(lines: &[String]) -> Rope {
    let mut builder = RopeBuilder::new();
    for line in lines {
        builder.append(line);
        builder.append("\n");
    }
    builder.finish()
}

#[derive(Debug)]
pub(crate) enum Error {
    DecodingFailedWhileReadingMarker(std::io::Error),
    DecodingFailedWhileReadingData(std::io::Error),
    DecodingFailedWithTypeMismatch(rmp::Marker),
    DecodingFailedWithOutOfRange,
    DecodingFailedWithInvalidUtf8(core::str::Utf8Error),
    DecodingFailedWritingStringInTooSmallABuffer(u32),
    DecodingFailedWhileSkippingData(std::io::Error),
    DecodingFailedWhileReadingString(std::io::Error),
    EncodingFailedWhileWritingMarker(std::io::Error),
    EncodingFailedWhileWritingData(std::io::Error),
    EncodingFailedWhileWritingString(std::io::Error),
    UnknownMessageType(u32, u8),
    UnknownEventMethod(String),
    NotEnoughArgsInBufLinesEvent(u32),
    ReceivedErrorEvent(u64, String),
    GotIncrementalUpdateBeforeFullUpdate,
    FailedWhileProcessingBufChange(Box<crate::Error>),
}

impl From<Error> for crate::Error {
    fn from(err: Error) -> crate::Error {
        if let Error::FailedWhileProcessingBufChange(original) = err {
            *original
        } else {
            crate::Error::NeovimMessageDecodingFailed(err)
        }
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

impl From<rmp::decode::MarkerReadError> for Error {
    fn from(
        rmp::decode::MarkerReadError(error): rmp::decode::MarkerReadError,
    ) -> Error {
        Error::DecodingFailedWhileReadingMarker(error)
    }
}

impl From<rmp::decode::ValueReadError> for Error {
    fn from(error: rmp::decode::ValueReadError) -> Error {
        match error {
            rmp::decode::ValueReadError::InvalidMarkerRead(sub_error) => {
                Error::DecodingFailedWhileReadingMarker(sub_error)
            }
            rmp::decode::ValueReadError::InvalidDataRead(sub_error) => {
                Error::DecodingFailedWhileReadingData(sub_error)
            }
            rmp::decode::ValueReadError::TypeMismatch(sub_error) => {
                Error::DecodingFailedWithTypeMismatch(sub_error)
            }
        }
    }
}

impl From<rmp::decode::NumValueReadError> for Error {
    fn from(error: rmp::decode::NumValueReadError) -> Error {
        match error {
            rmp::decode::NumValueReadError::InvalidMarkerRead(sub_error) => {
                Error::DecodingFailedWhileReadingMarker(sub_error)
            }
            rmp::decode::NumValueReadError::InvalidDataRead(sub_error) => {
                Error::DecodingFailedWhileReadingData(sub_error)
            }
            rmp::decode::NumValueReadError::TypeMismatch(sub_error) => {
                Error::DecodingFailedWithTypeMismatch(sub_error)
            }
            rmp::decode::NumValueReadError::OutOfRange => {
                Error::DecodingFailedWithOutOfRange
            }
        }
    }
}

impl From<rmp::decode::DecodeStringError<'_>> for Error {
    fn from(error: rmp::decode::DecodeStringError) -> Error {
        match error {
            rmp::decode::DecodeStringError::InvalidMarkerRead(sub_error) => {
                Error::DecodingFailedWhileReadingMarker(sub_error)
            }
            rmp::decode::DecodeStringError::InvalidDataRead(sub_error) => {
                Error::DecodingFailedWhileReadingData(sub_error)
            }
            rmp::decode::DecodeStringError::TypeMismatch(sub_error) => {
                Error::DecodingFailedWithTypeMismatch(sub_error)
            }
            rmp::decode::DecodeStringError::BufferSizeTooSmall(sub_error) => {
                Error::DecodingFailedWritingStringInTooSmallABuffer(sub_error)
            }
            rmp::decode::DecodeStringError::InvalidUtf8(_, sub_error) => {
                Error::DecodingFailedWithInvalidUtf8(sub_error)
            }
        }
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
        count += skip_one_object(read, marker)
            .map_err(Error::DecodingFailedWhileSkippingData)?;
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

fn read_string<R>(read: &mut R) -> Result<String, Error>
where
    R: Read,
{
    let len = rmp::decode::read_str_len(read)?;
    let mut buffer = vec![0; len as usize];
    read.read_exact(&mut buffer)
        .map_err(Error::DecodingFailedWhileReadingString)?;
    std::string::String::from_utf8(buffer)
        .map_err(|err| Error::DecodingFailedWithInvalidUtf8(err.utf8_error()))
}

fn read_buf<R>(read: &mut R) -> Result<u8, Error>
where
    R: Read,
{
    let (_, buf) = rmp::decode::read_fixext1(read)?;
    Ok(buf as u8)
}

pub struct NeovimDriver<W> {
    write: Arc<Mutex<W>>,
}

impl<W> crate::EditorDriver for NeovimDriver<W>
where
    W: Write,
{
    fn apply_edits(&self, refactor: Vec<Edit>) -> Result<(), crate::Error> {
        println!("REFACTOR: {:?}", refactor);
        let mut write_guard = crate::lock(&self.write);
        let write = write_guard.deref_mut();
        write_refactor(write, refactor)?;
        Ok(())
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
