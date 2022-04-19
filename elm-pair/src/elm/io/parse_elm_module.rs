use crate::editors;
use crate::elm::io::ExportedName;
use crate::elm::module_name::ModuleName;
use crate::elm::queries::exports;
use crate::elm::queries::imports;
use crate::lib::log;
use crate::lib::log::Error;
use crate::lib::source_code::{Buffer, SourceFileSnapshot};
use std::io::Read;
use std::path::Path;
use tree_sitter::QueryCursor;

pub type Module = (Vec<ExportedName>, Vec<ModuleName>);

pub fn parse_elm_module(
    query_for_exports: &exports::Query,
    query_for_imports: &imports::Query,
    path: &Path,
) -> Result<Module, Error> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(err) => {
            if let std::io::ErrorKind::NotFound = err.kind() {
                return Ok((Vec::new(), Vec::new()));
            } else {
                return Err(log::mk_err!(
                    "failed to open module file: {:?}",
                    err
                ));
            };
        }
    };
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|err| log::mk_err!("failed to read module file: {:?}", err))?;
    parse_bytes(query_for_exports, query_for_imports, &bytes)
}

fn parse_bytes(
    query_for_exports: &exports::Query,
    query_for_imports: &imports::Query,
    bytes: &[u8],
) -> Result<(Vec<ExportedName>, Vec<ModuleName>), Error> {
    let tree = crate::lib::source_code::parse_bytes(&bytes)?;
    let code = SourceFileSnapshot {
        // TODO: Handle UTF8 parsing error nicely.
        bytes: std::str::from_utf8(bytes).unwrap().into(),
        tree,
        // TODO: Remove need to make up a buffer and revision here.
        buffer: Buffer {
            editor_id: editors::Id::new(0),
            buffer_id: 0,
        },
        revision: 0,
    };
    let exports = query_for_exports.run(&code)?;
    let mut cursor = QueryCursor::new();
    let imports = query_for_imports.run(&mut cursor, &code);
    Ok((
        exports,
        imports.map(|import| import.module_name()).collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lib::included_answer_test as ia_test;
    use crate::lib::intersperse::Intersperse;

    macro_rules! exports_scanning_test {
        ($name:ident) => {
            #[test]
            fn $name() {
                let mut path = std::path::PathBuf::new();
                path.push("./tests/exports-scanning");
                let module_name = stringify!($name);
                path.push(module_name.to_owned() + ".elm");
                println!("Run simulation {:?}", &path);
                run_exports_scanning_test(&path);
            }
        };
    }

    fn run_exports_scanning_test(path: &Path) {
        ia_test::for_file(path, |input| match run_exports_scanning_test_helper(
            input,
        ) {
            Err(err) => {
                eprintln!("{:?}", err);
                panic!();
            }
            Ok(res) => res,
        })
    }

    fn run_exports_scanning_test_helper(input: &str) -> Result<String, Error> {
        let language = tree_sitter_elm::language();
        let query_for_exports = exports::Query::init(language)?;
        let query_for_imports = imports::Query::init(language)?;
        let (exports, _) = parse_bytes(
            &query_for_exports,
            &query_for_imports,
            input.as_bytes(),
        )?;
        let output = exports
            .into_iter()
            .map(|export| format!("{:?}", export))
            .my_intersperse("\n".to_owned())
            .collect();
        Ok(output)
    }

    exports_scanning_test!(exposing_all);
    exports_scanning_test!(exposing_minimal);
    exports_scanning_test!(hiding_constructors);
}
