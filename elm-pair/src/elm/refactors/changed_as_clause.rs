use crate::elm::{Queries, Refactor};
use crate::lib::log::Error;
use crate::lib::source_code::SourceFileSnapshot;
use ropey::RopeSlice;
use tree_sitter::QueryCursor;

pub fn refactor(
    queries: &Queries,
    refactor: &mut Refactor,
    code: &SourceFileSnapshot,
    old_aliased_name: RopeSlice,
    new_aliased_name: RopeSlice,
) -> Result<(), Error> {
    let mut cursor = QueryCursor::new();
    for result in queries.query_for_qualified_values.run(&mut cursor, code) {
        let (node, reference) = result?;
        let old_qualifier_len = 1 + old_aliased_name.len_bytes();
        if reference.qualifier == old_aliased_name {
            refactor.add_change(
                node.start_byte()..(node.start_byte() + old_qualifier_len),
                format!("{}.", new_aliased_name),
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::elm::refactors::lib::simulations::simulation_test;

    simulation_test!(add_as_clause_to_import);
    simulation_test!(change_as_clause_of_import);
    simulation_test!(remove_as_clause_from_import);
}
