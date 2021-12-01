use crate::{Edit, EditorSourceChange, Error, InputEdit, SourceFileSnapshot};
use byteorder::ReadBytesExt;
use ropey::RopeBuilder;
use std::io::{BufReader, Read, Write};
use std::ops::DerefMut;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub(crate) struct Neovim<R, W, P> {
    read: BufReader<R>,
    write: Arc<Mutex<W>>,
    apply_buf_change: P,
}

impl<R: Read, W: Write, P: FnMut(BufChange) -> Result<(), Error>>
    Neovim<R, W, P>
{
    pub fn new(read: R, write: W, apply_buf_change: P) -> Self
    where
        R: Read,
    {
        Neovim {
            read: BufReader::new(read),
            write: Arc::new(Mutex::new(write)),
            apply_buf_change,
        }
    }

    pub fn driver(&self) -> NeovimDriver<W> {
        NeovimDriver {
            write: self.write.clone(),
        }
    }

    pub fn start(mut self) -> Result<(), Error> {
        // TODO: figure out how to stop this loop when we the reader closes.
        loop {
            self.parse_msg()?;
        }
    }

    // Messages we receive from neovim's webpack-rpc API:
    // neovim api:  https://neovim.io/doc/user/api.html
    // webpack-rpc: https://github.com/msgpack-rpc/msgpack-rpc/blob/master/spec.md
    //
    // TODO handle neovim API versions
    fn parse_msg(&mut self) -> Result<(), Error> {
        let array_len =
            rmp::decode::read_array_len(&mut self.read).map_err(val_err)?;
        let type_ =
            rmp::decode::read_int(&mut self.read).map_err(num_val_err)?;
        if array_len == 3 && type_ == 2 {
            self.parse_notification_msg()
        } else {
            decoding_error(DecodingError::UnknownMessageType(array_len, type_))
        }
    }

    fn parse_notification_msg(&mut self) -> Result<(), Error> {
        let mut buffer = [0u8; 30];
        // TODO: Don't parse to UTF8 here. Compare bytestrings instead.
        let method = rmp::decode::read_str(&mut self.read, &mut buffer)
            .map_err(str_err)?;
        match method {
            "nvim_error_event" => self.parse_error_event(),
            "nvim_buf_lines_event" => self.parse_buf_lines_event(),
            "nvim_buf_changedtick_event" => self.parse_buf_changedtick_event(),
            "nvim_buf_detach_event" => self.parse_buf_detach_event(),
            "buffer_opened" => self.parse_buffer_opened(),
            method => decoding_error(DecodingError::UnknownEventMethod(
                method.to_owned(),
            )),
        }
    }

    fn parse_error_event(&mut self) -> Result<(), Error> {
        let array_len =
            rmp::decode::read_array_len(&mut self.read).map_err(val_err)?;
        if array_len < 2 {
            return decoding_error(
                DecodingError::NotEnoughArgsInBufLinesEvent(array_len),
            );
        }
        let type_ =
            rmp::decode::read_int(&mut self.read).map_err(num_val_err)?;
        let msg = read_string(&mut self.read)?;
        skip_objects(&mut self.read, array_len - 2)?;

        decoding_error(DecodingError::ReceivedErrorEvent(type_, msg))
    }

    fn parse_buffer_opened(&mut self) -> Result<(), Error> {
        let array_len =
            rmp::decode::read_array_len(&mut self.read).map_err(val_err)?;
        if array_len < 2 {
            return decoding_error(
                DecodingError::NotEnoughArgsInBufLinesEvent(array_len),
            );
        }
        let buf = rmp::decode::read_int(&mut self.read).map_err(num_val_err)?;
        skip_objects(&mut self.read, array_len - 1)?;
        self.nvim_buf_attach(buf)
    }

    fn parse_buf_lines_event(&mut self) -> Result<(), Error> {
        let array_len =
            rmp::decode::read_array_len(&mut self.read).map_err(val_err)?;
        if array_len < 6 {
            return decoding_error(
                DecodingError::NotEnoughArgsInBufLinesEvent(array_len),
            );
        }
        let buf = read_buf(&mut self.read)?;
        let changedtick =
            rmp::decode::read_int(&mut self.read).map_err(num_val_err)?;
        let firstline =
            rmp::decode::read_int(&mut self.read).map_err(num_val_err)?;
        let lastline =
            rmp::decode::read_int(&mut self.read).map_err(num_val_err)?;
        let mut line_count =
            rmp::decode::read_array_len(&mut self.read).map_err(val_err)?;
        let mut linedata = Vec::with_capacity(line_count as usize);
        while line_count > 0 {
            line_count -= 1;
            linedata.push(read_string(&mut self.read)?);
        }
        // I'm not using the `more` argument for anything.
        let _more = rmp::decode::read_bool(&mut self.read).map_err(val_err)?;
        let extra_args = array_len - 6;
        skip_objects(&mut self.read, extra_args)?;
        (self.apply_buf_change)(BufChange {
            _buf: buf as u64,
            changedtick,
            firstline,
            lastline,
            linedata,
        })
    }

    fn parse_buf_changedtick_event(&mut self) -> Result<(), Error> {
        // We're not interested in these events, so we skip them.
        let array_len =
            rmp::decode::read_array_len(&mut self.read).map_err(val_err)?;
        skip_objects(&mut self.read, array_len)?;
        Ok(())
    }

    fn parse_buf_detach_event(&mut self) -> Result<(), Error> {
        // Re-attach this buffer
        // TODO: consider when we might not want to reattach.
        let array_len =
            rmp::decode::read_array_len(&mut self.read).map_err(val_err)?;
        if array_len < 1 {
            return decoding_error(
                DecodingError::NotEnoughArgsInBufLinesEvent(array_len),
            );
        }
        let buf = read_buf(&mut self.read)?;
        skip_objects(&mut self.read, array_len - 1)?;
        self.nvim_buf_attach(buf)
    }

    fn nvim_buf_attach(&self, buf: u8) -> Result<(), Error> {
        let mut write_guard = crate::lock(&self.write);
        let write = write_guard.deref_mut();
        rmp::encode::write_array_len(write, 3).unwrap();
        rmp::encode::write_i8(write, 2).unwrap();
        write_str(write, "nvim_buf_attach");
        // nvim_buf_attach arguments
        rmp::encode::write_array_len(write, 3).unwrap();
        rmp::encode::write_u8(write, buf).unwrap(); //buf
        rmp::encode::write_bool(write, true).unwrap(); // send_buffer
        rmp::encode::write_map_len(write, 0).unwrap(); // opts
        Ok(())
    }
}

