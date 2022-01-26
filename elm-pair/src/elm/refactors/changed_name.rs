use crate::elm::dependencies::DataflowComputation;
use crate::elm::refactors::lib::renaming;
use crate::elm::{
    Name, Queries, Refactor, FUNCTION_DECLARATION_LEFT, LOWER_PATTERN,
    RECORD_PATTERN, VALUE_QID,
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
    if new_name.name.len_chars() == 0 {
        return Ok(());
    }
    let mut cursor = QueryCursor::new();
    let mut cursor2 = QueryCursor::new();
    let (record_pattern, scope) = queries
        .query_for_scopes
        .run(&mut cursor, code)
        .filter_map(|scope| {
            is_changed_scope(
                &mut cursor2,
                queries,
                new_node,
                &old_name,
                code,
                &scope,
            )
        })
        // If the variable definition is in multiple scopes, the innermost
        // (i.e. shortes) scope will be the one the variable can be used in.
        .min_by_key(|(_, scope)| scope.len())
        .ok_or_else(|| {
            log::mk_err!(
                "could not find definition site of local var: {:?}",
                old_name
            )
        })?;

    // Suppose the changed variable originates from a record match:
    //
    //     nextYear : { date | years : Int } -> Int
    //     nextYear { years } = years + 1
    //
    // In this case we cannot change `years` without also changing the record
    // type and possibly other functions using that same type, so we do nothing.
    if record_pattern {
        return Ok(());
    }

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
    cursor: &mut QueryCursor,
    queries: &Queries,
    changed_node: &Node,
    old_name: &Name,
    code: &SourceFileSnapshot,
    scope_node: &Node,
) -> Option<(bool, Range<usize>)> {
    if !scope_node.byte_range().contains(&changed_node.start_byte()) {
        return None;
    }
    match changed_node.kind_id() {
        FUNCTION_DECLARATION_LEFT | LOWER_PATTERN => {
            // We changed the definition site of the name to a new name.
            Some((
                is_record_field_pattern(changed_node),
                scope_node.byte_range(),
            ))
        }
        VALUE_QID => {
            // We changed a variable at a usage site, not where it is defined.
            queries
                .query_for_name_definitions
                .run(cursor, code)
                .find_map(|(name, node)| {
                    if &name == old_name {
                        Some((
                            is_record_field_pattern(&node),
                            scope_node.byte_range(),
                        ))
                    } else {
                        None
                    }
                })
        }
        kind => {
            log::mk_err!("no rename behavior for kind {:?}", kind);
            None
        }
    }
}

fn is_record_field_pattern(node: &Node) -> bool {
    let pattern_kind = node
        .parent()
        .and_then(|n| n.parent())
        .as_ref()
        .unwrap_or(node)
        .kind_id();
    pattern_kind == RECORD_PATTERN
}

#[cfg(test)]
mod tests {
    use crate::elm::refactors::lib::simulations::simulation_test;

    simulation_test!(change_variable_name_in_let_binding);
    simulation_test!(change_variable_name_in_let_binding_pattern);
    simulation_test!(change_variable_name_defined_in_let_binding);
    simulation_test!(change_variable_name_defined_in_let_binding_pattern);
    simulation_test!(change_function_argument_name);
    simulation_test!(change_variable_name_defined_as_function_argument);
    simulation_test!(change_variable_name_in_case_pattern);
    simulation_test!(change_variable_name_defined_in_case_pattern);
    simulation_test!(
        change_variable_name_in_let_binding_to_name_already_in_use
    );
    simulation_test!(change_lambda_argument_name);
    simulation_test!(change_variable_name_defined_as_lambda_argument);
    simulation_test!(change_variable_name_to_already_existing_name_in_scope);
    // Changing a field record requires changing the record type and all other
    // uses of that type. We don't support that yet, so for now we do nothing!
    simulation_test!(change_variable_name_of_record_field_pattern);
}
