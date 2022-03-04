use crate::elm::dependencies::DataflowComputation;
use crate::elm::io::ExportedName;
use crate::elm::queries::imports::{ExposedConstructors, ExposedName, Import};
use crate::elm::refactors::lib::add_qualifier_to_references::add_qualifier_to_references;
use crate::elm::refactors::lib::constructors_of_exports::constructors_of_exports;
use crate::elm::{Name, NameKind, Queries, Refactor, COMMA};
use crate::lib::log;
use crate::lib::log::Error;
use crate::lib::source_code::SourceFileSnapshot;
use ropey::Rope;
use std::collections::HashSet;
use std::ops::Range;
use tree_sitter::{Node, QueryCursor};

pub fn qualify_value(
    queries: &Queries,
    computation: &mut DataflowComputation,
    refactor: &mut Refactor,
    code: &SourceFileSnapshot,
    skip_byteranges: &[&Range<usize>],
    qualifier: &Rope,
    reference: &Name,
    // If the qualified value is coming from an import that exposing everything,
    // then this boolean decides whether to keep the `exposing (..)` clause as
    // is, or whether to replace it with an explicit list of currently used
    // values minus the now qualified value.
    remove_expose_all_if_necessary: bool,
) -> Result<(), Error> {
    let import = queries
        .query_for_imports
        .by_aliased_name(code, &qualifier.slice(..))?;
    let exposing_list_length = import.exposing_list().count();
    let mut references_to_qualify = HashSet::new();
    for result in import.exposing_list() {
        let (node, exposed) = result?;
        match &exposed {
            ExposedName::Operator(op) => {
                if op.name == reference.name
                    && reference.kind == NameKind::Operator
                {
                    return Err(log::mk_err!(
                        "cannot qualify operator, Elm doesn't allow it!"
                    ));
                }
            }
            ExposedName::Type(type_) => {
                if type_.name == reference.name
                    && reference.kind == NameKind::Type
                {
                    if exposing_list_length == 1 {
                        remove_exposing_list(refactor, &import);
                    } else {
                        remove_from_exposing_list(refactor, &node)?;
                    }
                    references_to_qualify.insert(Name {
                        name: type_.name.into(),
                        kind: NameKind::Type,
                    });
                }

                let mut cursor = computation.exports_cursor(
                    code.buffer,
                    import.unaliased_name().to_string(),
                );
                match constructors_of_exports(cursor.iter(), type_.name)? {
                    ExposedConstructors::FromTypeAlias(ctor) => {
                        if ctor == &reference.name {
                            // Ensure we don't remove the item from the exposing
                            // list twice (see code above).
                            // TODO: Clean this up.
                            if reference.kind != NameKind::Type {
                                if exposing_list_length == 1 {
                                    remove_exposing_list(refactor, &import);
                                } else {
                                    remove_from_exposing_list(refactor, &node)?;
                                }
                            }
                            references_to_qualify.insert(Name {
                                name: Rope::from_str(ctor),
                                kind: NameKind::Type,
                            });
                            references_to_qualify.insert(Name {
                                name: Rope::from_str(ctor),
                                kind: NameKind::Constructor,
                            });
                        }
                    }
                    ExposedConstructors::FromCustomType(ctors) => {
                        if ctors.iter().any(|ctor| *ctor == reference.name) {
                            // Remove `(..)` behind type from constructor this.
                            let exposing_ctors_node = node.child(1).ok_or_else(|| {
                                log::mk_err!("could not find `(..)` node behind exposed type")
                            })?;
                            refactor.add_change(
                                exposing_ctors_node.byte_range(),
                                String::new(),
                            );

                            // We're qualifying a constructor. In Elm you can only
                            // expose either all constructors of a type or none of them,
                            // so if the programmer qualifies one constructor assume
                            // intend to do them all.
                            let constructor_references =
                                ctors.iter().map(|ctor| Name {
                                    name: Rope::from_str(ctor),
                                    kind: NameKind::Constructor,
                                });
                            references_to_qualify
                                .extend(constructor_references);
                        }
                    }
                }
            }
            ExposedName::Value(val) => {
                if val.name == reference.name
                    && reference.kind == NameKind::Value
                {
                    if exposing_list_length == 1 {
                        remove_exposing_list(refactor, &import);
                    } else {
                        remove_from_exposing_list(refactor, &node)?;
                    }
                    references_to_qualify.insert(Name {
                        name: val.name.into(),
                        kind: NameKind::Value,
                    });
                    break;
                }
            }
            ExposedName::All => {
                if remove_expose_all_if_necessary {
                    let mut exposed_names: Vec<(Name, &ExportedName)> =
                        Vec::new();
                    let mut cursor = computation.exports_cursor(
                        code.buffer,
                        import.unaliased_name().to_string(),
                    );
                    cursor.iter().for_each(|export| match export {
                        ExportedName::Value { name } => {
                            exposed_names.push((
                                Name {
                                    name: Rope::from_str(name),
                                    kind: NameKind::Value,
                                },
                                export,
                            ));
                        }
                        ExportedName::RecordTypeAlias { name } => {
                            exposed_names.push((
                                Name {
                                    name: Rope::from_str(name),
                                    kind: NameKind::Type,
                                },
                                export,
                            ));
                            exposed_names.push((
                                Name {
                                    name: Rope::from_str(name),
                                    kind: NameKind::Constructor,
                                },
                                export,
                            ));
                        }
                        ExportedName::Type { name, constructors } => {
                            exposed_names.push((
                                Name {
                                    name: Rope::from_str(name),
                                    kind: NameKind::Type,
                                },
                                export,
                            ));
                            for ctor in constructors {
                                exposed_names.push((
                                    Name {
                                        name: Rope::from_str(ctor),
                                        kind: NameKind::Constructor,
                                    },
                                    export,
                                ));
                            }
                        }
                    });
                    let mut cursor = QueryCursor::new();
                    let mut unqualified_names_in_use: HashSet<Name> = queries
                        .query_for_unqualified_values
                        .run(&mut cursor, code)
                        .map(|r| r.map(|(_, _, reference)| reference))
                        .collect::<Result<HashSet<Name>, Error>>()?;
                    unqualified_names_in_use.remove(reference);
                    let mut new_exposed: String = String::new();
                    exposed_names.sort_by_key(|(name, _)| name.name.clone());
                    exposed_names.into_iter().for_each(
                        |(reference, export)| {
                            if unqualified_names_in_use.contains(&reference) {
                                if !new_exposed.is_empty() {
                                    new_exposed.push_str(", ")
                                }
                                match export {
                                    ExportedName::Value { name } => {
                                        new_exposed.push_str(name);
                                    }
                                    ExportedName::RecordTypeAlias { name } => {
                                        new_exposed.push_str(name);
                                    }
                                    ExportedName::Type { name, .. } => {
                                        if reference.kind
                                            == NameKind::Constructor
                                        {
                                            new_exposed.push_str(&format!(
                                                "{}(..)",
                                                name
                                            ));
                                        } else {
                                            new_exposed.push_str(name);
                                        }
                                    }
                                }
                            }
                        },
                    );
                    refactor.add_change(node.byte_range(), new_exposed);
                }

                match reference.kind {
                    NameKind::Operator => {
                        return Err(log::mk_err!(
                            "cannot qualify operator, Elm doesn't allow it!"
                        ));
                    }
                    NameKind::Value | NameKind::Type => {
                        references_to_qualify.insert(reference.clone());
                    }
                    NameKind::Constructor => {
                        // We know a constructor got qualified, but not which
                        // type it belongs too. To find it, we iterate over all
                        // the exports from the module matching the qualifier we
                        // added. The type must be among them!
                        let mut cursor = computation.exports_cursor(
                            code.buffer,
                            import.unaliased_name().to_string(),
                        );
                        for export in cursor.iter() {
                            match export {
                                ExportedName::Value { .. } => {}
                                ExportedName::RecordTypeAlias { .. } => {}
                                ExportedName::Type { constructors, .. } => {
                                    if constructors
                                        .iter()
                                        .any(|ctor| *ctor == reference.name)
                                    {
                                        let constructor_references =
                                            constructors.iter().map(|ctor| {
                                                Name {
                                                    name: Rope::from_str(ctor),
                                                    kind: NameKind::Constructor,
                                                }
                                            });
                                        references_to_qualify
                                            .extend(constructor_references);
                                    }
                                }
                            }
                        }
                        break;
                    }
                }
            }
        }
    }
    add_qualifier_to_references(
        queries,
        refactor,
        &mut QueryCursor::new(),
        code,
        skip_byteranges,
        &import,
        references_to_qualify,
    )?;
    Ok(())
}

fn remove_exposing_list(refactor: &mut Refactor, import: &Import) {
    match import.exposing_list_node {
        None => {}
        Some(node) => refactor.add_change(node.byte_range(), String::new()),
    };
}

fn remove_from_exposing_list(
    refactor: &mut Refactor,
    node: &Node,
) -> Result<(), Error> {
    // TODO: Automatically clean up extra or missing comma's.
    let range_including_comma_and_whitespace = |exposed_node: &Node| {
        let next = exposed_node.next_sibling();
        if let Some(node) = next {
            if node.kind_id() == COMMA {
                let end_byte = match node.next_sibling() {
                    Some(next) => next.start_byte(),
                    None => node.end_byte(),
                };
                return exposed_node.start_byte()..end_byte;
            }
        }
        let prev = exposed_node.prev_sibling();
        if let Some(node) = prev {
            if node.kind_id() == COMMA {
                let start_byte = match node.prev_sibling() {
                    Some(prev) => prev.end_byte(),
                    None => node.start_byte(),
                };
                return start_byte..exposed_node.end_byte();
            }
        }
        exposed_node.byte_range()
    };
    refactor
        .add_change(range_including_comma_and_whitespace(node), String::new());
    Ok(())
}
