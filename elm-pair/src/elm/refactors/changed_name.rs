use crate::elm::dependencies::DataflowComputation;
use crate::elm::refactors::lib::renaming;
use crate::elm::{
    Name, Queries, Refactor, FUNCTION_DECLARATION_LEFT, LOWER_PATTERN,
    RECORD_PATTERN, TYPE_ANNOTATION, VALUE_QID,
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
    let opt_scope = match new_node.kind_id() {
        FUNCTION_DECLARATION_LEFT | LOWER_PATTERN | TYPE_ANNOTATION => {
            find_scope(queries, code, |scope| {
                is_name_definition_in_scope(new_node, scope)
            })
        }
        VALUE_QID => find_scope(queries, code, |scope| {
            let mut cursor = QueryCursor::new();
            let definition_sites: Vec<Node> = queries
                .query_for_name_definitions
                .run(&mut cursor, code)
                .filter(|(name, _)| name == &old_name)
                .map(|(_, node)| node)
                .collect();
            is_variable_usage_in_scope(new_node, definition_sites, scope)
        }),
        kind => {
            log::error!(
                "could not find definition site of local var: {:?}",
                kind
            );
            None
        }
    };

    if let Some(scope) = opt_scope {
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
        )?;
    }
    Ok(())
}

fn find_scope<F>(
    queries: &Queries,
    code: &SourceFileSnapshot,
    in_scope: F,
) -> Option<Range<usize>>
where
    F: FnMut(&Range<usize>) -> bool,
{
    let mut cursor = QueryCursor::new();
    queries
        .query_for_scopes
        .run(&mut cursor, code)
        .filter(in_scope)
        // If the variable definition is in multiple scopes, the innermost
        // (i.e. shortes) scope will be the one the variable can be used in.
        .min_by_key(|scope| scope.len())
}

fn is_variable_usage_in_scope(
    changed_node: &Node,
    definition_sites: Vec<Node>,
    scope: &Range<usize>,
) -> bool {
    if !scope.contains(&changed_node.start_byte()) {
        return false;
    }
    definition_sites.iter().any(|definition_node| {
        is_name_definition_in_scope(definition_node, scope)
    })
}

fn is_name_definition_in_scope(
    changed_node: &Node,
    scope: &Range<usize>,
) -> bool {
    scope.contains(&changed_node.start_byte())
        && !is_record_field_pattern(changed_node)
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
    simulation_test!(change_name_of_top_level_function);
    simulation_test!(change_name_of_top_level_function_in_type_definition);
    simulation_test!(change_variable_name_defined_as_top_level_function);
    simulation_test!(change_name_of_function_in_type_definition_in_let_binding);
    // Changing a field record requires changing the record type and all other
    // uses of that type. We don't support that yet, so for now we do nothing!
    simulation_test!(change_variable_name_of_record_field_pattern);
}
