use crate::support::source_code::Buffer;
use crate::Error;
use byteorder::{BigEndian, ReadBytesExt};
use ropey::RopeSlice;
use serde::Deserialize;
use std::collections::HashMap;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

pub struct KnowledgeBase {
    buffer_path: HashMap<Buffer, PathBuf>,
    buffer_project: HashMap<Buffer, Project>,
    compilation_params: HashMap<Buffer, CompilationParams>,
    project_root: HashMap<PathBuf, PathBuf>,
}

macro_rules! memoize {
    ($cache:expr, $question:expr, $answer:expr) => {{
        if !$cache.contains_key($question) {
            let res = match $answer {
                Ok(ok) => ok,
                Err(err) => return Err(err),
            };
            $cache.insert($question.clone(), res);
        }
        Ok($cache.get($question).unwrap())
    }};
}

impl KnowledgeBase {
    pub fn new() -> KnowledgeBase {
        KnowledgeBase {
            buffer_path: HashMap::new(),
            buffer_project: HashMap::new(),
            project_root: HashMap::new(),
            compilation_params: HashMap::new(),
        }
    }

    pub(crate) fn constructors_for_type<'a, 'b>(
        &'a mut self,
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
        &mut self,
        buffer: Buffer,
        module: RopeSlice,
    ) -> Result<&Vec<ElmExport>, Error> {
        let project = self.buffer_project(buffer)?;
        match project.modules.get(&module.to_string()) {
            None => panic!("no such module"),
            Some(ElmModule::_InProject { .. }) => panic!("not implemented yet"),
            Some(ElmModule::FromDependency { exposed_modules }) => {
                Ok(exposed_modules)
            }
        }
    }

    pub(crate) fn buffer_project(
        &mut self,
        buffer: Buffer,
    ) -> Result<&Project, Error> {
        memoize!(self.buffer_project, &buffer, {
            // TODO: Avoid need to copy buffer_path here.
            let buffer_path = self.buffer_path(buffer)?.to_owned();
            let project_root = self.project_root(&buffer_path)?;
            let file =
                std::fs::File::open(project_root.join("elm.json")).unwrap();
            let reader = BufReader::new(file);
            let _elm_json: ElmJson = serde_json::from_reader(reader).unwrap();
            let modules_from_dependencies =
                parse_idat(project_root.join("elm-stuff/0.19.1/i.dat"))?;
            let project = Project {
                // TODO: Add project sourcefiles.
                // TODO: Remove harcoded Elm version.
                modules: modules_from_dependencies,
            };
            Ok(project)
        })
    }

    pub(crate) fn get_compilation_params(
        &mut self,
        buffer: Buffer,
    ) -> Result<&CompilationParams, Error> {
        memoize!(self.compilation_params, &buffer, {
            let buffer_path = self.buffer_path(buffer)?.to_owned();
            let root = self.project_root(&buffer_path)?.to_owned();
            let elm_bin = find_executable("elm")?;
            Ok(CompilationParams { root, elm_bin })
        })
    }

    pub(crate) fn insert_buffer_path(&mut self, buffer: Buffer, path: PathBuf) {
        self.buffer_path.insert(buffer, path);
    }

    fn buffer_path(&mut self, buffer: Buffer) -> Result<&PathBuf, Error> {
        memoize!(self.buffer_path, &buffer, {
            Err(Error::ElmNoProjectStoredForBuffer(buffer))
        })
    }

    // TODO: return path to `elm.json` instead.
    fn project_root(&mut self, maybe_root: &Path) -> Result<&PathBuf, Error> {
        memoize!(self.project_root, &maybe_root.to_path_buf(), {
            if maybe_root.join("elm.json").exists() {
                Ok(maybe_root.to_owned())
            } else {
                // TODO: find a way that queries for multiple file can share a
                // reference to the same PathBuf.
                match maybe_root.parent() {
                    None => Err(Error::NoElmJsonFoundInAnyAncestorDirectory),
                    Some(parent) => {
                        let root = self.project_root(&parent.to_owned())?;
                        Ok(root.to_owned())
                    }
                }
            }
        })
    }
}

