use crate::analysis_thread::{SourceFileDiff, TreeChanges};
use crate::support::source_code::{Edit, SourceFileSnapshot};
use ropey::RopeSlice;
use tree_sitter::{Node, Query, QueryCursor, TreeCursor};

pub(crate) struct RefactorEngine {
    query_for_exposed_imports: Query,
    query_for_unqualified_values: Query,
}

#[derive(Debug)]
pub(crate) enum Error {
    FailureWhileTraversingTree,
    InvalidQuery(tree_sitter::QueryError),
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
                (import_clause
                  moduleName: (upper_case_qid) @name
                ) @import
                "#,
            )?,
            query_for_unqualified_values: mk_query(
                r#"
                [ (value_qid
                     .
                     (lower_case_identifier) @val
                  )
                  (upper_case_qid
                     .
                     (upper_case_identifier) @val
                  )
                ]"#,
            )?,
        };
        Ok(engine)
    }

    pub(in crate::analysis_thread) fn respond_to_change(
        &self,
        diff: &SourceFileDiff,
        tree_changes: TreeChanges,
    ) -> Result<Option<Vec<Edit>>, Error> {
        let change = match interpret_change(tree_changes) {
            Some(change) => change,
            None => return Ok(None),
        };

        let mut edits = Vec::new();
        let mut cursor = QueryCursor::new();
        match change {
            ElmChange::QualifierAdded {
                qualifier,
                base_name,
            } => {
                let base_name_str = diff.new.slice(&base_name.byte_range());
                let qualifier_str = diff.new.slice(&qualifier.byte_range());
                self.remove_from_exposed_list(
                    &mut edits,
                    &mut cursor,
                    diff,
                    &base_name_str,
                    &qualifier_str,
                )?;
                self.remove_qualifier_from_name(
                    &mut edits,
                    &mut cursor,
                    diff,
                    &base_name_str,
                    &qualifier_str,
                )?;
            }
            ElmChange::ExposedValuesRemoved(nodes) => {
                nodes.into_iter().try_for_each(|node| {
                    let import_name = node
                        .parent()
                        .ok_or(Error::FailureWhileTraversingTree)?
                        .parent()
                        .ok_or(Error::FailureWhileTraversingTree)?
                        .child_by_field_name("moduleName")
                        .ok_or(Error::FailureWhileTraversingTree)?;
                    self.remove_qualifier_from_name(
                        &mut edits,
                        &mut cursor,
                        diff,
                        &diff.old.slice(&node.byte_range()),
                        &diff.old.slice(&import_name.byte_range()),
                    )
                })?;
            }
            ElmChange::ExposingListRemoved(node) => {
                let qualifier = diff.old.slice(
                    &node
                        .parent()
                        .unwrap()
                        .child_by_field_name("moduleName")
                        .unwrap()
                        .byte_range(),
                );
                self.imports(&mut cursor, &diff.old)
                    .find(|import| import.name() == qualifier)
                    .ok_or(Error::FailureWhileTraversingTree)?
                    .exposed_list()
                    .try_for_each(|exposed| {
                        let mut val_cursor = QueryCursor::new();
                        self.remove_qualifier_from_name(
                            &mut edits,
                            &mut val_cursor,
                            diff,
                            &exposed.name(),
                            &qualifier,
                        )
                    })?;
            }
            _ => {}
        };
        if edits.is_empty() {
            Ok(None)
        } else {
            Ok(Some(sort_edits(edits)))
        }
    }

    fn remove_from_exposed_list(
        &self,
        edits: &mut Vec<Edit>,
        cursor: &mut QueryCursor,
        diff: &SourceFileDiff,
        name: &RopeSlice,
        qualifier: &RopeSlice,
    ) -> Result<(), Error> {
        let imports = self
            .imports(cursor, &diff.new)
            .find(|import| import.name() == *qualifier)
            .ok_or(Error::FailureWhileTraversingTree)?;
        let exposed = imports
            .exposed_list()
            .find(|exposed| *name == exposed.name())
            .ok_or(Error::FailureWhileTraversingTree)?
            .node;
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
        edits.push(Edit::new(
            diff.new.buffer,
            &mut diff.new.bytes.clone(),
            &range(),
            String::new(),
        ));
        Ok(())
    }

    fn remove_qualifier_from_name(
        &self,
        edits: &mut Vec<Edit>,
        cursor: &mut QueryCursor,
        diff: &SourceFileDiff,
        name: &RopeSlice,
        qualifier: &RopeSlice,
    ) -> Result<(), Error> {
        cursor
            .matches(
                &self.query_for_unqualified_values,
                diff.new.tree.root_node(),
                &diff.new,
            )
            .for_each(|match_| {
                let node = match match_.captures {
                    [capture] => capture.node,
                    _ => panic!("wrong number of capures"),
                };
                if *name == diff.new.slice(&node.byte_range()) {
                    edits.push(Edit::new(
                        diff.new.buffer,
                        &mut diff.new.bytes.clone(),
                        &(node.start_byte()..node.start_byte()),
                        format!("{}.", qualifier),
                    ))
                }
            });
        Ok(())
    }

    fn imports<'a, 'tree>(
        &'a self,
        cursor: &'a mut QueryCursor,
        code: &'tree SourceFileSnapshot,
    ) -> Imports<'a, 'tree> {
        let matches = cursor.matches(
            &self.query_for_exposed_imports,
            code.tree.root_node(),
            code,
        );
        Imports { code, matches }
    }
}

