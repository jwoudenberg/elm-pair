use std::sync::mpsc::SendError;

// TODO: Add timestamps to logs

#[macro_export]
macro_rules! mk_err {
    ( $($args:tt)* ) => {{
        crate::support::log::Error(
            format!(
                "[{}:{:?}] {}",
                std::file!(),
                std::line!(),
                format!($($args)*)
            )
        )
    }};
}
pub use mk_err;

#[macro_export]
macro_rules! info {
    ( $str:literal ) => {{
        eprintln!("[info] {}", $str);
    }};
    ( $str:literal, $($args:tt)* ) => {{
        eprintln!(concat!("[info] ", $str), $($args)*);
    }};
}
pub use info;

#[macro_export]
macro_rules! error {
    ( $str:literal ) => {{
        eprintln!("[error] {}", $str);
    }};
    ( $str:literal, $($args:tt)* ) => {{
        eprintln!(concat!("[error] ", $str), $($args)*);
    }};
}
pub use error;

pub struct Error(pub String);

impl std::fmt::Debug for Error {
    fn fmt(
        &self,
        f: &mut std::fmt::Formatter<'_>,
    ) -> Result<(), std::fmt::Error> {
        let Error(msg) = self;
        f.write_str(msg)
    }
}

impl From<rmp::encode::ValueWriteError> for Error {
    fn from(error: rmp::encode::ValueWriteError) -> Error {
        mk_err!("failed writing msgpack value: {:?}", error)
    }
}

impl From<rmp::decode::MarkerReadError> for Error {
    fn from(error: rmp::decode::MarkerReadError) -> Error {
        mk_err!("failed reading msgpack marker: {:?}", error)
    }
}

impl From<rmp::decode::ValueReadError> for Error {
    fn from(error: rmp::decode::ValueReadError) -> Error {
        mk_err!("failed reading msgpack value: {:?}", error)
    }
}

impl From<rmp::decode::NumValueReadError> for Error {
    fn from(error: rmp::decode::NumValueReadError) -> Error {
        mk_err!("failed reading msgpack numerical value: {:?}", error)
    }
}

impl From<rmp::decode::DecodeStringError<'_>> for Error {
    fn from(error: rmp::decode::DecodeStringError) -> Error {
        mk_err!("failed reading msgpack string: {:?}", error)
    }
}

impl<T> From<SendError<T>> for Error {
    fn from(err: SendError<T>) -> Error {
        mk_err!("failed sending message to channel: {:?}", err)
    }
}
