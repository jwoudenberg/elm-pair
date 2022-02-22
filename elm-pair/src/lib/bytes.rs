use crate::lib::log;
use crate::lib::log::Error;
use std::io::{Read, Write};

// TODO: use custom error type here to force more specific errors higher up.

pub fn read_u8<R: Read>(read: &mut R) -> Result<u8, Error> {
    let mut buffer = [0; 1];
    read.read_exact(&mut buffer)
        .map_err(|err| log::mk_err!("could not read u8: {:?}", err))?;
    Ok(buffer[0])
}

pub fn read_u32<R: Read>(read: &mut R) -> Result<u32, Error> {
    let mut buffer = [0; 4];
    read.read_exact(&mut buffer)
        .map_err(|err| log::mk_err!("could not read u32: {:?}", err))?;
    Ok(std::primitive::u32::from_be_bytes(buffer))
}

pub fn read_string<R: Read>(read: &mut R, len: usize) -> Result<String, Error> {
    let mut buffer = vec![0; len];
    read.read_exact(&mut buffer)
        .map_err(|err| log::mk_err!("failed reading string: {:?}", err))?;
    std::string::String::from_utf8(buffer).map_err(|err| {
        log::mk_err!("failed to parse string as utf8: {:?}", err)
    })
}

// Reads chunks of string slices of a reader. Used to copy bits of a reader
// somewhere else without needing intermediate heap allocation.
pub fn read_chunks<R, F, G, E>(
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

pub fn write_u32<W: Write>(write: &mut W, n: u32) -> Result<(), Error> {
    write
        .write_all(&std::primitive::u32::to_be_bytes(n))
        .map_err(|err| log::mk_err!("failed to write u32: {:?}", err))
}

pub fn skip<R>(read: &mut R, count: u64) -> Result<(), std::io::Error>
where
    R: Read,
{
    std::io::copy(&mut read.take(count), &mut std::io::sink())?;
    Ok(())
}
