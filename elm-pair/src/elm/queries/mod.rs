use crate::lib::log;
use crate::lib::log::Error;
use tree_sitter::Query;

pub mod exports;
pub mod imports;
pub mod name_definitions;
pub mod qualified_values;
pub mod scopes;
pub mod unqualified_values;

#[macro_export]
macro_rules! query {
    ($file:literal $(, $capture:ident )* $(,)? ) => {
        pub struct Query {
            query: tree_sitter::Query,
            $($capture: u32,)*
        }

        impl Query {
            pub fn init(lang: tree_sitter::Language) -> Result<Query, $crate::lib::log::Error> {
                let query_file_contents = include_str!($file);
                let separator = "=== test input below ===";
                let query_str =
                      match query_file_contents.split_once(separator) {
                          Some((query, _)) => query,
                          None => query_file_contents,
                      };
                let query = tree_sitter::Query::new(lang, query_str).map_err(|err| {
                    $crate::lib::log::mk_err!(
                        "failed to parse tree-sitter {}: {:?}",
                        stringify!(Query),
                        err
                    )
                })?;
                let query_struct = Query {
                    $($capture: $crate::elm::queries::index_for_name(&query, stringify!($capture))?,)*
                    query,
                };
                Ok(query_struct)
            }
        }

        #[cfg(test)]
        mod query_tests {
            use super::Query;

            #[test]
            fn query_sample_data() {
                let language = tree_sitter_elm::language();
                let query = Query::init(language).unwrap();
                let separator = "=== test input below ===";
                let test_str =
                      match include_str!($file).split_once(separator) {
                          Some((_, test)) => test,
                          None => panic!("No test input found in query file.")
                      };
                let mut cursor = tree_sitter::QueryCursor::new();
                let tree = $crate::lib::source_code::parse_bytes(test_str).unwrap();
                let root_node = tree.root_node();
                if root_node.has_error() {
                    panic!("Parsing resulted in invalid syntax tree.");
                }
                let capture_names = query.query.capture_names();
                let output: String =
                    cursor
                      .matches(&query.query, root_node, test_str.as_bytes())
                      .map(|m| {
                          let captures_str: String =
                                m.captures.into_iter().map(|c| {
                                    let position = c.node.start_position();
                                    format!("{}: [{}:{}] {}\n",
                                        capture_names[c.index as usize],
                                        position.row,
                                        position.column,
                                        test_str.get(c.node.byte_range()).unwrap(),
                                    )
                                }).collect();
                          format!("{}\n", captures_str)
                      })
                      .collect();
                let mut query_file_path = std::path::PathBuf::from(std::file!());
                query_file_path.pop();
                query_file_path = query_file_path.join($file);
                $crate::lib::included_answer_test::assert_eq_answer_in(
                    output.as_str(),
                    &query_file_path,
                );
            }
        }
    };
}
pub use query;

pub fn index_for_name(query: &Query, name: &str) -> Result<u32, Error> {
    query.capture_index_for_name(name).ok_or_else(|| {
        log::mk_err!(
            "failed to find index {} in tree-sitter query: {:?}",
            name,
            query
        )
    })
}
