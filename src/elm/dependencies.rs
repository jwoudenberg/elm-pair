use crate::elm::idat;
use crate::support::log;
use crate::support::log::Error;
use serde::Deserialize;
use std::collections::HashMap;
use std::collections::HashSet;
use std::io::{BufReader, Read};
use std::iter::FromIterator;
use std::path::{Path, PathBuf};
use tree_sitter::{Language, Node, Query, QueryCursor, Tree};

#[derive(Debug)]
pub struct ProjectInfo {
    pub source_directories: Vec<PathBuf>,
    pub modules: HashMap<String, ElmModule>,
    pub elm_json_path: PathBuf,
    pub idat_path: PathBuf,
}

#[derive(Debug)]
pub struct ElmModule {
    pub exports: Vec<ElmExport>,
}

#[derive(Debug)]
pub enum ElmExport {
    Value {
        name: String,
    },
    Type {
        name: String,
        constructors: Vec<String>,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct ElmJson {
    #[serde(rename = "source-directories")]
    source_directories: Vec<PathBuf>,
}

pub(crate) fn load_dependencies(
    query_for_exports: &ExportsQuery,
    project_root: &Path,
) -> Result<ProjectInfo, Error> {
    // TODO: Remove harcoded Elm version.
    let idat_path = project_root.join("elm-stuff/0.19.1/i.dat");
    let mut modules = from_idat(project_root, &idat_path)?;
    let elm_json_path = project_root.join("elm.json");
    let elm_json = load_elm_json(&elm_json_path)?;
    modules.extend(find_project_modules(
        query_for_exports,
        project_root,
        &elm_json,
    )?);
    let project_info = ProjectInfo {
        elm_json_path,
        idat_path,
        modules,
        source_directories: elm_json
            .source_directories
            .into_iter()
            .map(|dir| project_root.join(dir))
            .collect(),
    };
    Ok(project_info)
}

fn load_elm_json(path: &Path) -> Result<ElmJson, Error> {
    let file = std::fs::File::open(path).map_err(|err| {
        log::mk_err!("error while reading elm.json: {:?}", err)
    })?;
    let reader = BufReader::new(file);
    serde_json::from_reader(reader)
        .map_err(|err| log::mk_err!("error while parsing elm.json: {:?}", err))
}

fn find_project_modules(
    query_for_exports: &ExportsQuery,
    project_root: &Path,
    elm_json: &ElmJson,
) -> Result<HashMap<String, ElmModule>, Error> {
    let mut modules_found = HashMap::new();
    for dir in elm_json.source_directories.iter() {
        match project_root.join(&dir).canonicalize() {
            Ok(source_dir) => find_project_modules_in_dir(
                query_for_exports,
                &source_dir,
                &source_dir,
                &mut modules_found,
            ),
            Err(err) => {
                // If a source directory does not exist skip it. We'll still
                // read the other directories, but if the missing directory is
                // added later we'll need to load it then.
                log::error!(
                    "error while canonicalizing source directory {:?}: {:?}",
                    dir,
                    err
                )
            }
        };
    }
    Ok(modules_found)
}

// This function finds as many modules as it can and so logs rather than fails
// when it encounters an error.
fn find_project_modules_in_dir(
    query_for_exports: &ExportsQuery,
    dir_path: &Path,
    source_dir: &Path,
    modules: &mut HashMap<String, ElmModule>,
) {
    let read_dir = match std::fs::read_dir(dir_path) {
        Ok(d) => d,
        Err(err) => {
            return log::error!(
                "error while reading contents of source directory {:?}: {:?}",
                dir_path,
                err
            );
        }
    };
    let valid_paths = read_dir.filter_map(|entry| match entry {
        Ok(entry_) => Some(entry_.path()),
        Err(err) => {
            log::error!(
                "error while reading entry of source (sub)directory {:?}: {:?}",
                dir_path,
                err
            );
            None
        }
    });
    for path in valid_paths {
        if path.is_dir() {
            find_project_modules_in_dir(
                query_for_exports,
                &path,
                source_dir,
                modules,
            );
        } else if path.extension() == Some(std::ffi::OsStr::new("elm")) {
            let module_name = match module_name_from_path(source_dir, &path) {
                Ok(name) => name,
                Err(err) => {
                    log::error!("I've skipped scanning a source path because I encountered an error: {:?}", err);
                    continue;
                }
            };
            let elm_module = match parse_module(query_for_exports, &path) {
                Ok(module) => module,
                Err(err) => {
                    log::error!("I've skipped scanning a source path because I encountered an error: {:?}", err);
                    continue;
                }
            };
            modules.insert(module_name, elm_module);
        }
    }
}

fn module_name_from_path(
    source_dir: &Path,
    path: &Path,
) -> Result<String, Error> {
    path.with_extension("")
        .strip_prefix(source_dir)
        .map_err(|err|
            log::mk_err!("error stripping source directory {:?} from elm module path {:?}: {:?}", path, source_dir, err)
        )?
        .components()
        .filter_map(|component| {
            if let std::path::Component::Normal(os_str) = component {
                Some(os_str.to_str().ok_or(os_str))
            } else {
                None
            }
        })
        .my_intersperse(Ok("."))
        .collect::<Result<String, &std::ffi::OsStr>>()
        .map_err(|os_str|
            log::mk_err!("directory segment of Elm module used in module name is not valid UTF8: {:?}", os_str)
        )
}

fn parse_module(
    query_for_exports: &ExportsQuery,
    path: &Path,
) -> Result<ElmModule, Error> {
    let mut file = std::fs::File::open(path)
        .map_err(|err| log::mk_err!("failed to open module file: {:?}", err))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|err| log::mk_err!("failed to read module file: {:?}", err))?;
    let tree = crate::support::source_code::parse_bytes(&bytes)?;
    let exports = query_for_exports.run(&tree, &bytes)?;
    let elm_module = ElmModule { exports };
    Ok(elm_module)
}

pub struct ExportsQuery {
    query: Query,
    exposed_all_index: u32,
    exposed_value_index: u32,
    exposed_type_index: u32,
    value_index: u32,
    type_index: u32,
}

impl ExportsQuery {
    pub(crate) fn init(lang: Language) -> Result<ExportsQuery, Error> {
        let query_str = r#"
            [
              (module_declaration
                exposing: (exposing_list
                  [
                    (double_dot)       @exposed_all
                    (exposed_value)    @exposed_value
                    (exposed_type)     @exposed_type
                    (exposed_operator) @exposed_value
                  ]
                )
              )
              (value_declaration
                  (function_declaration_left
                    .
                    (lower_case_identifier) @value
                  )
              )
              (type_alias_declaration
                name: (type_identifier) @type
              )
              (type_declaration
                name: (type_identifier) @type
                (union_variant
                    name: (constructor_identifier) @constructor
                )
                (
                    "|"
                    (union_variant
                        name: (constructor_identifier) @constructor
                    )
                )*
              )
            ]"#;
        let query = Query::new(lang, query_str).map_err(|err| {
            log::mk_err!("Failed to parse tree-sitter ExportsQuery: {:?}", err)
        })?;
        let exports_query = ExportsQuery {
            exposed_all_index: index_for_name(&query, "exposed_all")?,
            exposed_value_index: index_for_name(&query, "exposed_value")?,
            exposed_type_index: index_for_name(&query, "exposed_type")?,
            value_index: index_for_name(&query, "value")?,
            type_index: index_for_name(&query, "type")?,
            query,
        };
        Ok(exports_query)
    }

