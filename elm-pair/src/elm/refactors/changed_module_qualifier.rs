use crate::elm::refactors;
use crate::elm::{QualifiedName, Queries, Refactor};
use crate::lib::log;
use crate::lib::log::Error;
use crate::lib::source_code::SourceFileSnapshot;
use tree_sitter::QueryCursor;

pub fn refactor(
    queries: &Queries,
    refactor: &mut Refactor,
    code: &SourceFileSnapshot,
    old_name: QualifiedName,
    new_name: QualifiedName,
) -> Result<(), Error> {
    let mut cursor = QueryCursor::new();
    let import = queries
        .query_for_imports
        .run(&mut cursor, code)
        .find(|import| import.aliased_name() == old_name.qualifier)
        .ok_or_else(|| {
            log::mk_err!(
                "did not find import statement with the expected aliased name"
            )
        })?;
    match import.as_clause_node {
        Some(as_clause_name_node) => {
            if import.unaliased_name() == new_name.qualifier {
                let as_clause_node =
                    as_clause_name_node.parent().ok_or_else(|| {
                        log::mk_err!(
                            "found unexpected root as clause name nood"
                        )
                    })?;
                refactor.add_change(
                    (as_clause_node.start_byte() - 1)
                        ..as_clause_node.end_byte(),
                    String::new(),
                )
            } else {
                refactor.add_change(
                    as_clause_name_node.byte_range(),
                    new_name.qualifier.to_string(),
                )
            }
        }
        None => {
            let insert_point = import.name_node.end_byte();
            refactor.add_change(
                insert_point..insert_point,
                format!(" as {}", new_name.qualifier),
            );
        }
    }

    refactors::changed_as_clause::refactor(
        queries,
        refactor,
        code,
        old_name.qualifier.slice(..),
        new_name.qualifier.slice(..),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::elm::refactors::simulations::simulation_test;

    simulation_test!(change_module_qualifier_of_value);
    simulation_test!(change_module_qualifier_of_type);
    simulation_test!(change_module_qualifier_of_constructor);
    simulation_test!(
        change_module_qualifier_of_variable_from_unaliased_import_name
    );
    simulation_test!(change_module_qualifier_to_match_unaliased_import_name);
    simulation_test!(change_module_qualifier_to_invalid_name);
}