pub struct BufChange {
    _buf: u64,
    changedtick: u64,
    firstline: i64,
    lastline: i64,
    linedata: Vec<String>,
}

impl EditorSourceChange for BufChange {
    fn apply(
        &self,
        opt_code: &mut Option<SourceFileSnapshot>,
    ) -> Option<InputEdit> {
        match (self.lastline, opt_code) {
            (-1, code) => {
                let mut builder = RopeBuilder::new();
                for line in &self.linedata {
                    builder.append(line);
                    builder.append("\n");
                }
                let bytes = builder.finish();
                *code = Some(SourceFileSnapshot {
                    tree: crate::parse(None, &bytes).unwrap(),
                    bytes,
                    revision: self.changedtick as usize,
                    file_data: Arc::new(crate::FileData {
                        // TODO: put real data here.
                        path: PathBuf::new(),
                        project_root: PathBuf::from(
                            "/home/jasper/dev/elm-pair/tests",
                        ),
                        elm_bin: PathBuf::from("elm"),
                    }),
                });
                None
            }
            (_, None) => panic!("incremental update for unknown code."),
            (lastline, Some(code)) => {
                let start_line = self.firstline as usize;
                let old_end_line = lastline as usize;
                let start_char = code.bytes.line_to_char(start_line);
                let start_byte = code.bytes.line_to_byte(start_line);
                let old_end_char = code.bytes.line_to_char(old_end_line);
                let old_end_byte = code.bytes.line_to_byte(old_end_line);
                let mut new_end_byte = start_byte;
                code.bytes.remove(start_char..old_end_char);
                for line in &self.linedata {
                    code.bytes.insert(start_char, line.as_str());
                    new_end_byte += line.len();
                    let new_end_char = code.bytes.byte_to_char(new_end_byte);
                    code.bytes.insert_char(new_end_char, '\n');
                    new_end_byte += 1;
                }
                code.revision = self.changedtick as usize;
                Some(InputEdit {
                    start_byte,
                    old_end_byte,
                    new_end_byte,
                    start_position: crate::byte_to_point(code, start_byte),
                    old_end_position: crate::byte_to_point(code, old_end_byte),
                    new_end_position: crate::byte_to_point(code, new_end_byte),
                })
            }
        }
    }
}