    fn run(&self, tree: &Tree, code: &[u8]) -> Result<Vec<ElmExport>, Error> {
        let mut cursor = QueryCursor::new();
        let matches = cursor
            .matches(&self.query, tree.root_node(), code)
            .filter_map(|match_| {
                if let [capture, rest @ ..] = match_.captures {
                    Some((capture, rest))
                } else {
                    None
                }
            });
        let mut exposed = ExposedList::Some(HashSet::new());
        let mut exports = Vec::new();
        for (capture, rest) in matches {
            if self.exposed_all_index == capture.index {
                exposed = ExposedList::All;
            } else if self.exposed_value_index == capture.index {
                let val = Exposed::Value(code_slice(code, &capture.node)?);
                exposed = exposed.add(val);
            } else if self.exposed_type_index == capture.index {
                let name_node = capture.node.child(0).ok_or_else(|| {
                    log::mk_err!(
                        "could not find name node of type in exposing list"
                    )
                })?;
                let name = code_slice(code, &name_node)?;
                let val = if capture.node.child(1).is_some() {
                    Exposed::TypeWithConstructors(name)
                } else {
                    Exposed::Type(name)
                };
                exposed = exposed.add(val);
            } else if self.value_index == capture.index {
                let name = code_slice(code, &capture.node)?;
                if exposed.has(&Exposed::Value(name)) {
                    let export = ElmExport::Value {
                        name: name.to_owned(),
                    };
                    exports.push(export);
                }
            } else if self.type_index == capture.index {
                let name = code_slice(code, &capture.node)?;
                if exposed.has(&Exposed::TypeWithConstructors(name)) {
                    let constructors = rest
                        .iter()
                        .map(|ctor_capture| {
                            code_slice(code, &ctor_capture.node)
                                .map(std::borrow::ToOwned::to_owned)
                        })
                        .collect::<Result<Vec<String>, Error>>()?;
                    let export = ElmExport::Type {
                        name: name.to_owned(),
                        constructors,
                    };
                    exports.push(export);
                } else if exposed.has(&Exposed::Type(name)) {
                    let export = ElmExport::Type {
                        name: name.to_owned(),
                        constructors: Vec::new(),
                    };
                    exports.push(export);
                }
            }
        }
        Ok(exports)
    }
}

enum ExposedList<'a> {
    All,
    Some(HashSet<Exposed<'a>>),
}

impl<'a> ExposedList<'a> {
    fn add(mut self, item: Exposed<'a>) -> Self {
        match &mut self {
            ExposedList::All => {}
            ExposedList::Some(items) => {
                items.insert(item);
            }
        }
        self
    }

