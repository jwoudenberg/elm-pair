use crate::elm::dependencies::DataflowComputation;
use crate::elm::{
    add_qualifier_to_references, constructors_of_exports, ExposedConstructors,
    Import, Name, NameKind, Queries, Refactor,
};
use crate::support::log::Error;
use crate::support::source_code::SourceFileSnapshot;
use ropey::{Rope, RopeSlice};
use std::collections::HashSet;
use tree_sitter::QueryCursor;

pub fn refactor(
    queries: &Queries,
    computation: &mut DataflowComputation,
    refactor: &mut Refactor,
    code: &SourceFileSnapshot,
    old_import: Import,
    type_name: RopeSlice,
) -> Result<(), Error> {
    let mut references_to_qualify = HashSet::new();
    let mut cursor = computation
        .exports_cursor(code.buffer, old_import.unaliased_name().to_string());
    match constructors_of_exports(cursor.iter(), type_name)? {
        ExposedConstructors::FromTypeAlias(ctor) => {
            references_to_qualify.insert(Name {
                name: Rope::from_str(ctor),
                kind: NameKind::Constructor,
            });
        }
        ExposedConstructors::FromCustomType(ctors) => {
            for ctor in ctors {
                references_to_qualify.insert(Name {
                    name: Rope::from_str(ctor),
                    kind: NameKind::Constructor,
                });
            }
        }
    }
    add_qualifier_to_references(
        queries,
        refactor,
        &mut QueryCursor::new(),
        code,
        None,
        &old_import,
        references_to_qualify,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::elm::refactors::simulations::simulation_test;

    simulation_test!(remove_constructor_from_exposing_list_of_import);
}
