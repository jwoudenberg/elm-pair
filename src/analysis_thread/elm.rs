use crate::analysis_thread::{
    byte_to_point, Error, SourceFileDiff, SourceFileSnapshot, TreeChanges,
};
use crate::{debug_code_slice, Edit};
use core::ops::Range;
use tree_sitter::{Node, Query, QueryCursor};

pub(crate) struct RefactorEngine {
    query_for_exposed_imports: Query,
}

#[derive(Debug)]
pub(crate) enum RefactorError {
    NoneImplementedForThisChange(ElmChange),
    FailureWhileTraversingTree,
}

impl RefactorEngine {
    pub(crate) fn new() -> Result<RefactorEngine, Error> {
        let language = tree_sitter_elm::language();
        let mk_query = |query_string| {
            Query::new(language, query_string).map_err(Error::InvalidQuery)
        };
        let engine = RefactorEngine {
            query_for_exposed_imports: mk_query(
                r#"
                [ (import_clause
                    exposing:
                      (exposing_list
                        [ (double_dot)
                          (exposed_value)
                          (exposed_type)
                          (exposed_operator)
                        ] @exposed_value
                      )
                  ) @import
                ]"#,
            )?,
        };
        Ok(engine)
    }

    pub(crate) fn respond_to_change(
        &self,
        diff: &SourceFileDiff,
        change: ElmChange,
    ) -> Result<Vec<Edit>, RefactorError> {
        match change {
            ElmChange::QualifierAdded(name, qualifier) => {
                let mut cursor = QueryCursor::new();
                let exposed = cursor
                    .matches(
                        &self.query_for_exposed_imports,
                        diff.new.tree.root_node(),
                        &diff.new,
                    )
                    .find_map(|m| {
                        let (import, exposed_val) = match m.captures {
                            [x, y] => (x, y),
                            _ => panic!("wrong number of capures"),
                        };
                        let import_node =
                            import.node.child_by_field_name("moduleName")?;
                        let import_name = debug_code_slice(
                            &diff.new,
                            &import_node.byte_range(),
                        );
                        let exposed_name = debug_code_slice(
                            &diff.new,
                            &exposed_val.node.byte_range(),
                        );
                        if import_name == *qualifier && exposed_name == *name {
                            Some(exposed_val.node)
                        } else {
                            None
                        }
                    })
                    .ok_or(RefactorError::FailureWhileTraversingTree)?;
                let range = || {
                    let next = exposed.next_sibling();
                    if let Some(node) = next {
                        if node.kind() == "," {
                            let end_byte = match node.next_sibling() {
                                Some(next) => next.start_byte(),
                                None => node.end_byte(),
                            };
                            return exposed.start_byte()..end_byte;
                        }
                    }
                    let prev = exposed.prev_sibling();
                    if let Some(node) = prev {
                        if node.kind() == "," {
                            let start_byte = match node.prev_sibling() {
                                Some(prev) => prev.end_byte(),
                                None => node.start_byte(),
                            };
                            return start_byte..exposed.end_byte();
                        }
                    }
                    exposed.byte_range()
                };
                Ok(vec![mk_edit(&diff.new, &range(), String::new())])
            }
            _ => Err(RefactorError::NoneImplementedForThisChange(change)),
        }
    }
}

#[derive(Debug)]
pub(crate) enum ElmChange {
    NameChanged(String, String),
    TypeChanged(String, String),
    ImportAdded(String),
    ImportRemoved(String),
    FieldAdded(String),
    FieldRemoved(String),
    TypeAdded(String),
    TypeRemoved(String),
    TypeAliasAdded(String),
    TypeAliasRemoved(String),
    QualifierAdded(String, String),
    QualifierRemoved(String, String),
    AsClauseAdded(String, String),
    AsClauseRemoved(String, String),
    AsClauseChanged(String, String),
}

