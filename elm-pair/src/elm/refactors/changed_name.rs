use crate::elm::dependencies::DataflowComputation;
use crate::elm::refactors::lib::renaming;
use crate::elm::{
    Name, Queries, Refactor, FUNCTION_DECLARATION_LEFT, LOWER_PATTERN,
    VALUE_QID,
};
use crate::lib::log;
use crate::lib::log::Error;
use crate::lib::source_code::SourceFileSnapshot;
use std::collections::HashSet;
use std::iter::FromIterator;
use std::ops::Range;
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
            Ok((name, node, scope)) => is_changed_scope(
                new_node, &old_name, &new_name, name, node, scope,
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

// Check if a scope (a variable name and the code range in which it can be used)
// has been affected by the programmar changing a variable name.
fn is_changed_scope(
    changed_node: &Node,
    old_name: &Name,
    new_name: &Name,
    scope_name: &Name,
    scope_name_node: &Node,
    scope_range: &Range<usize>,
) -> bool {
    match changed_node.kind_id() {
        FUNCTION_DECLARATION_LEFT | LOWER_PATTERN => {
            // We changed the definition site of the name to a new name.
            scope_name == new_name
                && changed_node
                    .byte_range()
                    .contains(&scope_name_node.start_byte())
        }
        VALUE_QID => {
            // We changed a variable at a usage site, not where it is defined.
            scope_name == old_name
                && scope_range.contains(&changed_node.start_byte())
        }
        kind => {
            log::mk_err!("no rename behavior for kind {:?}", kind);
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::elm::refactors::lib::simulations::simulation_test;

    simulation_test!(change_variable_name_in_let_binding);
    simulation_test!(change_variable_name_defined_in_let_binding);
    simulation_test!(change_function_argument_name);
    simulation_test!(change_variable_name_defined_as_function_argument);
    simulation_test!(
        change_variable_name_in_let_binding_to_name_already_in_use
    );
}