pub struct CompilationParams {
    // Root of the Elm project containing this source file.
    pub root: PathBuf,
    // Absolute path to the `elm` compiler.
    pub elm_bin: PathBuf,
}

#[derive(Debug)]
pub struct Project {
    pub modules: HashMap<String, ElmModule>,
}

#[derive(Debug)]
pub enum ElmModule {
    _InProject { path: PathBuf },
    FromDependency { exposed_modules: Vec<ElmExport> },
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
    _source_directories: Vec<PathBuf>,
}

fn find_executable(name: &str) -> Result<PathBuf, Error> {
    let cwd = std::env::current_dir()
        .map_err(Error::CouldNotReadCurrentWorkingDirectory)?;
    let path = std::env::var_os("PATH").ok_or(Error::DidNotFindPathEnvVar)?;
    let dirs = std::env::split_paths(&path);
    for dir in dirs {
        let mut bin_path = cwd.join(dir);
        bin_path.push(name);
        if bin_path.is_file() {
            return Ok(bin_path);
        };
    }
    Err(Error::ElmDidNotFindCompilerBinaryOnPath)
}

fn parse_idat(path: PathBuf) -> Result<HashMap<String, ElmModule>, Error> {
    let file = std::fs::File::open(path).unwrap();
    let mut parser = ElmIdatParser {
        reader: BufReader::new(file),
    };
    let idat = parser.data_binary_map(
        ElmIdatParser::elm_canonical_module_name,
        ElmIdatParser::elm_dependency_interface,
    )?;
    panic!()
}

struct ElmIdatParser<R> {
    reader: BufReader<R>,
}

impl<R: Read> ElmIdatParser<R> {
    fn data_binary_int(&mut self) -> Result<i64, Error> {
        self.reader
            .read_i64::<BigEndian>()
            .map_err(Error::ElmIdatReadingFailed)
    }

    fn data_binary_word8(&mut self) -> Result<u8, Error> {
        let mut bytes = [0];
        self.reader
            .read_exact(&mut bytes)
            .map_err(Error::ElmIdatReadingFailed)?;
        Ok(bytes[0])
    }

    fn data_binary_word16(&mut self) -> Result<u16, Error> {
        self.reader
            .read_u16::<BigEndian>()
            .map_err(Error::ElmIdatReadingFailed)
    }

    fn elm_utf8_under256(&mut self) -> Result<String, Error> {
        let len = self.data_binary_word8()? as usize;
        let mut full_bytes = [0; 256];
        let mut bytes = &mut full_bytes[0..len];
        self.reader
            .read_exact(&mut bytes)
            .map_err(Error::ElmIdatReadingFailed)?;
        let str = std::str::from_utf8(bytes)
            .map_err(Error::ElmIdatReadingUtf8Failed)?;
        Ok(str.to_owned())
    }

    fn data_binary_maybe<F, A>(&mut self, val: F) -> Result<Option<A>, Error>
    where
        F: Fn(&mut Self) -> Result<A, Error>,
    {
        let kind = self.data_binary_word8()?;
        match kind {
            0 => Ok(None),
            1 => val(self).map(Some),
            _ => Err(Error::ElmIdatReadingUnexpectedKind {
                kind,
                during: "Maybe".to_owned(),
            }),
        }
    }

    fn data_binary_list<F, A>(&mut self, val: F) -> Result<Vec<A>, Error>
    where
        F: Fn(&mut Self) -> Result<A, Error>,
    {
        let len = self.data_binary_int()? as usize;
        let mut vec = Vec::with_capacity(len);
        while vec.len() < len {
            vec.push(val(self)?);
        }
        Ok(vec)
    }

    fn data_binary_map<F, G, K, V>(
        &mut self,
        key: F,
        val: G,
    ) -> Result<DataMap<K, V>, Error>
    where
        F: Fn(&mut Self) -> Result<K, Error>,
        G: Fn(&mut Self) -> Result<V, Error>,
    {
        self.data_binary_list(|list_self| {
            Ok((key(list_self)?, val(list_self)?))
        })
    }

