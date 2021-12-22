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
