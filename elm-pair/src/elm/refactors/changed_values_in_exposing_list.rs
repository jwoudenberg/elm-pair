use crate::elm::dependencies::DataflowComputation;
use crate::elm::{
    add_qualifier_to_references, remove_qualifier_from_references, Import,
    Queries, Refactor,
};
use crate::lib::log::Error;
use crate::lib::source_code::SourceFileSnapshot;
use std::collections::HashSet;
use tree_sitter::QueryCursor;

pub fn refactor(
    queries: &Queries,
    computation: &mut DataflowComputation,
    refactor: &mut Refactor,
    code: &SourceFileSnapshot,
    old_import: Import,
    new_import: Import,
) -> Result<(), Error> {
    let mut cursor = computation
        .exports_cursor(code.buffer, old_import.unaliased_name().to_string());
    let mut old_references = HashSet::new();
    for result in old_import.exposing_list() {
        let (_, exposed) = result?;
        exposed.for_each_name(cursor.iter(), |reference| {
            old_references.insert(reference);
        });
    }

    let mut new_references = HashSet::new();
    for result in new_import.exposing_list() {
        let (_, exposed) = result?;
        exposed.for_each_name(cursor.iter(), |reference| {
            new_references.insert(reference);
        });
    }

    let references_to_qualify = old_references
        .clone()
        .into_iter()
        .filter(|reference| !new_references.contains(reference))
        .collect();

    let references_to_unqualify = new_references
        .into_iter()
        .filter(|reference| !old_references.contains(reference))
        .collect();

    add_qualifier_to_references(
        queries,
        refactor,
        &mut QueryCursor::new(),
        code,
        None,
        &new_import,
        references_to_qualify,
    )?;

    remove_qualifier_from_references(
        queries,
        computation,
        refactor,
        code,
        &new_import.aliased_name(),
        references_to_unqualify,
        None,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::elm::refactors::lib::simulations::simulation_test;

    simulation_test!(remove_multiple_values_from_exposing_list_of_import);
    simulation_test!(remove_operator_from_exposing_list_of_import);
    simulation_test!(remove_type_with_constructor_from_exposing_list_of_import);
    simulation_test!(remove_value_from_exposing_list_of_import_with_as_clause);
    simulation_test!(remove_variable_from_exposing_list_of_import);
    simulation_test!(remove_record_type_alias_from_exposing_list_of_import);
    simulation_test!(add_record_type_alias_to_exposing_list_of_import);
    simulation_test!(add_value_to_exposing_list);
    simulation_test!(add_type_to_exposing_list);
    simulation_test!(add_type_exposing_constructors_to_exposing_list);
    simulation_test!( add_record_type_alias_with_same_name_as_local_constructor_to_exposing_list_of_import);
    simulation_test!( add_non_record_type_alias_with_same_name_as_local_constructor_to_exposing_list_of_import);
    simulation_test!(add_and_remove_items_in_exposing_list);
    simulation_test!(replace_exposing_list_with_double_dot);
    simulation_test!(replace_double_dot_with_exposing_list);
    simulation_test!(
        add_value_to_exposing_list_with_same_name_as_local_variable
    );
    simulation_test!(
        add_value_to_exposing_list_with_same_name_as_top_level_function
    );
    simulation_test!(
        expose_value_with_same_name_as_exposed_value_from_other_module
    );
    simulation_test!(
        expose_value_with_same_name_as_value_from_other_module_exposing_all
    );

    // --- TESTS DEMONSTRATING CURRENT BUGS ---
    // When we expose a value with the same name as a local variable the local
    // variable gets renamed to something else. This test demonstrates an edge
    // case in this logic where the renaming logic is failing. When we expose
    // multiple variables at the same time, one of which has the same name as
    // a local variable and the other which has the name we would rename the
    // local variable too, then we still end up with a naming conflict when all
    // is done.
    simulation_test!( add_value_to_exposing_list_of_import_with_same_name_as_local_variable_and_another_with_the_same_name_plus_trailing_2);
}