    fn elm_canonical_module_name(
        &mut self,
    ) -> Result<ElmCanonicalModuleName, Error> {
        let name = ElmCanonicalModuleName {
            package: self.elm_package_name()?,
            module: self.elm_name()?,
        };
        Ok(name)
    }

    fn elm_package_name(&mut self) -> Result<ElmPackageName, Error> {
        let name = ElmPackageName {
            author: self.elm_utf8_under256()?,
            package: self.elm_utf8_under256()?,
        };
        Ok(name)
    }

    fn elm_name(&mut self) -> Result<ElmName, Error> {
        let name = self.elm_utf8_under256()?;
        Ok(ElmName(name))
    }

    fn elm_dependency_interface(
        &mut self,
    ) -> Result<ElmDependencyInterface, Error> {
        let kind = self.data_binary_word8()?;
        match kind {
            0 => {
                let interface = self.elm_interface()?;
                Ok(ElmDependencyInterface::Public(interface))
            }
            1 => {
                let package_name = self.elm_package_name()?;
                let unions = self.data_binary_map(
                    Self::elm_name,
                    Self::elm_canonical_union,
                )?;
                let aliases = self.data_binary_map(
                    Self::elm_name,
                    Self::elm_canonical_alias,
                )?;
                Ok(ElmDependencyInterface::Private(
                    package_name,
                    unions,
                    aliases,
                ))
            }
            _ => Err(Error::ElmIdatReadingUnexpectedKind {
                kind,
                during: "ElmDependencyInterface".to_owned(),
            }),
        }
    }

    fn elm_interface(&mut self) -> Result<ElmInterface, Error> {
        let interface = ElmInterface {
            home: self.elm_package_name()?,
            values: self.data_binary_map(
                Self::elm_name,
                Self::elm_canonical_annotation,
            )?,
            unions: self.data_binary_map(Self::elm_name, Self::elm_union)?,
            aliases: self.data_binary_map(Self::elm_name, Self::elm_alias)?,
            binops: self.data_binary_map(Self::elm_name, Self::elm_binop)?,
        };
        Ok(interface)
    }

    fn elm_canonical_union(&mut self) -> Result<ElmCanonicalUnion, Error> {
        let vars = self.data_binary_list(Self::elm_name)?;
        let alts = self.data_binary_list(Self::elm_ctor)?;
        let numAlts = self.data_binary_int()?;
        let opts = self.elm_ctor_opts()?;
        Ok(ElmCanonicalUnion {
            vars,
            alts,
            numAlts,
            opts,
        })
    }

    fn elm_ctor(&mut self) -> Result<ElmCtor, Error> {
        let name = self.elm_name()?;
        let index = self.elm_index_zero_based()?;
        let len = self.data_binary_int()?;
        let args = self.data_binary_list(Self::elm_type)?;
        Ok(ElmCtor(name, index, len, args))
    }

    fn elm_index_zero_based(&mut self) -> Result<ElmIndexZeroBased, Error> {
        let index = self.data_binary_int()?;
        Ok(ElmIndexZeroBased(index))
    }

    fn elm_ctor_opts(&mut self) -> Result<ElmCtorOpts, Error> {
        let kind = self.data_binary_word8()?;
        match kind {
            0 => Ok(ElmCtorOpts::Normal),
            1 => Ok(ElmCtorOpts::Enum),
            2 => Ok(ElmCtorOpts::Unbox),
            _ => Err(Error::ElmIdatReadingUnexpectedKind {
                kind,
                during: "CtorOpts".to_owned(),
            }),
        }
    }

    fn elm_canonical_alias(&mut self) -> Result<ElmCanonicalAlias, Error> {
        let names = self.data_binary_list(Self::elm_name)?;
        let type_ = self.elm_type()?;
        Ok(ElmCanonicalAlias(names, type_))
    }

    fn elm_canonical_annotation(
        &mut self,
    ) -> Result<ElmCanonicalAnnotation, Error> {
        let free_vars = self.data_binary_map(Self::elm_name, |_| Ok(()))?;
        let type_ = self.elm_type()?;
        Ok(ElmCanonicalAnnotation(free_vars, type_))
    }

