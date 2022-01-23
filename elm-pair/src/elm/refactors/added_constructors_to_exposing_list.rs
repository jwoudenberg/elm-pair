use crate::elm::dependencies::DataflowComputation;
use crate::elm::io::ExportedName;
use crate::elm::refactors::lib::remove_qualifier_from_references::remove_qualifier_from_references;
use crate::elm::{Import, Name, NameKind, Queries, Refactor};
use crate::lib::log::Error;
use crate::lib::source_code::SourceFileSnapshot;
use ropey::{Rope, RopeSlice};
use std::collections::HashSet;

pub fn refactor(
    queries: &Queries,
    computation: &mut DataflowComputation,
    refactor: &mut Refactor,
    code: &SourceFileSnapshot,
    import: Import,
    type_name: RopeSlice,
) -> Result<(), Error> {
    let mut cursor = computation
        .exports_cursor(code.buffer, import.unaliased_name().to_string());
    let mut references_to_unqualify = HashSet::new();
    for export in cursor.iter() {
        if let ExportedName::Type { name, constructors } = export {
            if name == &type_name {
                references_to_unqualify.extend(constructors.iter().map(
                    |ctor| Name {
                        name: Rope::from_str(ctor),
                        kind: NameKind::Constructor,
                    },
                ));
                break;
            }
        }
    }
    remove_qualifier_from_references(
        queries,
        computation,
        refactor,
        code,
        &import.aliased_name(),
        references_to_unqualify,
        None,
    )
}

#[cfg(test)]
mod tests {
    use crate::elm::refactors::lib::simulations::simulation_test;

    simulation_test!(add_constructors_for_type_to_exposing_list);
}