    fn has(&self, item: &Exposed) -> bool {
        match self {
            ExposedList::All => true,
            ExposedList::Some(items) => items.contains(item),
        }
    }
}

#[derive(Hash, PartialEq)]
enum Exposed<'a> {
    Type(&'a str),
    TypeWithConstructors(&'a str),
    Value(&'a str),
}

impl Eq for Exposed<'_> {}

fn code_slice<'a>(code: &'a [u8], node: &Node) -> Result<&'a str, Error> {
    std::str::from_utf8(&code[node.byte_range()]).map_err(|err| {
        log::mk_err!(
            "Failed to decode code slice for node {} as UTF8: {:?}",
            node.kind(),
            err
        )
    })
}

pub(crate) fn index_for_name(query: &Query, name: &str) -> Result<u32, Error> {
    query.capture_index_for_name(name).ok_or_else(|| {
        log::mk_err!(
            "failed to find index {} in tree-sitter query: {:?}",
            name,
            query
        )
    })
}

// Tust nightlies already contain a `intersperse` iterator. Once that lands
// in stable we should switch over.
trait Intersperse: Iterator {
    fn my_intersperse(self, separator: Self::Item) -> IntersperseState<Self>
    where
        Self::Item: Clone,
        Self: Sized;
}

impl<I: Iterator> Intersperse for I {
    fn my_intersperse(self, separator: Self::Item) -> IntersperseState<I> {
        IntersperseState {
            iterator: self.peekable(),
            separator,
            separator_is_next: false,
        }
    }
}

struct IntersperseState<I: Iterator> {
    iterator: std::iter::Peekable<I>,
    separator: I::Item,
    separator_is_next: bool,
}

impl<I: Iterator> Iterator for IntersperseState<I>
where
    I::Item: Clone,
{
    type Item = I::Item;

    fn next(&mut self) -> Option<Self::Item> {
        if self.iterator.peek().is_none() {
            None
        } else if self.separator_is_next {
            self.separator_is_next = false;
            Some(self.separator.clone())
        } else {
            self.separator_is_next = true;
            self.iterator.next()
        }
    }
}

fn from_idat(
    project_root: &Path,
    path: &Path,
) -> Result<HashMap<String, ElmModule>, Error> {
    let file = std::fs::File::open(path).or_else(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            create_elm_stuff(project_root)?;
            std::fs::File::open(path).map_err(|err| {
                log::mk_err!("error opening elm-stuff/i.dat file: {:?}", err)
            })
        } else {
            Err(log::mk_err!(
                "error opening elm-stuff/i.dat file: {:?}",
                err
            ))
        }
    })?;
    let reader = BufReader::new(file);
    let iter =
        idat::parse(reader)?
            .into_iter()
            .filter_map(|(canonical_name, i)| {
                let idat::Name(name) = canonical_name.module;
                let module = elm_module_from_interface(i)?;
                Some((name, module))
            });
    Ok(HashMap::from_iter(iter))
}