struct Imports<'a, 'tree> {
    code: &'tree SourceFileSnapshot,
    matches: tree_sitter::QueryMatches<'a, 'tree, &'a SourceFileSnapshot>,
}

impl<'a, 'tree> Iterator for Imports<'a, 'tree> {
    type Item = Import<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let (root_node, name_node) = match self.matches.next()?.captures {
            [root, name] => (root.node, name.node),
            _ => return None,
        };
        Some(Import {
            code: self.code,
            root_node,
            name_node,
        })
    }
}

struct Import<'a> {
    code: &'a SourceFileSnapshot,
    root_node: Node<'a>,
    name_node: Node<'a>,
}

impl Import<'_> {
    fn name(&self) -> RopeSlice {
        self.code.slice(&self.name_node.byte_range())
    }

    fn exposed_list(&self) -> ExposedList {
        let cursor =
            self.root_node
                .child_by_field_name("exposing")
                .and_then(|node| {
                    let mut cursor = node.walk();
                    if cursor.goto_first_child() {
                        Some(cursor)
                    } else {
                        None
                    }
                });
        ExposedList {
            code: self.code,
            cursor,
        }
    }
}

struct ExposedList<'a> {
    code: &'a SourceFileSnapshot,
    cursor: Option<TreeCursor<'a>>,
}

impl<'a> Iterator for ExposedList<'a> {
    type Item = Exposed<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let cursor = self.cursor.as_mut()?;
        let node = cursor.node();
        if !cursor.goto_next_sibling() {
            self.cursor = None
        }
        Some(Exposed {
            code: self.code,
            node,
        })
    }
}

struct Exposed<'a> {
    code: &'a SourceFileSnapshot,
    node: Node<'a>,
}

impl Exposed<'_> {
    fn name(&self) -> RopeSlice {
        self.code.slice(&self.node.byte_range())
    }
}

// Sort edits in reverse order of where they change the source file. This
// ensures when we apply the edits in sorted order that earlier edits don't
// move the area of affect of later edits.
//
// We're assuming here that the areas of operation of different edits never
// overlap.
fn sort_edits(mut edits: Vec<Edit>) -> Vec<Edit> {
    edits.sort_by(|x, y| y.input_edit.start_byte.cmp(&x.input_edit.start_byte));
    edits
}

