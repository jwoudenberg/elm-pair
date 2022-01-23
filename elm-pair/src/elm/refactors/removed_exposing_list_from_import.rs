use crate::elm::dependencies::DataflowComputation;
use crate::elm::{add_qualifier_to_references, Import, Queries, Refactor};
use crate::lib::log::Error;
use crate::lib::source_code::SourceFileSnapshot;
use std::collections::HashSet;
use tree_sitter::QueryCursor;

pub fn refactor(
    queries: &Queries,
    computation: &mut DataflowComputation,
    refactor: &mut Refactor,
    code: &SourceFileSnapshot,
    import: Import,
) -> Result<(), Error> {
    let mut val_cursor = QueryCursor::new();
    let mut cursor = computation
        .exports_cursor(code.buffer, import.unaliased_name().to_string());
    let mut references_to_qualify = HashSet::new();
    for result in import.exposing_list() {
        let (_, exposed) = result?;
        exposed.for_each_name(cursor.iter(), |reference| {
            references_to_qualify.insert(reference);
        });
    }
    add_qualifier_to_references(
        queries,
        refactor,
        &mut val_cursor,
        code,
        None,
        &import,
        references_to_qualify,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::elm::refactors::lib::simulations::simulation_test;

    simulation_test!(remove_exposing_all_clause_from_import);
    simulation_test!(remove_exposing_all_clause_from_local_import);
    simulation_test!(remove_exposing_clause_from_import);
    simulation_test!(remove_exposing_clause_from_import_with_as_clause);

    // --- TESTS DEMONSTRATING CURRENT BUGS --
    // The exposing lists in these tests contained an operator. It doesn't get a
    // qualifier because Elm doesn't allow qualified operators, and as a result
    // this refactor doesn't produce compiling code.
    // Potential fix: Add the exposing list back containing just the operator.
    simulation_test!(remove_exposing_clause_containing_operator_from_import);
    simulation_test!(
        remove_exposing_all_clause_containing_operator_from_import
    );
}
