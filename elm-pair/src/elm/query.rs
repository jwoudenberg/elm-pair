#[macro_export]
macro_rules! query {
    ($name:ident, $test_mod_name:ident, $file:literal $(, $capture:ident )* $(,)? ) => {
        struct $name {
            query: Query,
            $($capture: u32,)*
        }

        impl $name {
            fn init(lang: Language) -> Result<$name, Error> {
                let query_file_contents = include_str!($file);
                let separator = "=== test input below ===";
                let query_str =
                      match query_file_contents.split_once(separator) {
                          Some((query, _)) => query,
                          None => query_file_contents,
                      };
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

        #[cfg(test)]
        mod $test_mod_name {
            use super::$name;

            #[test]
            fn query_sample_data() {
                let language = tree_sitter_elm::language();
                let query = $name::init(language).unwrap();
                let separator = "=== test input below ===";
                let test_str =
                      match include_str!($file).split_once(separator) {
                          Some((_, test)) => test,
                          None => panic!("No test input found in query file.")
                      };
                let mut cursor = tree_sitter::QueryCursor::new();
                let tree = $crate::support::source_code::parse_bytes(test_str).unwrap();
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
                $crate::test_support::included_answer_test::assert_eq_answer_in(
                    output.as_str(),
                    &query_file_path,
                );
            }
        }
    };
}
pub use query;
