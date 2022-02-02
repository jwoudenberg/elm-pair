use crate::elm::dependencies::DataflowComputation;
use crate::elm::refactors::lib::renaming;
use crate::elm::{
    Name, NameKind, Queries, Refactor, RECORD_PATTERN, RECORD_TYPE,
};
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
    if new_name.name.len_chars() == 0 {
        return Ok(());
    }

    // We've renamed a variable at its definition site.
    let mut cursor = QueryCursor::new();
    let opt_scope = queries
        .query_for_scopes
        .run(&mut cursor, code)
        .filter_map(|scope| {
            if scope.contains(&new_node.start_byte()) {
                Some((rename_kind(new_node), scope))
            } else {
                None
            }
        })
        // If the variable definition is in multiple scopes, the innermost
        // (i.e. shortes) scope will be the one the variable can be used in.
        .min_by_key(|(_, scope)| scope.len());

    match opt_scope {
        Some((RenameKind::RecordFieldPattern, _)) => Ok(()),
        Some((RenameKind::RecordTypeAlias, scope)) => {
            let old_constructor = Name {
                name: old_name.name.clone(),
                kind: NameKind::Constructor,
            };
            let new_constructor = Name {
                name: new_name.name.clone(),
                kind: NameKind::Constructor,
            };
            let old_type = Name {
                name: old_name.name,
                kind: NameKind::Type,
            };
            let new_type = Name {
                name: new_name.name,
                kind: NameKind::Type,
            };
            renaming::free_names(
                queries,
                computation,
                refactor,
                code,
                &HashSet::from_iter([
                    new_constructor.clone(),
                    new_type.clone(),
                ]),
                &[&new_node.byte_range()],
            )?;
            renaming::rename(
                queries,
                refactor,
                code,
                &old_type,
                &new_type,
                &[&scope],
                &[],
            )?;
            renaming::rename(
                queries,
                refactor,
                code,
                &old_constructor,
                &new_constructor,
                &[&scope],
                &[],
            )
        }
        Some((RenameKind::AnyOther, scope)) => {
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
        None => Err(log::mk_err!(
            "Could not find variable definition for rename"
        )),
    }
}

// Helper type for injecting behaviors for a couple of difficult renames.
#[derive(Clone, Copy, Debug)]
enum RenameKind {
    // If we change a field name we need to change the record it belongs to.
    RecordFieldPattern,
    // Record type aliases can be used both as a type and as a constructor.
    RecordTypeAlias,
    // Remaining rename operations all share the same logic.
    AnyOther,
}

fn rename_kind(node: &Node) -> RenameKind {
    if is_record_field_pattern(node) {
        RenameKind::RecordFieldPattern
    } else if is_record_type_alias(node) {
        RenameKind::RecordTypeAlias
    } else {
        RenameKind::AnyOther
    }
}

fn is_record_field_pattern(node: &Node) -> bool {
    let pattern_kind = node.parent().as_ref().unwrap_or(node).kind_id();
    pattern_kind == RECORD_PATTERN
}

fn is_record_type_alias(node: &Node) -> bool {
    let kind = node
        .child_by_field_name("typeExpression")
        .and_then(|n| n.child_by_field_name("part"))
        .as_ref()
        .unwrap_or(node)
        .kind_id();
    kind == RECORD_TYPE
}

#[cfg(test)]
mod tests {
    use crate::elm::refactors::lib::simulations::simulation_test;

    simulation_test!(change_variable_name_in_let_binding);
    simulation_test!(change_variable_name_in_let_binding_pattern);
    simulation_test!(
        change_variable_name_in_let_binding_to_name_already_in_use
    );
    simulation_test!(change_name_of_function_in_type_definition_in_let_binding);
    simulation_test!(change_function_argument_name);
    simulation_test!(change_variable_name_in_case_pattern);
    simulation_test!(change_variable_name_to_already_existing_name_in_scope);
    simulation_test!(change_lambda_argument_name);
    simulation_test!(change_name_of_top_level_function);
    simulation_test!(change_name_of_top_level_function_in_type_definition);
    simulation_test!(change_type_name);
    simulation_test!(change_constructor_name);
    simulation_test!(change_type_alias_name);
    simulation_test!(change_record_type_alias_name);

    // Changing a field record requires changing the record type and all other
    // uses of that type. We don't support that yet, so for now we do nothing!
    simulation_test!(change_variable_name_of_record_field_pattern);
}