#[derive(Debug)]
pub enum DecodingError {
    DecodingFailedWithInvalidMarkerRead(rmp::decode::Error),
    DecodingFailedWithInvalidDataRead(rmp::decode::Error),
    DecodingFailedWithTypeMismatch(rmp::Marker),
    DecodingFailedWithOutOfRange,
    DecodingFailedWithInvalidUtf8(core::str::Utf8Error),
    DecodingFailedWithBufferSizeTooSmall(u32),
    DecodingFailedWhileSkipping(std::io::Error),
    UnknownMessageType(u32, u8),
    UnknownEventMethod(String),
    NotEnoughArgsInBufLinesEvent(u32),
    ReceivedErrorEvent(u64, String),
}

fn decoding_error(err: DecodingError) -> Result<(), Error> {
    Err(Error::NeovimMessageDecodingFailed(err))
}

// Skip `count` messagepack options. If one of these objects is an array or
// map, skip its contents too.
fn skip_objects<R>(read: &mut BufReader<R>, count: u32) -> Result<(), Error>
where
    R: Read,
{
    let mut count = count;
    while count > 0 {
        count -= 1;
        let marker = rmp::decode::read_marker(read).map_err(marker_err)?;
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
                let bytes = read
                    .read_u8()
                    .map_err(DecodingError::DecodingFailedWhileSkipping)
                    .map_err(Error::NeovimMessageDecodingFailed)?;
                skip_bytes(read, bytes as u64)?
            }
            rmp::Marker::Str16 => {
                let bytes = read
                    .read_u16::<byteorder::BigEndian>()
                    .map_err(DecodingError::DecodingFailedWhileSkipping)
                    .map_err(Error::NeovimMessageDecodingFailed)?;
                skip_bytes(read, bytes as u64)?
            }
            rmp::Marker::Str32 => {
                let bytes = read
                    .read_u32::<byteorder::BigEndian>()
                    .map_err(DecodingError::DecodingFailedWhileSkipping)
                    .map_err(Error::NeovimMessageDecodingFailed)?;
                skip_bytes(read, bytes as u64)?
            }
            rmp::Marker::Bin8 => {
                let bytes = read
                    .read_u8()
                    .map_err(DecodingError::DecodingFailedWhileSkipping)
                    .map_err(Error::NeovimMessageDecodingFailed)?;
                skip_bytes(read, bytes as u64)?
            }
            rmp::Marker::Bin16 => {
                let bytes = read
                    .read_u16::<byteorder::BigEndian>()
                    .map_err(DecodingError::DecodingFailedWhileSkipping)
                    .map_err(Error::NeovimMessageDecodingFailed)?;
                skip_bytes(read, bytes as u64)?
            }
            rmp::Marker::Bin32 => {
                let bytes = read
                    .read_u32::<byteorder::BigEndian>()
                    .map_err(DecodingError::DecodingFailedWhileSkipping)
                    .map_err(Error::NeovimMessageDecodingFailed)?;
                skip_bytes(read, bytes as u64)?
            }
            rmp::Marker::FixArray(objects) => {
                count += objects as u32;
            }
            rmp::Marker::Array16 => {
                let objects = read
                    .read_u16::<byteorder::BigEndian>()
                    .map_err(DecodingError::DecodingFailedWhileSkipping)
                    .map_err(Error::NeovimMessageDecodingFailed)?;
                count += objects as u32;
            }
            rmp::Marker::Array32 => {
                let objects = read
                    .read_u32::<byteorder::BigEndian>()
                    .map_err(DecodingError::DecodingFailedWhileSkipping)
                    .map_err(Error::NeovimMessageDecodingFailed)?;
                count += objects;
            }
            rmp::Marker::FixMap(entries) => {
                count += 2 * entries as u32;
            }
            rmp::Marker::Map16 => {
                let entries = read
                    .read_u16::<byteorder::BigEndian>()
                    .map_err(DecodingError::DecodingFailedWhileSkipping)
                    .map_err(Error::NeovimMessageDecodingFailed)?;
                count += 2 * entries as u32;
            }
            rmp::Marker::Map32 => {
                let entries = read
                    .read_u32::<byteorder::BigEndian>()
                    .map_err(DecodingError::DecodingFailedWhileSkipping)
                    .map_err(Error::NeovimMessageDecodingFailed)?;
                count += 2 * entries;
            }
            rmp::Marker::FixExt1 => skip_bytes(read, 2)?,
            rmp::Marker::FixExt2 => skip_bytes(read, 3)?,
            rmp::Marker::FixExt4 => skip_bytes(read, 5)?,
            rmp::Marker::FixExt8 => skip_bytes(read, 9)?,
            rmp::Marker::FixExt16 => skip_bytes(read, 17)?,
            rmp::Marker::Ext8 => {
                let bytes = read
                    .read_u8()
                    .map_err(DecodingError::DecodingFailedWhileSkipping)
                    .map_err(Error::NeovimMessageDecodingFailed)?;
                skip_bytes(read, 1 + bytes as u64)?
            }
            rmp::Marker::Ext16 => {
                let bytes = read
                    .read_u16::<byteorder::BigEndian>()
                    .map_err(DecodingError::DecodingFailedWhileSkipping)
                    .map_err(Error::NeovimMessageDecodingFailed)?;
                skip_bytes(read, 1 + bytes as u64)?
            }
            rmp::Marker::Ext32 => {
                let bytes = read
                    .read_u32::<byteorder::BigEndian>()
                    .map_err(DecodingError::DecodingFailedWhileSkipping)
                    .map_err(Error::NeovimMessageDecodingFailed)?;
                skip_bytes(read, 1 + bytes as u64)?
            }
            rmp::Marker::Reserved => {}
        }
    }
    Ok(())
}

