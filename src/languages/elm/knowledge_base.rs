use crate::languages::elm::idat;
use crate::support::source_code::Buffer;
use crate::Error;
use core::ops::Range;
use ropey::RopeSlice;
use serde::Deserialize;
use std::collections::HashMap;
use std::collections::HashSet;
use std::io::{BufReader, Read};
use std::iter::FromIterator;
use std::path::{Path, PathBuf};
use tree_sitter::{Language, Query, QueryCursor, Tree};

pub struct KnowledgeBase {
    buffers: HashMap<Buffer, BufferInfo>,
    projects: HashMap<PathBuf, ProjectInfo>,
    query_for_exports: ExportsQuery,
}

impl KnowledgeBase {
    pub(crate) fn new() -> Result<KnowledgeBase, Error> {
        let language = tree_sitter_elm::language();
        let kb = KnowledgeBase {
            buffers: HashMap::new(),
            projects: HashMap::new(),
            query_for_exports: ExportsQuery::init(language)?,
        };
        Ok(kb)
    }

    pub(crate) fn constructors_for_type<'a, 'b>(
        &'a self,
        buffer: Buffer,
        module_name: RopeSlice<'b>,
        type_name: RopeSlice<'b>,
    ) -> Result<&'a Vec<String>, Error> {
        self.module_exports(buffer, module_name)?
            .iter()
            .find_map(|export| match export {
                ElmExport::Value { .. } => None,
                ElmExport::Type { name, constructors } => {
                    if type_name.eq(name) {
                        Some(constructors)
                    } else {
                        None
                    }
                }
            })
            .ok_or_else(|| Error::ElmNoSuchTypeInModule {
                module_name: module_name.to_string(),
                type_name: type_name.to_string(),
            })
    }

    pub(crate) fn module_exports(
        &self,
        buffer: Buffer,
        module: RopeSlice,
    ) -> Result<&Vec<ElmExport>, Error> {
        let project = self.buffer_project(buffer)?;
        match project.modules.get(&module.to_string()) {
            None => panic!("no such module"),
            Some(ElmModule { exports }) => Ok(exports),
        }
    }

    fn buffer_project(&self, buffer: Buffer) -> Result<&ProjectInfo, Error> {
        let buffer_info = self
            .buffers
            .get(&buffer)
            .ok_or(Error::ElmNoProjectStoredForBuffer(buffer))?;
        let project_info =
            self.projects.get(&buffer_info.project_root).unwrap();
        Ok(project_info)
    }

    pub(crate) fn init_buffer(
        &mut self,
        buffer: Buffer,
        path: PathBuf,
    ) -> Result<(), Error> {
        let project_root = project_root_for_path(&path)?.to_owned();
        self.init_project(&project_root)?;
        let buffer_info = BufferInfo { path, project_root };
        self.buffers.insert(buffer, buffer_info);
        Ok(())
    }

    fn init_project(&mut self, project_root: &Path) -> Result<(), Error> {
        if self.projects.contains_key(project_root) {
            return Ok(());
        }
        // TODO: Remove harcoded Elm version.
        let mut modules =
            from_idat(project_root.join("elm-stuff/0.19.1/i.dat"))?;
        modules.extend(find_project_modules(self, project_root));
        let project_info = ProjectInfo { modules };
        self.projects.insert(project_root.to_owned(), project_info);
        Ok(())
    }
}

pub(crate) fn project_root_for_path(path: &Path) -> Result<&Path, Error> {
    let mut maybe_root = path;
    loop {
        if maybe_root.join("elm.json").exists() {
            return Ok(maybe_root);
        } else {
            match maybe_root.parent() {
                None => {
                    return Err(Error::NoElmJsonFoundInAnyAncestorDirectory);
                }
                Some(parent) => {
                    maybe_root = parent;
                }
            }
        }
    }
}

pub struct BufferInfo {
    pub project_root: PathBuf,
    pub path: PathBuf,
}

#[derive(Debug)]
struct ProjectInfo {
    modules: HashMap<String, ElmModule>,
}

