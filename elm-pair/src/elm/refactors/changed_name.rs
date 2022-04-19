use crate::editors;
use crate::elm::dependencies::DataflowComputation;
use crate::elm::io::ExportedName;
use crate::elm::module_name::ModuleName;
use crate::elm::queries::imports::{ExposedName, Import};
use crate::elm::queries::qualified_values::QualifiedName;
use crate::elm::refactors::lib::renaming;
use crate::elm::{
    Name, NameKind, Queries, Refactor, RECORD_PATTERN, RECORD_TYPE,
};
use crate::lib::log;
use crate::lib::log::Error;
use crate::lib::source_code::{Buffer, SourceFileSnapshot};
use std::collections::HashMap;
use std::collections::HashSet;
use std::iter::FromIterator;
use std::ops::Range;
use std::path::PathBuf;
use tree_sitter::{Node, QueryCursor};

pub fn refactor(
    queries: &Queries,
    computation: &mut DataflowComputation,
    refactor: &mut Refactor,
    code: &SourceFileSnapshot,
    old_code: &SourceFileSnapshot,
    buffers: &HashMap<Buffer, SourceFileSnapshot>,
    buffers_by_path: &HashMap<(editors::Id, PathBuf), Buffer>,
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
        // (i.e. shortest) scope will be the one the variable can be used in.
        .min_by_key(|(_, scope)| scope.len());

    let files_to_open: Vec<PathBuf> = computation
        .dependent_modules_cursor(code.buffer)
        .iter()
        .filter(|path| {
            !buffers_by_path.contains_key(&(code.buffer.editor_id, path.into()))
        })
        .cloned()
        .collect();
    if !files_to_open.is_empty() {
        refactor.open_files(files_to_open);
        return Ok(());
    }

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
            batch_rename(
                queries,
                computation,
                refactor,
                code,
                old_code,
                buffers,
                &mut cursor,
                HashMap::from([
                    (old_constructor, new_constructor),
                    (old_type, new_type),
                ]),
                new_node,
                &scope,
            )
        }
        Some((RenameKind::AnyOther, scope)) => batch_rename(
            queries,
            computation,
            refactor,
            code,
            old_code,
            buffers,
            &mut cursor,
            HashMap::from_iter(std::iter::once((old_name, new_name))),
            new_node,
            &scope,
        ),
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

fn batch_rename(
    queries: &Queries,
    computation: &mut DataflowComputation,
    refactor: &mut Refactor,
    code: &SourceFileSnapshot,
    old_code: &SourceFileSnapshot,
    buffers: &HashMap<Buffer, SourceFileSnapshot>,
    cursor: &mut QueryCursor,
    new_name_by_old: HashMap<Name, Name>,
    new_node: &Node,
    scope: &Range<usize>,
) -> Result<(), Error> {
    // Propopage new name in buffer where the name is defined.
    renaming::free_names(
        queries,
        computation,
        refactor,
        code,
        &HashSet::from_iter(new_name_by_old.values().cloned()),
        &[scope],
        &[&new_node.byte_range()],
    )?;
    for (old_name, new_name) in new_name_by_old.iter() {
        renaming::rename(
            queries,
            refactor,
            code,
            old_name,
            new_name,
            &[scope],
            &[],
        )?;
    }

    // Update other buffers that import and use the name.
    let module_name = queries.query_for_module_declaration.run(cursor, code)?;
    let exports = queries.query_for_exports.run(old_code)?;
    let exported_names: Vec<(Name, Name, ExportedName)> = new_name_by_old
        .into_iter()
        .filter_map(|(old_name, new_name)| {
            let exported_name = find_exported_name(&exports, &old_name)?;
            Some((old_name, new_name, exported_name))
        })
        .collect();
    for other_buffer_code in buffers.values() {
        if let Some(import) = code_imports_module(
            queries,
            cursor,
            other_buffer_code,
            &module_name,
        ) {
            for (old_name, new_name, exported_name) in exported_names.iter() {
                rename_imported_name(
                    queries,
                    computation,
                    refactor,
                    other_buffer_code,
                    &import,
                    old_name,
                    new_name,
                    exported_name,
                )?;
            }
        }
    }
    Ok(())
}

// Find the ExportedName describing the export of a given name from a module.
// If the module is not exporting the provided name this function will return
// None.
fn find_exported_name(
    exports: &[ExportedName],
    name_: &Name,
) -> Option<ExportedName> {
    exports
        .iter()
        .find(|exported_name| match (name_.kind, exported_name) {
            (NameKind::Value, ExportedName::Value { name }) => {
                &name_.name == name
            }
            (NameKind::Type, ExportedName::Type { name, .. }) => {
                &name_.name == name
            }
            (
                NameKind::Constructor,
                ExportedName::Type { constructors, .. },
            ) => constructors.iter().any(|name| &name_.name == name),
            (NameKind::Type, ExportedName::RecordTypeAlias { name }) => {
                &name_.name == name
            }
            (NameKind::Constructor, ExportedName::RecordTypeAlias { name }) => {
                &name_.name == name
            }
            _ => false,
        })
        .cloned()
}

