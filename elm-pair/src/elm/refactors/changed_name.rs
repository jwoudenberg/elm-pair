use crate::elm::dependencies::DataflowComputation;
use crate::elm::refactors::lib::renaming;
use crate::elm::{Name, Queries, Refactor};
use crate::lib::log;
use crate::lib::log::Error;
use crate::lib::source_code::SourceFileSnapshot;
use std::collections::HashSet;
use std::iter::FromIterator;
use tree_sitter::{Node, QueryCursor};

pub fn refactor(
    queries: &Queries,
    computation: &mut DataflowComputation,
    refactor: &mut Refactor,
    code: &SourceFileSnapshot,
    old_name: Name,
    new_name: Name,
    new_node: &Node,
) -> Result<(), Error> {
    let mut cursor = QueryCursor::new();
    let (_, _, scope) = queries
        .query_for_names_with_scopes
        .run(&mut cursor, code)
        .find(|res| match res {
            Err(err) => {
                log::mk_err!(
                    "could not parse match in names_with_scopes query: {:?}",
                    err,
                );
                false
            }
            Ok((_, name_node, _)) => crate::lib::range::contains_range(
                &new_node.byte_range(),
                &name_node.byte_range(),
            ),
        })
        .ok_or_else(|| {
            log::mk_err!(
                "could not find definition site of local var: {:?}",
                old_name
            )
        })??;
    renaming::free_names(
        queries,
        computation,
        refactor,
        code,
        &HashSet::from_iter(std::iter::once(new_name.clone())),
        &[&new_node.byte_range()],
    )?;
    renaming::rename(
        queries,
        refactor,
        code,
        &old_name,
        &new_name,
        &[&scope],
        &[],
    )
}

#[cfg(test)]
mod tests {
    use crate::elm::refactors::lib::simulations::simulation_test;

    simulation_test!(change_variable_name_in_let_binding);
    simulation_test!(
        change_variable_name_in_let_binding_to_name_already_in_use
    );
}
