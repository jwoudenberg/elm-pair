use crate::elm::dependencies::DataflowComputation;
use crate::elm::{remove_qualifier_from_references, Import, Queries, Refactor};
use crate::support::log::Error;
use crate::support::source_code::SourceFileSnapshot;
use std::collections::HashSet;

pub fn refactor(
    queries: &Queries,
    computation: &mut DataflowComputation,
    refactor: &mut Refactor,
    code: &SourceFileSnapshot,
    import: Import,
) -> Result<(), Error> {
    let mut cursor = computation
        .exports_cursor(code.buffer, import.unaliased_name().to_string());
    let mut references_to_unqualify = HashSet::new();
    for result in import.exposing_list() {
        let (_, exposed) = result?;
        exposed.for_each_name(cursor.iter(), |reference| {
            references_to_unqualify.insert(reference);
        })
    }
    remove_qualifier_from_references(
        queries,
        computation,
        refactor,
        code,
        &import.aliased_name(),
        references_to_unqualify,
        None,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::elm::refactors::simulations::simulation_test;

    simulation_test!(add_exposing_list);
    simulation_test!(add_exposing_all_list);
}