#[derive(Debug)]
pub(crate) enum ElmChange<'a> {
    NameChanged(RopeSlice<'a>, RopeSlice<'a>),
    TypeChanged(RopeSlice<'a>, RopeSlice<'a>),
    ImportAdded(RopeSlice<'a>),
    ImportRemoved(RopeSlice<'a>),
    FieldAdded(RopeSlice<'a>),
    FieldRemoved(RopeSlice<'a>),
    TypeAdded(RopeSlice<'a>),
    TypeRemoved(RopeSlice<'a>),
    TypeAliasAdded(RopeSlice<'a>),
    TypeAliasRemoved(RopeSlice<'a>),
    QualifierAdded {
        qualifier: Node<'a>,
        base_name: Node<'a>,
    },
    QualifierRemoved(RopeSlice<'a>, RopeSlice<'a>),
    AsClauseAdded(RopeSlice<'a>, RopeSlice<'a>),
    AsClauseRemoved(RopeSlice<'a>, RopeSlice<'a>),
    AsClauseChanged(RopeSlice<'a>, RopeSlice<'a>),
    ExposedValuesRemoved(Vec<Node<'a>>),
    ExposingListRemoved(Node<'a>),
}

// TODO: use kind ID's instead of names for pattern matching.
fn interpret_change(changes: TreeChanges) -> Option<ElmChange> {
    // debug_print_tree_changes(&changes);
    match (
        attach_kinds(changes.old_removed).as_slice(),
        attach_kinds(changes.new_added).as_slice(),
    ) {
        (
            [("lower_case_identifier", before)],
            [("lower_case_identifier", after)],
        ) => Some(ElmChange::NameChanged(
            changes.old_code.slice(&before.byte_range()),
            changes.new_code.slice(&after.byte_range()),
        )),
        (
            [("upper_case_identifier", before)],
            [("upper_case_identifier", after)],
        ) => match before.parent()?.kind() {
            "as_clause" => Some(ElmChange::AsClauseChanged(
                changes.old_code.slice(&before.byte_range()),
                changes.new_code.slice(&after.byte_range()),
            )),
            _ => Some(ElmChange::TypeChanged(
                changes.old_code.slice(&before.byte_range()),
                changes.new_code.slice(&after.byte_range()),
            )),
        },
        ([], [("import_clause", after)]) => Some(ElmChange::ImportAdded(
            changes.new_code.slice(&after.byte_range()),
        )),
        ([("import_clause", before)], []) => Some(ElmChange::ImportRemoved(
            changes.old_code.slice(&before.byte_range()),
        )),
        ([], [("type_declaration", after)]) => Some(ElmChange::TypeAdded(
            changes.new_code.slice(&after.byte_range()),
        )),
        ([("type_declaration", before)], []) => Some(ElmChange::TypeRemoved(
            changes.old_code.slice(&before.byte_range()),
        )),
        ([], [("type_alias_declaration", after)]) => {
            Some(ElmChange::TypeAliasAdded(
                changes.new_code.slice(&after.byte_range()),
            ))
        }
        ([("type_alias_declaration", before)], []) => {
            Some(ElmChange::TypeAliasRemoved(
                changes.old_code.slice(&before.byte_range()),
            ))
        }
        ([], [("field_type", after)])
        | ([], [(",", _), ("field_type", after)])
        | ([], [("field_type", after), (",", _)]) => Some(
            ElmChange::FieldAdded(changes.new_code.slice(&after.byte_range())),
        ),
        ([("field_type", before)], [])
        | ([(",", _), ("field_type", before)], [])
        | ([("field_type", before), (",", _)], []) => {
            Some(ElmChange::FieldRemoved(
                changes.old_code.slice(&before.byte_range()),
            ))
        }
        ([("exposed_value", before), rest @ ..], after)
        | ([(",", _), ("exposed_value", before), rest @ ..], after) => {
            match after {
                [] => {}
                [("exposed_value", node)]
                    if changes.new_code.slice(&node.byte_range()) == "" => {}
                _ => return None,
            }
            let mut removed_nodes = vec![*before];
            let mut rest = rest;
            while !rest.is_empty() {
                match rest {
                    [] => break,
                    [(",", _), new_rest @ ..] => rest = new_rest,
                    [("exposed_value", node), new_rest @ ..] => {
                        removed_nodes.push(*node);
                        rest = new_rest;
                    }
                    _ => return None,
                }
            }
            Some(ElmChange::ExposedValuesRemoved(removed_nodes))
        }
        (
            [("upper_case_identifier", qualifier), ("dot", _), ("upper_case_identifier", before)],
            [("upper_case_identifier", after)],
        )
        | (
            [("upper_case_identifier", qualifier), ("dot", _), ("lower_case_identifier", before)],
            [("lower_case_identifier", after)],
        ) => {
            let name_before = changes.old_code.slice(&before.byte_range());
            let name_after = changes.new_code.slice(&after.byte_range());
            if name_before == name_after {
                Some(ElmChange::QualifierRemoved(
                    name_before,
                    changes.old_code.slice(&qualifier.byte_range()),
                ))
            } else {
                None
            }
        }
        (
            [("upper_case_identifier", before)],
            [("upper_case_identifier", qualifier), ("dot", _), ("upper_case_identifier", after)],
        )
        | (
            [("lower_case_identifier", before)],
            [("upper_case_identifier", qualifier), ("dot", _), ("lower_case_identifier", after)],
        ) => {
            let name_before = changes.old_code.slice(&before.byte_range());
            let name_after = changes.new_code.slice(&after.byte_range());
            if name_before == name_after {
                Some(ElmChange::QualifierAdded {
                    qualifier: *qualifier,
                    base_name: *after,
                })
            } else {
                None
            }
        }
        ([("as_clause", before)], []) => Some(ElmChange::AsClauseRemoved(
            changes.old_code.slice(&before.prev_sibling()?.byte_range()),
            changes
                .old_code
                .slice(&before.child_by_field_name("name")?.byte_range()),
        )),
        ([], [("as_clause", after)]) => Some(ElmChange::AsClauseAdded(
            changes.new_code.slice(&after.prev_sibling()?.byte_range()),
            changes
                .new_code
                .slice(&after.child_by_field_name("name")?.byte_range()),
        )),
        ([("exposing_list", before)], []) => {
            Some(ElmChange::ExposingListRemoved(*before))
        }
        _ => None,
    }
}

fn attach_kinds(nodes: Vec<Node>) -> Vec<(&str, Node)> {
    nodes.into_iter().map(|node| (node.kind(), node)).collect()
}

// TODO: remove debug helper when it's no longer needed.
#[allow(dead_code)]
fn debug_print_tree_changes(changes: &TreeChanges) {
    println!("REMOVED NODES:");
    for node in &changes.old_removed {
        crate::debug_print_node(changes.old_code, 2, node);
    }
    println!("ADDED NODES:");
    for node in &changes.new_added {
        crate::debug_print_node(changes.new_code, 2, node);
    }
}

#[cfg(test)]
mod tests {
    use crate::analysis_thread::elm::RefactorEngine;
    use crate::analysis_thread::{diff_trees, SourceFileDiff};
    use crate::test_support::included_answer_test as ia_test;
    use crate::test_support::simulation::Simulation;
    use crate::{Buffer, SourceFileSnapshot};
    use std::path::Path;

    macro_rules! simulation_test {
        ($name:ident) => {
            #[test]
            fn $name() {
                let mut path = std::path::PathBuf::new();
                path.push("./tests");
                let module_name = ia_test::snake_to_camel(stringify!($name));
                path.push(module_name + ".elm");
                println!("Run simulation {:?}", &path);
                run_simulation_test(&path);
            }
        };
    }

    pub fn run_simulation_test(path: &Path) {
        match run_simulation_test_helper(path) {
            Err(err) => panic!("simulation failed with: {:?}", err),
            Ok(res) => ia_test::assert_eq_answer_in(&res, path),
        }
    }

    fn run_simulation_test_helper(path: &Path) -> Result<String, Error> {
        let simulation = Simulation::from_file(path)?;
        let buffer = Buffer {
            buffer_id: 0,
            editor_id: 0,
        };
        let old = SourceFileSnapshot::new(buffer, simulation.start_bytes)?;
        let new = SourceFileSnapshot::new(buffer, simulation.end_bytes)?;
        let diff = SourceFileDiff { old, new };
        let tree_changes = diff_trees(&diff);
        let refactor_engine = RefactorEngine::new()?;
        match refactor_engine.respond_to_change(&diff, tree_changes)? {
            None => Ok("No refactor for this change.".to_owned()),
            Some(refactor) => {
                let mut post_refactor = diff.new.bytes;
                for edit in refactor {
                    edit.apply(&mut post_refactor)
                }
                Ok(post_refactor.to_string())
            }
        }
    }

    simulation_test!(add_module_qualifier_to_variable);
    simulation_test!(add_module_qualifier_to_type);
    simulation_test!(add_module_qualifier_to_type_with_same_name);
    simulation_test!(remove_variable_from_exposing_list);
    simulation_test!(remove_multiple_variables_from_exposing_list);
    simulation_test!(remove_exposing_clause);

    #[derive(Debug)]
    enum Error {
        RunningSimulation(crate::test_support::simulation::Error),
        ParsingSourceCode(crate::support::source_code::ParseError),
        AnalyzingElm(crate::analysis_thread::elm::Error),
    }

    impl From<crate::test_support::simulation::Error> for Error {
        fn from(err: crate::test_support::simulation::Error) -> Error {
            Error::RunningSimulation(err)
        }
    }

    impl From<crate::support::source_code::ParseError> for Error {
        fn from(err: crate::support::source_code::ParseError) -> Error {
            Error::ParsingSourceCode(err)
        }
    }

    impl From<crate::analysis_thread::elm::Error> for Error {
        fn from(err: crate::analysis_thread::elm::Error) -> Error {
            Error::AnalyzingElm(err)
        }
    }
}