    fn elm_union(&mut self) -> Result<ElmUnion, Error> {
        let kind = self.data_binary_word8()?;
        match kind {
            0 => self.elm_canonical_union().map(ElmUnion::Open),
            1 => self.elm_canonical_union().map(ElmUnion::Closed),
            2 => self.elm_canonical_union().map(ElmUnion::Private),
            _ => Err(Error::ElmIdatReadingUnexpectedKind {
                kind,
                during: "Union".to_owned(),
            }),
        }
    }

    fn elm_alias(&mut self) -> Result<ElmAlias, Error> {
        let kind = self.data_binary_word8()?;
        match kind {
            0 => self.elm_canonical_alias().map(ElmAlias::Public),
            1 => self.elm_canonical_alias().map(ElmAlias::Private),
            _ => Err(Error::ElmIdatReadingUnexpectedKind {
                kind,
                during: "Alias".to_owned(),
            }),
        }
    }

    fn elm_binop(&mut self) -> Result<ElmBinop, Error> {
        let name = self.elm_name()?;
        let annotation = self.elm_canonical_annotation()?;
        let associativity = self.elm_binop_associativity()?;
        let precedence = self.elm_binop_precedence()?;
        Ok(ElmBinop {
            name,
            annotation,
            associativity,
            precedence,
        })
    }

    fn elm_binop_associativity(
        &mut self,
    ) -> Result<ElmBinopAssociativity, Error> {
        let kind = self.data_binary_word8()?;
        match kind {
            0 => Ok(ElmBinopAssociativity::Left),
            1 => Ok(ElmBinopAssociativity::Non),
            2 => Ok(ElmBinopAssociativity::Right),
            _ => Err(Error::ElmIdatReadingUnexpectedKind {
                kind,
                during: "BinopAssociativity".to_owned(),
            }),
        }
    }

    fn elm_binop_precedence(&mut self) -> Result<ElmBinopPrecedence, Error> {
        let n = self.data_binary_int()?;
        Ok(ElmBinopPrecedence(n))
    }

    fn elm_type(&mut self) -> Result<ElmType, Error> {
        let kind = self.data_binary_word8()?;
        match kind {
            // Lambda
            0 => {
                let a = self.elm_type()?;
                let b = self.elm_type()?;
                Ok(ElmType::Lambda(Box::new(a), Box::new(b)))
            }
            // Var
            1 => {
                let name = self.elm_name()?;
                Ok(ElmType::Var(name))
            }
            // Record
            2 => {
                let vals =
                    self.data_binary_map(Self::elm_name, Self::elm_field_type)?;
                let name = self.data_binary_maybe(Self::elm_name)?;
                Ok(ElmType::Record(vals, name))
            }
            // Unit
            3 => Ok(ElmType::Unit),
            // Tuple
            4 => {
                let a = self.elm_type()?;
                let b = self.elm_type()?;
                let name = self.data_binary_maybe(Self::elm_name)?;
                Ok(ElmType::Tuple(Box::new(a), Box::new(b), name))
            }
            // Alias
            5 => {
                let module_name = self.elm_canonical_module_name()?;
                let name = self.elm_name()?;
                let types = self.data_binary_list(|list_self| {
                    let list_elem_name = list_self.elm_name()?;
                    let list_elem_type = list_self.elm_type()?;
                    Ok((list_elem_name, list_elem_type))
                })?;
                let alias_type = self.elm_alias_type()?;
                Ok(ElmType::Alias(module_name, name, types, alias_type))
            }
            // Type
            _ => {
                let module_name = self.elm_canonical_module_name()?;
                let name = self.elm_name()?;
                let len = if kind > 7 { kind as usize - 7 } else { 0 };
                let mut ctors = Vec::with_capacity(len);
                while ctors.len() < len {
                    ctors.push(self.elm_type()?);
                }
                Ok(ElmType::Type(module_name, name, ctors))
            }
        }
    }

    fn elm_field_type(&mut self) -> Result<ElmFieldType, Error> {
        let index = self.data_binary_word16()?;
        let type_ = self.elm_type()?;
        Ok(ElmFieldType(index, type_))
    }

