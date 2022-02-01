use crate::elm::compiler::Compiler;
use crate::elm::io::ExportedName;
use crate::elm::project;
use crate::lib::log;
use crate::lib::log::Error;
use byteorder::{BigEndian, ReadBytesExt};
use std::io::BufReader;
use std::io::Read;
use std::iter::FromIterator;
use std::path::Path;

pub fn parse_elm_stuff_idat(
    compiler: &Compiler,
    path: &Path,
) -> Result<impl Iterator<Item = (String, ExportedName)>, Error> {
    let file = std::fs::File::open(path).or_else(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            let project_root = project::root_from_idat_path(path)?;
            create_elm_stuff(compiler, project_root)?;
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
    let exports = parse(reader)?
        .into_iter()
        .filter_map(|(canonical_name, dep_i)| {
            if let DependencyInterface::Public(interface) = dep_i {
                Some((canonical_name, interface))
            } else {
                None
            }
        })
        .flat_map(|(canonical_name, i)| {
            let Name(name) = canonical_name.module;
            elm_module_from_interface(i)
                .into_iter()
                .map(move |export| (name.clone(), export))
        });
    Ok(exports)
}

fn create_elm_stuff(
    compiler: &Compiler,
    project_root: &Path,
) -> Result<(), Error> {
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
    let output = compiler.make(project_root, &temp_module)?;
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
    interface: Interface,
) -> impl Iterator<Item = ExportedName> {
    // TODO: add binops
    let values = interface.values.into_iter().map(elm_export_from_value);
    let unions = interface.unions.into_iter().map(elm_export_from_union);
    let aliases = interface.aliases.into_iter().map(elm_export_from_alias);
    values.chain(unions).chain(aliases)
}

fn elm_export_from_value(
    (Name(name), _): (Name, CanonicalAnnotation),
) -> ExportedName {
    ExportedName::Value { name }
}

fn elm_export_from_union((Name(name), union): (Name, Union)) -> ExportedName {
    let constructors = match union {
        Union::Open(canonical_union) => {
            let iter = canonical_union
                .alts
                .into_iter()
                .map(|Ctor(Name(name), _, _, _)| name);
            Vec::from_iter(iter)
        }
        // We're reading this information for use by other modules.
        // These external modules can't see private constructors,
        // so we don't need to return them here.
        Union::Closed(_) => Vec::new(),
        Union::Private(_) => Vec::new(),
    };
    ExportedName::Type { name, constructors }
}

fn elm_export_from_alias((Name(name), _): (Name, Alias)) -> ExportedName {
    ExportedName::Type {
        name,
        constructors: Vec::new(),
    }
}

fn parse<R: Read>(
    reader: R,
) -> Result<DataMap<CanonicalModuleName, DependencyInterface>, Error> {
    let mut parser = IdatParser { reader };
    parser.data_binary_map(
        IdatParser::elm_canonical_module_name,
        IdatParser::elm_dependency_interface,
    )
}

struct IdatParser<R> {
    reader: R,
}

impl<R: Read> IdatParser<R> {
    fn data_binary_int(&mut self) -> Result<i64, Error> {
        self.reader.read_i64::<BigEndian>().map_err(|err| {
            log::mk_err!("error reading i64 from i.dat: {:?}", err)
        })
    }

    fn data_binary_word8(&mut self) -> Result<u8, Error> {
        let mut bytes = [0];
        self.reader.read_exact(&mut bytes).map_err(|err| {
            log::mk_err!("error reading u8 from i.dat: {:?}", err)
        })?;
        Ok(bytes[0])
    }

    fn data_binary_word16(&mut self) -> Result<u16, Error> {
        self.reader.read_u16::<BigEndian>().map_err(|err| {
            log::mk_err!("error reading u16 from i.dat: {:?}", err)
        })
    }

    fn elm_utf8_under256(&mut self) -> Result<String, Error> {
        let len = self.data_binary_word8()? as usize;
        let mut full_bytes = [0; 256];
        let bytes = &mut full_bytes[0..len];
        self.reader.read_exact(bytes).map_err(|err| {
            log::mk_err!("error reading text from i.dat: {:?}", err)
        })?;
        let str = std::str::from_utf8(bytes).map_err(|err| {
            log::mk_err!("error decoding i.dat bytest as utf8: {:?}", err)
        })?;
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
            _ => Err(log::mk_err!(
                "encountered unexpected kind {:?} reading Maybe from i.dat",
                kind
            )),
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
    ) -> Result<CanonicalModuleName, Error> {
        let name = CanonicalModuleName {
            package: self.elm_package_name()?,
            module: self.elm_name()?,
        };
        Ok(name)
    }

    fn elm_package_name(&mut self) -> Result<PackageName, Error> {
        let name = PackageName {
            author: self.elm_utf8_under256()?,
            package: self.elm_utf8_under256()?,
        };
        Ok(name)
    }

    fn elm_name(&mut self) -> Result<Name, Error> {
        let name = self.elm_utf8_under256()?;
        Ok(Name(name))
    }

    fn elm_dependency_interface(
        &mut self,
    ) -> Result<DependencyInterface, Error> {
        let kind = self.data_binary_word8()?;
        match kind {
            0 => {
                let interface = self.elm_interface()?;
                Ok(DependencyInterface::Public(interface))
            }
            1 => {
                let package_name = self.elm_package_name()?;
                let unions = self.data_binary_map(Self::elm_name, Self::elm_canonical_union)?;
                let aliases = self.data_binary_map(Self::elm_name, Self::elm_canonical_alias)?;
                Ok(DependencyInterface::Private(package_name, unions, aliases))
            }
            _ => Err(log::mk_err!(
                "encountered unexpected kind {:?} reading DependencyInterface from i.dat",
                kind
            )),
        }
    }

    fn elm_interface(&mut self) -> Result<Interface, Error> {
        let interface = Interface {
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

    fn elm_canonical_union(&mut self) -> Result<CanonicalUnion, Error> {
        let vars = self.data_binary_list(Self::elm_name)?;
        let alts = self.data_binary_list(Self::elm_ctor)?;
        let num_alts = self.data_binary_int()?;
        let opts = self.elm_ctor_opts()?;
        Ok(CanonicalUnion {
            vars,
            alts,
            num_alts,
            opts,
        })
    }

    fn elm_ctor(&mut self) -> Result<Ctor, Error> {
        let name = self.elm_name()?;
        let index = self.elm_index_zero_based()?;
        let len = self.data_binary_int()?;
        let args = self.data_binary_list(Self::elm_type)?;
        Ok(Ctor(name, index, len, args))
    }

    fn elm_index_zero_based(&mut self) -> Result<IndexZeroBased, Error> {
        let index = self.data_binary_int()?;
        Ok(IndexZeroBased(index))
    }

    fn elm_ctor_opts(&mut self) -> Result<CtorOpts, Error> {
        let kind = self.data_binary_word8()?;
        match kind {
            0 => Ok(CtorOpts::Normal),
            1 => Ok(CtorOpts::Enum),
            2 => Ok(CtorOpts::Unbox),
            _ => Err(log::mk_err!(
                "encountered unexpected kind {:?} reading CtorOpts from i.dat",
                kind
            )),
        }
    }

    fn elm_canonical_alias(&mut self) -> Result<CanonicalAlias, Error> {
        let names = self.data_binary_list(Self::elm_name)?;
        let type_ = self.elm_type()?;
        Ok(CanonicalAlias(names, type_))
    }

    fn elm_canonical_annotation(
        &mut self,
    ) -> Result<CanonicalAnnotation, Error> {
        let free_vars = self.data_binary_map(Self::elm_name, |_| Ok(()))?;
        let type_ = self.elm_type()?;
        Ok(CanonicalAnnotation(free_vars, type_))
    }

    fn elm_union(&mut self) -> Result<Union, Error> {
        let kind = self.data_binary_word8()?;
        match kind {
            0 => self.elm_canonical_union().map(Union::Open),
            1 => self.elm_canonical_union().map(Union::Closed),
            2 => self.elm_canonical_union().map(Union::Private),
            _ => Err(log::mk_err!(
                "encountered unexpected kind {:?} reading Union from i.dat",
                kind
            )),
        }
    }

    fn elm_alias(&mut self) -> Result<Alias, Error> {
        let kind = self.data_binary_word8()?;
        match kind {
            0 => self.elm_canonical_alias().map(Alias::Public),
            1 => self.elm_canonical_alias().map(Alias::Private),
            _ => Err(log::mk_err!(
                "encountered unexpected kind {:?} reading Alias from i.dat",
                kind
            )),
        }
    }

    fn elm_binop(&mut self) -> Result<Binop, Error> {
        let name = self.elm_name()?;
        let annotation = self.elm_canonical_annotation()?;
        let associativity = self.elm_binop_associativity()?;
        let precedence = self.elm_binop_precedence()?;
        Ok(Binop {
            name,
            annotation,
            associativity,
            precedence,
        })
    }

    fn elm_binop_associativity(&mut self) -> Result<BinopAssociativity, Error> {
        let kind = self.data_binary_word8()?;
        match kind {
            0 => Ok(BinopAssociativity::Left),
            1 => Ok(BinopAssociativity::Non),
            2 => Ok(BinopAssociativity::Right),
            _ => Err(log::mk_err!(
                "encountered unexpected kind {:?} reading BinopAssocioativity from i.dat",
                kind
            )),
        }
    }

    fn elm_binop_precedence(&mut self) -> Result<BinopPrecedence, Error> {
        let n = self.data_binary_int()?;
        Ok(BinopPrecedence(n))
    }

    fn elm_type(&mut self) -> Result<Type, Error> {
        let kind = self.data_binary_word8()?;
        match kind {
            // Lambda
            0 => {
                let a = self.elm_type()?;
                let b = self.elm_type()?;
                Ok(Type::Lambda(Box::new(a), Box::new(b)))
            }
            // Var
            1 => {
                let name = self.elm_name()?;
                Ok(Type::Var(name))
            }
            // Record
            2 => {
                let vals =
                    self.data_binary_map(Self::elm_name, Self::elm_field_type)?;
                let name = self.data_binary_maybe(Self::elm_name)?;
                Ok(Type::Record(vals, name))
            }
            // Unit
            3 => Ok(Type::Unit),
            // Tuple
            4 => {
                let a = self.elm_type()?;
                let b = self.elm_type()?;
                let name = self.data_binary_maybe(Self::elm_name)?;
                Ok(Type::Tuple(Box::new(a), Box::new(b), name))
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
                Ok(Type::Alias(module_name, name, types, alias_type))
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
                Ok(Type::Type(module_name, name, ctors))
            }
        }
    }

    fn elm_field_type(&mut self) -> Result<FieldType, Error> {
        let index = self.data_binary_word16()?;
        let type_ = self.elm_type()?;
        Ok(FieldType(index, type_))
    }

    fn elm_alias_type(&mut self) -> Result<AliasType, Error> {
        let kind = self.data_binary_word8()?;
        match kind {
            0 => {
                let type_ = self.elm_type()?;
                Ok(AliasType::Holey(Box::new(type_)))
            }
            1 => {
                let type_ = self.elm_type()?;
                Ok(AliasType::Filled(Box::new(type_)))
            }
            _ => Err(log::mk_err!(
                "encountered unexpected kind {:?} reading AliasType from i.dat",
                kind
            )),
        }
    }
}

// We currently represent Haskell's Data.Map type as a vector of tuples, to
// avoid the key constraints using a HashMap would involve. The  types here
// are not intended for direct use, only as a waystation between data read
// from `i.dat` file and whatever datastructure we use internally to contain
// the data relevant to elm-pair.
type DataMap<Key, Val> = Vec<(Key, Val)>;

#[allow(dead_code)]
struct CanonicalModuleName {
    package: PackageName,
    module: Name,
}

#[allow(dead_code)]
struct PackageName {
    author: String,
    package: String,
}

struct Name(String);

enum DependencyInterface {
    Public(Interface),
    Private(
        PackageName,
        DataMap<Name, CanonicalUnion>,
        DataMap<Name, CanonicalAlias>,
    ),
}

#[allow(dead_code)]
struct Interface {
    home: PackageName,
    values: DataMap<Name, CanonicalAnnotation>,
    unions: DataMap<Name, Union>,
    aliases: DataMap<Name, Alias>,
    binops: DataMap<Name, Binop>,
}

#[allow(dead_code)]
struct CanonicalUnion {
    vars: Vec<Name>,
    alts: Vec<Ctor>,
    num_alts: i64,
    opts: CtorOpts,
}

struct Ctor(Name, IndexZeroBased, i64, Vec<Type>);

struct IndexZeroBased(i64);

enum CtorOpts {
    Normal,
    Enum,
    Unbox,
}

struct CanonicalAlias(Vec<Name>, Type);

struct CanonicalAnnotation(FreeVars, Type);

type FreeVars = DataMap<Name, ()>;

#[allow(clippy::enum_variant_names)]
enum Type {
    Lambda(Box<Type>, Box<Type>),
    Var(Name),
    Type(CanonicalModuleName, Name, Vec<Type>),
    Record(DataMap<Name, FieldType>, Option<Name>),
    Unit,
    Tuple(Box<Type>, Box<Type>, Option<Name>),
    Alias(CanonicalModuleName, Name, Vec<(Name, Type)>, AliasType),
}

enum Union {
    Open(CanonicalUnion),
    Closed(CanonicalUnion),
    Private(CanonicalUnion),
}

enum Alias {
    Public(CanonicalAlias),
    Private(CanonicalAlias),
}

#[allow(dead_code)]
struct Binop {
    name: Name,
    annotation: CanonicalAnnotation,
    associativity: BinopAssociativity,
    precedence: BinopPrecedence,
}

enum BinopAssociativity {
    Left,
    Non,
    Right,
}

struct BinopPrecedence(i64);

struct FieldType(u16, Type);

enum AliasType {
    Holey(Box<Type>),
    Filled(Box<Type>),
}