fn skip_bytes<R>(read: &mut BufReader<R>, count: u64) -> Result<(), Error>
where
    R: Read,
{
    std::io::copy(&mut read.take(count), &mut std::io::sink())
        .map_err(DecodingError::DecodingFailedWhileSkipping)
        .map_err(Error::NeovimMessageDecodingFailed)?;
    Ok(())
}

fn read_string<R>(read: &mut BufReader<R>) -> Result<String, Error>
where
    R: Read,
{
    let len = rmp::decode::read_str_len(read).map_err(val_err)?;
    let mut buffer = vec![0; len as usize];
    read.read_exact(&mut buffer)
        .map_err(DecodingError::DecodingFailedWhileSkipping)
        .map_err(Error::NeovimMessageDecodingFailed)?;
    std::string::String::from_utf8(buffer).map_err(|err| {
        Error::NeovimMessageDecodingFailed(
            DecodingError::DecodingFailedWithInvalidUtf8(err.utf8_error()),
        )
    })
}

fn read_buf<R>(read: &mut BufReader<R>) -> Result<u8, Error>
where
    R: Read,
{
    let (_, buf) = rmp::decode::read_fixext1(read).map_err(val_err)?;
    Ok(buf as u8)
}

fn marker_err(error: rmp::decode::MarkerReadError) -> Error {
    match error {
        rmp::decode::MarkerReadError(sub_error) => {
            Error::NeovimMessageDecodingFailed(
                DecodingError::DecodingFailedWithInvalidMarkerRead(sub_error),
            )
        }
    }
}

fn val_err(error: rmp::decode::ValueReadError) -> Error {
    match error {
        rmp::decode::ValueReadError::InvalidMarkerRead(sub_error) => {
            Error::NeovimMessageDecodingFailed(
                DecodingError::DecodingFailedWithInvalidMarkerRead(sub_error),
            )
        }
        rmp::decode::ValueReadError::InvalidDataRead(sub_error) => {
            Error::NeovimMessageDecodingFailed(
                DecodingError::DecodingFailedWithInvalidDataRead(sub_error),
            )
        }
        rmp::decode::ValueReadError::TypeMismatch(sub_error) => {
            Error::NeovimMessageDecodingFailed(
                DecodingError::DecodingFailedWithTypeMismatch(sub_error),
            )
        }
    }
}

