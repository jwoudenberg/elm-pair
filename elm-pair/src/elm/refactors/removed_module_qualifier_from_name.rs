use crate::elm::dependencies::DataflowComputation;
use crate::elm::io::ExportedName;
use crate::elm::queries::imports::{ExposedName, Import};
use crate::elm::refactors::lib::remove_qualifier_from_references::remove_qualifier_from_references;
use crate::elm::{Name, NameKind, QualifiedName, Queries, Refactor};

use crate::lib::log::Error;
use crate::lib::source_code::SourceFileSnapshot;
use ropey::Rope;
use std::collections::HashSet;
use tree_sitter::Node;

pub fn refactor(
    queries: &Queries,
    computation: &mut DataflowComputation,
    refactor: &mut Refactor,
    code: &SourceFileSnapshot,
    node: Node,
    node_name: QualifiedName,
) -> Result<(), Error> {
    let QualifiedName {
        unqualified_name,
        qualifier,
    } = node_name;
    let import = queries
        .query_for_imports
        .by_aliased_name(code, &qualifier.slice(..))?;
    let mut references_to_unqualify = HashSet::new();
    if unqualified_name.kind == NameKind::Constructor {
        let mut cursor = computation
            .exports_cursor(code.buffer, import.unaliased_name().to_string());
        for export in cursor.iter() {
            match export {
                ExportedName::Value { .. } => {}
                ExportedName::RecordTypeAlias { name } => {
                    // We're dealing here with a type alias being used as a
                    // constructor. For example, given a type alias like:
                    //
                    //     type alias Point = { x : Int, y : Int }
                    //
                    // constructor usage would be doing this:
                    //
                    //     point = Point 7 2
                    if name == &unqualified_name.name.to_string() {
                        references_to_unqualify.insert(Name {
                            kind: NameKind::Constructor,
                            name: Rope::from_str(name),
                        });
                        add_to_exposing_list(
                            &import,
                            &Name {
                                kind: NameKind::Type,
                                name: unqualified_name.name,
                            },
                            None,
                            refactor,
                        )?;
                        break;
                    }
                }
                ExportedName::Type { name, constructors } => {
                    if constructors.contains(&unqualified_name.name.to_string())
                    {
                        for ctor in constructors.iter() {
                            references_to_unqualify.insert(Name {
                                kind: NameKind::Constructor,
                                name: Rope::from_str(ctor),
                            });
                        }
                        add_to_exposing_list(
                            &import,
                            &unqualified_name,
                            Some(name),
                            refactor,
                        )?;
                        references_to_unqualify.insert(unqualified_name);
                        break;
                    }
                }
            }
        }
    } else {
        add_to_exposing_list(&import, &unqualified_name, None, refactor)?;
        references_to_unqualify.insert(unqualified_name);
    };
    remove_qualifier_from_references(
        queries,
        computation,
        refactor,
        code,
        &qualifier.slice(..),
        references_to_unqualify,
        Some(node),
    )?;
    Ok(())
}

// Add a name to the list of values exposed from a particular module.
fn add_to_exposing_list(
    import: &Import,
    reference: &Name,
    ctor_type: Option<&String>,
    refactor: &mut Refactor,
) -> Result<(), Error> {
    let (target_exposed_name, insert_str) = match ctor_type {
        Some(type_name) => (type_name.to_owned(), format!("{}(..)", type_name)),
        None => (reference.name.to_string(), reference.name.to_string()),
    };

    let mut last_node = None;

    // Find the first node in the existing exposing list alphabetically
    // coming after the node we're looking to insert, then insert in
    // front of that node.
    for result in import.exposing_list() {
        let (node, exposed) = result?;
        let exposed_name = match exposed {
            ExposedName::Operator(op) => op.name,
            ExposedName::Value(val) => val.name,
            ExposedName::Type(type_) => type_.name,
            ExposedName::All => {
                return Ok(());
            }
        };
        last_node = Some(node);
        // Insert right before this item to maintain alphabetic order.
        // If the exposing list wasn't ordered alphabetically the insert
        // place might appear random.
        match std::cmp::Ord::cmp(
            &target_exposed_name,
            &exposed_name.to_string(),
        ) {
            std::cmp::Ordering::Equal => {
                if ctor_type.is_some() {
                    // node.child(1) is the node corresponding to the exposed
                    // contructors: `(..)`.
                    if node.child(1).is_none() {
                        let insert_at = node.end_byte();
                        refactor.add_change(
                            insert_at..insert_at,
                            "(..)".to_string(),
                        );
                    }
                };
                return Ok(());
            }
            std::cmp::Ordering::Less => {
                let insert_at = node.start_byte();
                refactor.add_change(
                    insert_at..insert_at,
                    format!("{}, ", insert_str),
                );
                return Ok(());
            }
            std::cmp::Ordering::Greater => {}
        }
    }

    // We didn't find anything in the exposing list alphabetically
    // after us. Either we come alphabetically after all currently
    // exposed elements, or there is no exposing list at all.
    match last_node {
        None => {
            refactor.add_change(
                import.root_node.end_byte()..import.root_node.end_byte(),
                format!(" exposing ({})", insert_str),
            );
        }
        Some(node) => {
            let insert_at = node.end_byte();
            refactor
                .add_change(insert_at..insert_at, format!(", {}", insert_str));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::elm::refactors::lib::simulations::simulation_test;

    simulation_test!(remove_module_qualifier_from_variable);
    simulation_test!(remove_module_qualifier_from_type);
    simulation_test!(
        remove_module_qualifier_inserting_variable_at_end_of_exposing_list
    );
    simulation_test!(remove_module_qualifier_for_module_without_exposing_list);
    simulation_test!(remove_module_qualifier_for_module_exposing_all);
    simulation_test!(remove_module_qualifier_from_constructor);
    simulation_test!(remove_module_qualifier_from_exposed_constructor);
    simulation_test!(remove_module_qualifier_from_constructor_of_exposed_type);
    simulation_test!(
        remove_module_qualifier_from_record_type_alias_used_as_function
    );
    simulation_test!(
        remove_module_qualifier_from_variable_with_same_name_as_local_variable
    );
    simulation_test!( remove_module_qualifier_from_variable_with_same_name_as_value_exposed_from_other_module);
    simulation_test!(
        remove_module_qualifier_from_type_with_same_name_as_local_type_alias
    );
    simulation_test!(
        remove_module_qualifier_from_type_with_same_name_as_local_type
    );
    simulation_test!(remove_module_qualifier_from_constructor_with_same_name_as_local_constructor);
}