fn create_elm_stuff(project_root: &Path) -> Result<(), Error> {
    log::info!(
        "Running `elm make` to generate elm-stuff in project: {:?}",
        project_root
    );
    // Running `elm make` will create elm-stuff. We'll pass it a valid module
    // to compile or `elm make` will return an error. `elm make` would create
    // `elm-stuff` before returning an error, but it'd be difficult to
    // distinguish that expected error from other potential unexpected ones.
    let temp_module = ropey::Rope::from_str(
        "\
        module Main exposing (..)\n\
        val : Int\n\
        val = 4\n\
        ",
    );
    let output = crate::elm::compiler::make(project_root, &temp_module)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(log::mk_err!(
            "failed running elm-make to generate elm-stuff:\n{:?}",
            std::string::String::from_utf8(output.stderr)
        ))
    }
}

fn elm_module_from_interface(
    dep_i: idat::DependencyInterface,
) -> Option<ElmModule> {
    if let idat::DependencyInterface::Public(interface) = dep_i {
        // TODO: add binops
        let values = interface.values.into_iter().map(elm_export_from_value);
        let unions = interface.unions.into_iter().map(elm_export_from_union);
        let aliases = interface.aliases.into_iter().map(elm_export_from_alias);
        let exports = Vec::from_iter(values.chain(unions).chain(aliases));
        Some(ElmModule { exports })
    } else {
        None
    }
}

fn elm_export_from_value(
    (idat::Name(name), _): (idat::Name, idat::CanonicalAnnotation),
) -> ElmExport {
    ElmExport::Value { name }
}

fn elm_export_from_union(
    (idat::Name(name), union): (idat::Name, idat::Union),
) -> ElmExport {
    let constructor_names = |canonical_union: idat::CanonicalUnion| {
        let iter = canonical_union
            .alts
            .into_iter()
            .map(|idat::Ctor(idat::Name(name), _, _, _)| name);
        Vec::from_iter(iter)
    };
    let constructors = match union {
        idat::Union::Open(canonical_union) => {
            constructor_names(canonical_union)
        }
        // We're reading this information for use by other modules.
        // These external modules can't see private constructors,
        // so we don't need to return them here.
        idat::Union::Closed(_) => Vec::new(),
        idat::Union::Private(_) => Vec::new(),
    };
    ElmExport::Type { name, constructors }
}

fn elm_export_from_alias(
    (idat::Name(name), _): (idat::Name, idat::Alias),
) -> ElmExport {
    ElmExport::Type {
        name,
        constructors: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use crate::elm::dependencies::{
        parse_module, ElmModule, ExportsQuery, Intersperse,
    };
    use crate::support::log::Error;
    use crate::test_support::included_answer_test as ia_test;
    use std::path::Path;

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
        match run_exports_scanning_test_helper(path) {
            Err(err) => {
                eprintln!("{:?}", err);
                panic!();
            }
            Ok(res) => ia_test::assert_eq_answer_in(&res, path),
        }
    }

    fn run_exports_scanning_test_helper(path: &Path) -> Result<String, Error> {
        let language = tree_sitter_elm::language();
        let query_for_exports = ExportsQuery::init(language)?;
        let ElmModule { exports } = parse_module(&query_for_exports, path)?;
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