#[derive(Debug)]
struct ElmModule {
    exports: Vec<ElmExport>,
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

fn find_project_modules(
    kb: &KnowledgeBase,
    project_root: &Path,
) -> HashMap<String, ElmModule> {
    // TODO: replace unwrap()'s with error logging
    let file = std::fs::File::open(project_root.join("elm.json")).unwrap();
    let reader = BufReader::new(file);
    let elm_json: ElmJson = serde_json::from_reader(reader).unwrap();
    let mut modules_found = HashMap::new();
    for dir in elm_json.source_directories {
        let source_dir = project_root.join(&dir).canonicalize().unwrap();
        find_project_modules_in_dir(
            kb,
            &source_dir,
            &source_dir,
            &mut modules_found,
        );
    }
    modules_found
}

fn find_project_modules_in_dir(
    kb: &KnowledgeBase,
    dir_path: &Path,
    source_dir: &Path,
    modules: &mut HashMap<String, ElmModule>,
) {
    let dir = std::fs::read_dir(dir_path).unwrap();
    for entry in dir {
        let path = entry.unwrap().path();
        if path.is_dir() {
            find_project_modules_in_dir(kb, &path, source_dir, modules);
        } else if path.extension() == Some(std::ffi::OsStr::new("elm")) {
            let module_name = path
                .with_extension("")
                .strip_prefix(source_dir)
                .unwrap()
                .components()
                .filter_map(|component| {
                    if let std::path::Component::Normal(os_str) = component {
                        let str = os_str.to_str().unwrap();
                        Some(str)
                    } else {
                        None
                    }
                })
                .my_intersperse(".")
                .collect();
            let elm_module = parse_module(kb, &path).unwrap();
            modules.insert(module_name, elm_module);
        }
    }
}

fn parse_module(kb: &KnowledgeBase, path: &Path) -> Result<ElmModule, Error> {
    let mut file =
        std::fs::File::open(path).map_err(Error::ElmFailedToReadFile)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(Error::ElmFailedToReadFile)?;
    let tree = crate::support::source_code::parse_bytes(&bytes)?;
    let exports = kb.query_for_exports.run(&tree, &bytes)?;
    let elm_module = ElmModule { exports };
    Ok(elm_module)
}

struct ExportsQuery {
    query: Query,
    pattern_query: Query,
    exposed_all_index: u32,
    exposed_value_index: u32,
    exposed_type_index: u32,
    value_index: u32,
    pattern_index: u32,
    type_index: u32,
}

impl ExportsQuery {
    fn init(lang: Language) -> Result<ExportsQuery, Error> {
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
                [
                  (function_declaration_left
                    .
                    (lower_case_identifier) @value
                  )
                  (pattern) @pattern
                ]
              )
              (type_alias_declaration
                name: (type_identifier) @type
              )
              (type_declaration
                name: (type_identifier) @type
                unionVariant: (union_variant
                  name: (constructor_identifier) @constructor
                )+
              )
            ]+"#;
        let query = Query::new(lang, query_str)
            .map_err(Error::TreeSitterFailedToParseQuery)?;
        let pattern_query = Query::new(lang, r"(lower_pattern)")
            .map_err(Error::TreeSitterFailedToParseQuery)?;
        let exports_query = ExportsQuery {
            exposed_all_index: index_for_name(&query, "exposed_all")?,
            exposed_value_index: index_for_name(&query, "exposed_value")?,
            exposed_type_index: index_for_name(&query, "exposed_type")?,
            value_index: index_for_name(&query, "value")?,
            type_index: index_for_name(&query, "type")?,
            pattern_index: index_for_name(&query, "pattern")?,
            query,
            pattern_query,
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
                let val = Exposed::Value(code_slice(
                    code,
                    capture.node.byte_range(),
                )?);
                exposed = exposed.add(val);
            } else if self.exposed_type_index == capture.index {
                let name = code_slice(
                    code,
                    capture.node.child(0).unwrap().byte_range(),
                )?;
                let val = if capture.node.child(1).is_some() {
                    Exposed::TypeWithConstructors(name)
                } else {
                    Exposed::Type(name)
                };
                exposed = exposed.add(val);
            } else if self.value_index == capture.index {
                let name = code_slice(code, capture.node.byte_range())?;
                if exposed.has(&Exposed::Value(name)) {
                    let export = ElmExport::Value {
                        name: name.to_owned(),
                    };
                    exports.push(export);
                }
            } else if self.pattern_index == capture.index {
                let mut pattern_cursor = QueryCursor::new();
                let pattern_vars = pattern_cursor
                    .matches(&self.pattern_query, capture.node, code)
                    .filter_map(|match_| {
                        if let [capture, ..] = match_.captures {
                            Some(capture)
                        } else {
                            None
                        }
                    });
                for var in pattern_vars {
                    let name = code_slice(code, var.node.byte_range())?;
                    if exposed.has(&Exposed::Value(name)) {
                        let export = ElmExport::Value {
                            name: name.to_owned(),
                        };
                        exports.push(export);
                    }
                }
            } else if self.type_index == capture.index {
                let name = code_slice(code, capture.node.byte_range())?;
                if exposed.has(&Exposed::Type(name)) {
                    let export = ElmExport::Type {
                        name: name.to_owned(),
                        constructors: Vec::new(),
                    };
                    exports.push(export);
                } else if exposed.has(&Exposed::TypeWithConstructors(name)) {
                    let constructors = rest
                        .iter()
                        .map(|ctor_capture| {
                            code_slice(code, ctor_capture.node.byte_range())
                                .map(std::borrow::ToOwned::to_owned)
                        })
                        .collect::<Result<Vec<String>, Error>>()?;
                    let export = ElmExport::Type {
                        name: name.to_owned(),
                        constructors,
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

fn code_slice(code: &[u8], range: Range<usize>) -> Result<&str, Error> {
    std::str::from_utf8(&code[range]).map_err(Error::ElmModuleReadingUtf8Failed)
}

fn index_for_name(query: &Query, name: &str) -> Result<u32, Error> {
    query
        .capture_index_for_name(name)
        .ok_or(Error::TreeSitterQueryDoesNotHaveExpectedIndex)
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

fn from_idat(path: PathBuf) -> Result<HashMap<String, ElmModule>, Error> {
    let file = std::fs::File::open(path).unwrap();
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