fn code_imports_module<'a>(
    queries: &Queries,
    cursor: &'a mut QueryCursor,
    code: &'a SourceFileSnapshot,
    module_name: &ModuleName,
) -> Option<Import<'a>> {
    queries
        .query_for_imports
        .run(cursor, code)
        .find(|import| &import.module_name() == module_name)
}

// Rename a name imported from another module.
fn rename_imported_name(
    queries: &Queries,
    computation: &mut DataflowComputation,
    refactor: &mut Refactor,
    code: &SourceFileSnapshot,
    import: &Import,
    old_name: &Name,
    new_name: &Name,
    exported_name: &ExportedName,
) -> Result<(), Error> {
    let mut exposed_names =
        import.exposing_list().filter_map(|res| match res {
            Ok((_, name)) => Some(name),
            Err(err) => {
                log::error!("error parsing exposing list: {:?}", err);
                None
            }
        });

    let qualifier = import.aliased_name();
    renaming::rename_qualified(
        queries,
        refactor,
        code,
        &QualifiedName {
            qualifier: qualifier.into(),
            unqualified_name: old_name.clone(),
        },
        &QualifiedName {
            qualifier: qualifier.into(),
            unqualified_name: new_name.clone(),
        },
    )?;

    let exposed = match old_name.kind {
        NameKind::Value | NameKind::Operator => {
            exposed_names.any(|exposed_name| {
                let exposes_all = matches!(exposed_name, ExposedName::All);
                let exposes_val = matches!(exposed_name,
                                    ExposedName::Value(val)
                                    if val.name == old_name.name);
                exposes_all || exposes_val
            })
        }
        NameKind::Type => exposed_names.any(|exposed_name| {
            let exposes_all = matches!(exposed_name, ExposedName::All);
            let exposes_type = matches!(exposed_name,
                                    ExposedName::Type(type_)
                                    if type_.name == old_name.name);
            exposes_all || exposes_type
        }),
        NameKind::Constructor => exposed_names.any(|exposed_name| {
            if matches!(exposed_name, ExposedName::All) {
                return true;
            }

            match &exported_name {
                ExportedName::Type { name, .. } => {
                    matches!(
                        exposed_name,
                        ExposedName::Type(type_)
                        if type_.exposing_constructors
                        && &type_.name == name
                    )
                }
                ExportedName::RecordTypeAlias { name, .. } => {
                    matches!(
                        exposed_name,
                        ExposedName::Type(type_)
                        if &type_.name == name
                    )
                }
                _ => {
                    log::error!(
                        "expected exported constructor, got: {:?}",
                        exported_name
                    );
                    false
                }
            }
        }),
    };

    if exposed {
        renaming::free_names(
            queries,
            computation,
            refactor,
            code,
            &HashSet::from_iter(std::iter::once(new_name.clone())),
            &[],
            &[],
        )?;
        renaming::rename(
            queries,
            refactor,
            code,
            old_name,
            new_name,
            &[],
            &[],
        )?;
    }

    Ok(())
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
    simulation_test!(
        change_function_argument_name_to_name_used_as_other_function_argument
    );
    simulation_test!(change_variable_name_in_case_pattern);
    simulation_test!(change_variable_name_to_already_existing_name_in_scope);
    simulation_test!(change_lambda_argument_name);
    simulation_test!(change_name_of_top_level_function);
    simulation_test!(change_name_of_top_level_function_in_type_definition);
    simulation_test!(change_type_name);
    simulation_test!(change_constructor_name);
    simulation_test!(change_type_alias_name);
    simulation_test!(change_record_type_alias_name);

    // Cross-file renaming
    simulation_test!(change_constructor_name_used_in_other_module);
    simulation_test!(change_constructor_name_unexposed_to_other_modules);
    simulation_test!(change_record_type_alias_name_used_in_other_module);
    simulation_test!(change_record_type_alias_name_unexposed_to_other_modules);
    simulation_test!(change_type_name_used_in_other_module);
    simulation_test!(change_type_name_unexposed_to_other_modules);
    simulation_test!(change_variable_name_used_in_other_module);
    simulation_test!(change_variable_name_unexposed_to_other_modules);

    // Using a different constructor in a function should not trigger a rename.
    simulation_test!(use_different_constructor);

    // Changing a field record requires changing the record type and all other
    // uses of that type. We don't support that yet, so for now we do nothing!
    simulation_test!(change_variable_name_of_record_field_pattern);
}