    fn elm_alias_type(&mut self) -> Result<ElmAliasType, Error> {
        let kind = self.data_binary_word8()?;
        match kind {
            0 => {
                let type_ = self.elm_type()?;
                Ok(ElmAliasType::Holey(Box::new(type_)))
            }
            1 => {
                let type_ = self.elm_type()?;
                Ok(ElmAliasType::Filled(Box::new(type_)))
            }
            _ => Err(Error::ElmIdatReadingUnexpectedKind {
                kind,
                during: "ElmAliasType".to_owned(),
            }),
        }
    }
}

// We currently represent Haskell's Data.Map type as a vector of tuples, to
// avoid the key constraints using a HashMap would involve. The Elm types here
// are not intended for direct use, only as a waystation between data read
// from `i.dat` file and whatever datastructure we use internally to contain
// the data relevant to elm-pair.
type DataMap<Key, Val> = Vec<(Key, Val)>;

#[derive(Debug)]
struct ElmCanonicalModuleName {
    package: ElmPackageName,
    module: ElmName,
}

#[derive(Debug)]
struct ElmPackageName {
    author: String,
    package: String,
}

#[derive(Debug)]
struct ElmName(String);

#[derive(Debug)]
enum ElmDependencyInterface {
    Public(ElmInterface),
    Private(
        ElmPackageName,
        DataMap<ElmName, ElmCanonicalUnion>,
        DataMap<ElmName, ElmCanonicalAlias>,
    ),
}

#[derive(Debug)]
struct ElmInterface {
    home: ElmPackageName,
    values: DataMap<ElmName, ElmCanonicalAnnotation>,
    unions: DataMap<ElmName, ElmUnion>,
    aliases: DataMap<ElmName, ElmAlias>,
    binops: DataMap<ElmName, ElmBinop>,
}

#[derive(Debug)]
struct ElmCanonicalUnion {
    vars: Vec<ElmName>,
    alts: Vec<ElmCtor>,
    numAlts: i64,
    opts: ElmCtorOpts,
}

#[derive(Debug)]
struct ElmCtor(ElmName, ElmIndexZeroBased, i64, Vec<ElmType>);

#[derive(Debug)]
struct ElmIndexZeroBased(i64);

#[derive(Debug)]
enum ElmCtorOpts {
    Normal,
    Enum,
    Unbox,
}

#[derive(Debug)]
struct ElmCanonicalAlias(Vec<ElmName>, ElmType);

#[derive(Debug)]
struct ElmCanonicalAnnotation(ElmFreeVars, ElmType);

type ElmFreeVars = DataMap<ElmName, ()>;

#[derive(Debug)]
enum ElmType {
    Lambda(Box<ElmType>, Box<ElmType>),
    Var(ElmName),
    Type(ElmCanonicalModuleName, ElmName, Vec<ElmType>),
    Record(DataMap<ElmName, ElmFieldType>, Option<ElmName>),
    Unit,
    Tuple(Box<ElmType>, Box<ElmType>, Option<ElmName>),
    Alias(
        ElmCanonicalModuleName,
        ElmName,
        Vec<(ElmName, ElmType)>,
        ElmAliasType,
    ),
}

#[derive(Debug)]
enum ElmUnion {
    Open(ElmCanonicalUnion),
    Closed(ElmCanonicalUnion),
    Private(ElmCanonicalUnion),
}

#[derive(Debug)]
enum ElmAlias {
    Public(ElmCanonicalAlias),
    Private(ElmCanonicalAlias),
}

#[derive(Debug)]
struct ElmBinop {
    name: ElmName,
    annotation: ElmCanonicalAnnotation,
    associativity: ElmBinopAssociativity,
    precedence: ElmBinopPrecedence,
}

#[derive(Debug)]
enum ElmBinopAssociativity {
    Left,
    Non,
    Right,
}

#[derive(Debug)]
struct ElmBinopPrecedence(i64);

#[derive(Debug)]
struct ElmFieldType(u16, ElmType);

#[derive(Debug)]
enum ElmAliasType {
    Holey(Box<ElmType>),
    Filled(Box<ElmType>),
}