fn num_val_err(error: rmp::decode::NumValueReadError) -> Error {
    match error {
        rmp::decode::NumValueReadError::InvalidMarkerRead(sub_error) => {
            Error::NeovimMessageDecodingFailed(
                DecodingError::DecodingFailedWithInvalidMarkerRead(sub_error),
            )
        }
        rmp::decode::NumValueReadError::InvalidDataRead(sub_error) => {
            Error::NeovimMessageDecodingFailed(
                DecodingError::DecodingFailedWithInvalidDataRead(sub_error),
            )
        }
        rmp::decode::NumValueReadError::TypeMismatch(sub_error) => {
            Error::NeovimMessageDecodingFailed(
                DecodingError::DecodingFailedWithTypeMismatch(sub_error),
            )
        }
        rmp::decode::NumValueReadError::OutOfRange => {
            Error::NeovimMessageDecodingFailed(
                DecodingError::DecodingFailedWithOutOfRange,
            )
        }
    }
}

fn str_err(error: rmp::decode::DecodeStringError) -> Error {
    match error {
        rmp::decode::DecodeStringError::InvalidMarkerRead(sub_error) => {
            Error::NeovimMessageDecodingFailed(
                DecodingError::DecodingFailedWithInvalidMarkerRead(sub_error),
            )
        }
        rmp::decode::DecodeStringError::InvalidDataRead(sub_error) => {
            Error::NeovimMessageDecodingFailed(
                DecodingError::DecodingFailedWithInvalidDataRead(sub_error),
            )
        }
        rmp::decode::DecodeStringError::TypeMismatch(sub_error) => {
            Error::NeovimMessageDecodingFailed(
                DecodingError::DecodingFailedWithTypeMismatch(sub_error),
            )
        }
        rmp::decode::DecodeStringError::BufferSizeTooSmall(sub_error) => {
            Error::NeovimMessageDecodingFailed(
                DecodingError::DecodingFailedWithBufferSizeTooSmall(sub_error),
            )
        }
        rmp::decode::DecodeStringError::InvalidUtf8(_, sub_error) => {
            Error::NeovimMessageDecodingFailed(
                DecodingError::DecodingFailedWithInvalidUtf8(sub_error),
            )
        }
    }
}

pub struct NeovimDriver<W> {
    write: Arc<Mutex<W>>,
}

impl<W> NeovimDriver<W>
where
    W: Write,
{
    pub(crate) fn apply_edits(&self, refactor: Vec<Edit>) -> Result<(), Error> {
        println!("REFACTOR: {:?}", refactor);
        let mut write_guard = crate::lock(&self.write);
        let write = write_guard.deref_mut();

        rmp::encode::write_array_len(write, 3).unwrap(); // msgpack envelope
        rmp::encode::write_i8(write, 2).unwrap();
        write_str(write, "nvim_call_atomic");

        rmp::encode::write_array_len(write, 1).unwrap(); // nvim_call_atomic args

        rmp::encode::write_array_len(write, refactor.len() as u32).unwrap(); // calls array
        let buf = 0; // TODO: use a real value here.
        for edit in refactor {
            let start = edit.input_edit.start_position;
            let end = edit.input_edit.old_end_position;

            rmp::encode::write_array_len(write, 2).unwrap(); // call tuple
            write_str(write, "nvim_buf_set_text");

            rmp::encode::write_array_len(write, 6).unwrap(); // nvim_buf_set_text args
            rmp::encode::write_u8(write, buf).unwrap();
            rmp::encode::write_u64(write, start.row as u64).unwrap();
            rmp::encode::write_u64(write, start.column as u64).unwrap();
            rmp::encode::write_u64(write, end.row as u64).unwrap();
            rmp::encode::write_u64(write, end.column as u64).unwrap();

            rmp::encode::write_array_len(write, 1).unwrap(); // array of lines
            write_str(write, &edit.new_bytes);
        }
        Ok(())
    }
}

pub fn write_str<W>(write: &mut W, str: &str)
where
    W: Write,
{
    let bytes = str.as_bytes();
    rmp::encode::write_str_len(write, bytes.len() as u32).unwrap();
    write.write_all(bytes).unwrap();
}
