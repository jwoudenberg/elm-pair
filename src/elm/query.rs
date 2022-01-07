#[macro_export]
macro_rules! query {
    ($name:ident, $file:literal $(, $capture:ident )* $(,)? ) => {
        pub struct $name {
            query: Query,
            $($capture: u32,)*
        }

        impl $name {
            pub fn init(lang: Language) -> Result<$name, Error> {
                let query_str = include_str!($file);
                let query = Query::new(lang, query_str).map_err(|err| {
                    log::mk_err!(
                        "failed to parse tree-sitter {}: {:?}",
                        stringify!($name),
                        err
                    )
                })?;
                let imports_query = $name {
                    $($capture: index_for_name(&query, stringify!($capture))?,)*
                    query,
                };
                Ok(imports_query)
            }
        }
    };
}
pub use query;
