use crate::analysis_thread::{SourceFileDiff, TreeChanges};
use crate::support::source_code::{Edit, SourceFileSnapshot};
use ropey::RopeSlice;
use tree_sitter::{Language, Node, Query, QueryCursor, TreeCursor};

pub(crate) struct RefactorEngine {
    query_for_imports: ImportsQuery,
    query_for_unqualified_values: UnqualifiedValuesQuery,
}

#[derive(Debug)]
pub(crate) enum Error {
    FailureWhileTraversingTree,
    InvalidQuery(tree_sitter::QueryError),
    MissingCaptureIndex,
}

impl RefactorEngine {
    pub(crate) fn new() -> Result<RefactorEngine, Error> {
        let language = tree_sitter_elm::language();
        let engine = RefactorEngine {
            query_for_imports: ImportsQuery::init(language)?,
            query_for_unqualified_values: UnqualifiedValuesQuery::init(
                language,
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
                self.remove_from_exposed_list(
                    &mut edits,
                    &mut cursor,
                    diff,
                    &base_name_str,
                    &qualifier,
                )?;
                self.remove_qualifier_from_name(
                    &mut edits,
                    &mut cursor,
                    diff,
                    &base_name_str,
                    &qualifier,
                )?;
            }
            ElmChange::ExposedValuesRemoved(nodes) => {
                // Find the name of the import in the old code.
                let first =
                    nodes.get(0).ok_or(Error::FailureWhileTraversingTree)?;
                let import_name_node = first
                    .parent()
                    .ok_or(Error::FailureWhileTraversingTree)?
                    .parent()
                    .ok_or(Error::FailureWhileTraversingTree)?
                    .child_by_field_name("moduleName")
                    .ok_or(Error::FailureWhileTraversingTree)?;
                let import_name =
                    diff.old.slice(&import_name_node.byte_range());

                // Modify the import in the new code with the name just found.
                let import = self
                    .query_for_imports
                    .run(&mut cursor, &diff.new)
                    .find(|import| import.name() == import_name)
                    .ok_or(Error::FailureWhileTraversingTree)?;
                let new_exposed_count = import.exposed_list().count();
                if new_exposed_count == 0 {
                    self.remove_exposed_list(&mut edits, &diff.new, &import);
                }
                let mut exposed_cursor = QueryCursor::new();
                nodes.into_iter().try_for_each(|node| {
                    self.remove_qualifier_from_name(
                        &mut edits,
                        &mut exposed_cursor,
                        diff,
                        &diff.old.slice(&node.byte_range()),
                        &import.aliased_name(),
                    )
                })?;
            }
            ElmChange::ExposingListRemoved(node) => {
                let import_node =
                    node.parent().ok_or(Error::FailureWhileTraversingTree)?;
                let import = self
                    .query_for_imports
                    .run_in(&mut cursor, &diff.old, import_node)
                    .next()
                    .ok_or(Error::FailureWhileTraversingTree)?;
                let qualifier = import.aliased_name();
                let mut cursor_2 = QueryCursor::new();
                self.query_for_imports
                    .run(&mut cursor_2, &diff.old)
                    .find(|import| import.aliased_name() == qualifier)
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
        };
        if edits.is_empty() {
            Ok(None)
        } else {
            Ok(Some(sort_edits(edits)))
        }
    }

    fn remove_exposed_list(
        &self,
        edits: &mut Vec<Edit>,
        code: &SourceFileSnapshot,
        import: &Import,
    ) {
        match import.exposing_list_node {
            None => {}
            Some(node) => edits.push(Edit::new(
                code.buffer,
                &mut code.bytes.clone(),
                &node.byte_range(),
                String::new(),
            )),
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
        let import = self
            .query_for_imports
            .run(cursor, &diff.new)
            .find(|import| import.aliased_name() == *qualifier)
            .ok_or(Error::FailureWhileTraversingTree)?;
        let mut exposed_list = import.exposed_list();
        let mut exposed_list_length = 0;
        let exposed = exposed_list
            .find(|exposed| {
                exposed_list_length += 1;
                *name == exposed.name()
            })
            .ok_or(Error::FailureWhileTraversingTree)?
            .node;
        exposed_list_length += exposed_list.count();
        if exposed_list_length == 1 {
            self.remove_exposed_list(edits, &diff.new, &import);
        } else {
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
        }
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
        self.query_for_unqualified_values
            .run(cursor, &diff.new)
            .for_each(|node| {
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
}

struct UnqualifiedValuesQuery {
    query: Query,
}

impl UnqualifiedValuesQuery {
    fn run<'a, 'tree>(
        &'a self,
        cursor: &'a mut QueryCursor,
        code: &'tree SourceFileSnapshot,
    ) -> UnqualifiedValues<'a, 'tree> {
        let matches = cursor.matches(&self.query, code.tree.root_node(), code);
        UnqualifiedValues { matches }
    }
}

struct UnqualifiedValues<'a, 'tree> {
    matches: tree_sitter::QueryMatches<'a, 'tree, &'a SourceFileSnapshot>,
}

impl<'a, 'tree> Iterator for UnqualifiedValues<'a, 'tree> {
    type Item = Node<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let match_ = self.matches.next()?;
        let capture = match_.captures.first()?;
        Some(capture.node)
    }
}

impl UnqualifiedValuesQuery {
    fn init(lang: Language) -> Result<UnqualifiedValuesQuery, Error> {
        let query_str = r#"
            [ (value_qid
                    .
                    (lower_case_identifier) @val
                )
                (upper_case_qid
                    .
                    (upper_case_identifier) @val
                )
            ]"#;
        let query = Query::new(lang, query_str).map_err(Error::InvalidQuery)?;
        let unqualified_values_query = UnqualifiedValuesQuery { query };
        Ok(unqualified_values_query)
    }
}

struct ImportsQuery {
    query: Query,
    root_index: usize,
    name_index: usize,
    as_clause_index: usize,
    exposing_list_index: usize,
}

impl ImportsQuery {
    fn init(lang: Language) -> Result<ImportsQuery, Error> {
        let query_str = r#"
            (import_clause
                moduleName: (module_identifier) @name
                asClause:
                (as_clause
                    name: (module_name_segment) @as_clause
                )?
                exposing: (exposing_list)? @exposing_list
            ) @root
            "#;
        let query = Query::new(lang, query_str).map_err(Error::InvalidQuery)?;
        let index_for_name = |name| {
            query
                .capture_index_for_name(name)
                .ok_or(Error::MissingCaptureIndex)
                .map(|index| index as usize)
        };
        let imports_query = ImportsQuery {
            root_index: index_for_name("root")?,
            name_index: index_for_name("name")?,
            as_clause_index: index_for_name("as_clause")?,
            exposing_list_index: index_for_name("exposing_list")?,
            query,
        };
        Ok(imports_query)
    }

    fn run<'a, 'tree>(
        &'a self,
        cursor: &'a mut QueryCursor,
        code: &'tree SourceFileSnapshot,
    ) -> Imports<'a, 'tree> {
        self.run_in(cursor, code, code.tree.root_node())
    }

    fn run_in<'a, 'tree>(
        &'a self,
        cursor: &'a mut QueryCursor,
        code: &'tree SourceFileSnapshot,
        node: Node<'tree>,
    ) -> Imports<'a, 'tree> {
        let matches = cursor.matches(&self.query, node, code);
        Imports {
            code,
            matches,
            query: self,
        }
    }
}

struct Imports<'a, 'tree> {
    code: &'tree SourceFileSnapshot,
    matches: tree_sitter::QueryMatches<'a, 'tree, &'a SourceFileSnapshot>,
    query: &'a ImportsQuery,
}

impl<'a, 'tree> Iterator for Imports<'a, 'tree> {
    type Item = Import<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut nodes: [Option<Node>; 4] = [None; 4];
        self.matches.next()?.captures.iter().for_each(|capture| {
            nodes[capture.index as usize] = Some(capture.node)
        });
        Some(Import {
            code: self.code,
            _root_node: nodes[self.query.root_index]?,
            name_node: nodes[self.query.name_index]?,
            as_clause_node: nodes[self.query.as_clause_index],
            exposing_list_node: nodes[self.query.exposing_list_index],
        })
    }
}

struct Import<'a> {
    code: &'a SourceFileSnapshot,
    _root_node: Node<'a>,
    name_node: Node<'a>,
    as_clause_node: Option<Node<'a>>,
    exposing_list_node: Option<Node<'a>>,
}

impl Import<'_> {
    fn name(&self) -> RopeSlice {
        self.code.slice(&self.name_node.byte_range())
    }

    fn aliased_name(&self) -> RopeSlice {
        let name_node = self.as_clause_node.unwrap_or(self.name_node);
        self.code.slice(&name_node.byte_range())
    }

    fn exposed_list(&self) -> ExposedList {
        let cursor = self.exposing_list_node.and_then(|node| {
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
        while cursor.goto_next_sibling() {
            let node = cursor.node();
            // When the programmer emptied out an exposing list entirely, so
            // only `exposing ()` remains, then the tree-sitter-elm parse result
            // will contain a single, empty `exposed_val` node, containing
            // another node marked 'missing'. This is not-unreasonable, given
            // an empty exposed list isn't valid Elm.
            //
            // For our purposes here we'd like to treat that exposed-list as
            // empty, so we can easily check for emptiness and then remove it.
            // Because the 'missing' node is wrapped inside a regular node, we
            // cannot use `is_missing()` on the outer nodes we see here, so we
            // check for length instead.
            //
            // We might consider tweaking the grammer to either put the
            // 'missing' state on the outside node, or maybe even remove the
            // wrapping entirely. Then this check likely wouldn't need this huge
            // comment explaining it.
            if node.is_named() && !node.byte_range().is_empty() {
                return Some(Exposed {
                    code: self.code,
                    node,
                });
            }
        }
        None
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
    QualifierAdded {
        qualifier: RopeSlice<'a>,
        base_name: Node<'a>,
    },
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
            [("upper_case_identifier", before)],
            [qualifier @ .., ("dot", _), ("upper_case_identifier", after)],
        )
        | (
            [("lower_case_identifier", before)],
            [qualifier @ .., ("dot", _), ("lower_case_identifier", after)],
        ) => {
            let name_before = changes.old_code.slice(&before.byte_range());
            let name_after = changes.new_code.slice(&after.byte_range());
            let valid_qualifier = qualifier.iter().all(|(kind, _)| {
                *kind == "dot" || *kind == "module_name_segment"
            });
            let (_, qualifier_start) = qualifier.first()?;
            let (_, qualifier_end) = qualifier.last()?;
            let range = qualifier_start.start_byte()..qualifier_end.end_byte();
            if valid_qualifier && name_before == name_after {
                Some(ElmChange::QualifierAdded {
                    qualifier: changes.new_code.slice(&range),
                    base_name: *after,
                })
            } else {
                None
            }
        }
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
    use crate::support::source_code::Buffer;
    use crate::test_support::included_answer_test as ia_test;
    use crate::test_support::simulation::Simulation;
    use crate::SourceFileSnapshot;
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
    simulation_test!(add_module_alias_as_qualifier_to_variable);
    simulation_test!(add_module_qualifier_to_type);
    simulation_test!(add_module_qualifier_to_type_with_same_name);
    simulation_test!(remove_variable_from_exposing_list_of_import);
    simulation_test!(remove_value_from_exposing_list_of_import_with_as_clause);
    simulation_test!(remove_multiple_values_from_exposing_list_of_import);
    simulation_test!(remove_exposing_clause_from_import);
    simulation_test!(remove_exposing_clause_from_import_with_as_clause);

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