// TODO: use kind ID's instead of names for pattern matching.
pub(in crate::analysis_thread) fn interpret_change(
    changes: &TreeChanges,
) -> Option<ElmChange> {
    match (
        attach_kinds(&changes.old_removed).as_slice(),
        attach_kinds(&changes.new_added).as_slice(),
    ) {
        (
            [("lower_case_identifier", before)],
            [("lower_case_identifier", after)],
        ) => Some(ElmChange::NameChanged(
            debug_code_slice(changes.old_code, &before.byte_range()),
            debug_code_slice(changes.new_code, &after.byte_range()),
        )),
        (
            [("upper_case_identifier", before)],
            [("upper_case_identifier", after)],
        ) => match before.parent()?.kind() {
            "as_clause" => Some(ElmChange::AsClauseChanged(
                debug_code_slice(changes.old_code, &before.byte_range()),
                debug_code_slice(changes.new_code, &after.byte_range()),
            )),
            _ => Some(ElmChange::TypeChanged(
                debug_code_slice(changes.old_code, &before.byte_range()),
                debug_code_slice(changes.new_code, &after.byte_range()),
            )),
        },
        ([], [("import_clause", after)]) => Some(ElmChange::ImportAdded(
            debug_code_slice(changes.new_code, &after.byte_range()),
        )),
        ([("import_clause", before)], []) => Some(ElmChange::ImportRemoved(
            debug_code_slice(changes.old_code, &before.byte_range()),
        )),
        ([], [("type_declaration", after)]) => Some(ElmChange::TypeAdded(
            debug_code_slice(changes.new_code, &after.byte_range()),
        )),
        ([("type_declaration", before)], []) => Some(ElmChange::TypeRemoved(
            debug_code_slice(changes.old_code, &before.byte_range()),
        )),
        ([], [("type_alias_declaration", after)]) => {
            Some(ElmChange::TypeAliasAdded(debug_code_slice(
                changes.new_code,
                &after.byte_range(),
            )))
        }
        ([("type_alias_declaration", before)], []) => {
            Some(ElmChange::TypeAliasRemoved(debug_code_slice(
                changes.old_code,
                &before.byte_range(),
            )))
        }
        ([], [("field_type", after)]) => Some(ElmChange::FieldAdded(
            debug_code_slice(changes.new_code, &after.byte_range()),
        )),
        ([], [(",", _), ("field_type", after)]) => Some(ElmChange::FieldAdded(
            debug_code_slice(changes.new_code, &after.byte_range()),
        )),
        ([], [("field_type", after), (",", _)]) => Some(ElmChange::FieldAdded(
            debug_code_slice(changes.new_code, &after.byte_range()),
        )),
        ([("field_type", before)], []) => Some(ElmChange::FieldRemoved(
            debug_code_slice(changes.old_code, &before.byte_range()),
        )),
        ([(",", _), ("field_type", before)], []) => {
            Some(ElmChange::FieldRemoved(debug_code_slice(
                changes.old_code,
                &before.byte_range(),
            )))
        }
        ([("field_type", before), (",", _)], []) => {
            Some(ElmChange::FieldRemoved(debug_code_slice(
                changes.old_code,
                &before.byte_range(),
            )))
        }
        (
            [("upper_case_identifier", qualifier), ("dot", _), ("upper_case_identifier", before)],
            [("upper_case_identifier", after)],
        ) => {
            let name_before =
                debug_code_slice(changes.old_code, &before.byte_range());
            let name_after =
                debug_code_slice(changes.new_code, &after.byte_range());
            if name_before == name_after {
                Some(ElmChange::QualifierRemoved(
                    name_before,
                    debug_code_slice(changes.old_code, &qualifier.byte_range()),
                ))
            } else {
                None
            }
        }
        (
            [("upper_case_identifier", qualifier), ("dot", _), ("lower_case_identifier", before)],
            [("lower_case_identifier", after)],
        ) => {
            let name_before =
                debug_code_slice(changes.old_code, &before.byte_range());
            let name_after =
                debug_code_slice(changes.new_code, &after.byte_range());
            if name_before == name_after {
                Some(ElmChange::QualifierRemoved(
                    name_before,
                    debug_code_slice(changes.old_code, &qualifier.byte_range()),
                ))
            } else {
                None
            }
        }
        (
            [("upper_case_identifier", before)],
            [("upper_case_identifier", qualifier), ("dot", _), ("upper_case_identifier", after)],
        ) => {
            let name_before =
                debug_code_slice(changes.old_code, &before.byte_range());
            let name_after =
                debug_code_slice(changes.new_code, &after.byte_range());
            if name_before == name_after {
                Some(ElmChange::QualifierAdded(
                    name_before,
                    debug_code_slice(changes.new_code, &qualifier.byte_range()),
                ))
            } else {
                None
            }
        }
        (
            [("lower_case_identifier", before)],
            [("upper_case_identifier", qualifier), ("dot", _), ("lower_case_identifier", after)],
        ) => {
            let name_before =
                debug_code_slice(changes.old_code, &before.byte_range());
            let name_after =
                debug_code_slice(changes.new_code, &after.byte_range());
            if name_before == name_after {
                Some(ElmChange::QualifierAdded(
                    name_before,
                    debug_code_slice(changes.new_code, &qualifier.byte_range()),
                ))
            } else {
                None
            }
        }
        ([("as_clause", before)], []) => Some(ElmChange::AsClauseRemoved(
            debug_code_slice(
                changes.old_code,
                &before.prev_sibling()?.byte_range(),
            ),
            debug_code_slice(
                changes.old_code,
                &before.child_by_field_name("name")?.byte_range(),
            ),
        )),
        ([], [("as_clause", after)]) => Some(ElmChange::AsClauseAdded(
            debug_code_slice(
                changes.new_code,
                &after.prev_sibling()?.byte_range(),
            ),
            debug_code_slice(
                changes.new_code,
                &after.child_by_field_name("name")?.byte_range(),
            ),
        )),
        _ => {
            // debug_print_tree_changes(changes);
            None
        }
    }
}

fn attach_kinds<'a>(nodes: &'a [Node<'a>]) -> Vec<(&'a str, &'a Node<'a>)> {
    nodes.iter().map(|node| (node.kind(), node)).collect()
}

fn mk_edit(
    code: &SourceFileSnapshot,
    range: &Range<usize>,
    new_bytes: String,
) -> Edit {
    let new_end_byte = range.start + new_bytes.len();
    Edit {
        buffer: code.buffer,
        new_bytes,
        input_edit: tree_sitter::InputEdit {
            start_byte: range.start,
            old_end_byte: range.end,
            new_end_byte,
            start_position: byte_to_point(&code.bytes, range.start),
            old_end_position: byte_to_point(&code.bytes, range.end),
            new_end_position: byte_to_point(&code.bytes, new_end_byte),
        },
    }
}
