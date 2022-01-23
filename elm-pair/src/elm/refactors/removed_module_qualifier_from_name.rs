use crate::elm::dependencies::DataflowComputation;
use crate::elm::io::ExportedName;
use crate::elm::{
    add_to_exposing_list, get_import_by_aliased_name,
    remove_qualifier_from_references, Name, NameKind, QualifiedName, Queries,
    Refactor,
};
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
    let import =
        get_import_by_aliased_name(queries, code, &qualifier.slice(..))?;
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

#[cfg(test)]
mod tests {
    use crate::elm::refactors::simulations::simulation_test;

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
