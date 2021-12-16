use crate::analysis_thread::{SourceFileDiff, TreeChanges};
use crate::languages::elm::knowledge_base::{ElmExport, KnowledgeBase};
use crate::support::source_code::{Buffer, Edit, SourceFileSnapshot};
use crate::Error;
use ropey::{Rope, RopeSlice};
use tree_sitter::{Language, Node, Query, QueryCursor, TreeCursor};

pub mod idat;
pub mod knowledge_base;

// These constants come from the tree-sitter-elm grammar. They might need to
// be changed when tree-sitter-elm updates.
const COMMA: u16 = 6;
const DOT: u16 = 55;
const DOUBLE_DOT: u16 = 49;
const EXPOSED_TYPE: u16 = 92;
const EXPOSED_VALUE: u16 = 91;
const EXPOSED_OPERATOR: u16 = 94;
const EXPOSED_UNION_CONSTRUCTORS: u16 = 93;
const EXPOSING_LIST: u16 = 90;
const LOWER_CASE_IDENTIFIER: u16 = 1;
const MODULE_NAME_SEGMENT: u16 = 200;
const UPPER_CASE_IDENTIFIER: u16 = 33;

#[cfg(test)]
mod kind_constant_tests {
    #[test]
    fn check_kind_constants() {
        let language = tree_sitter_elm::language();
        let check = |constant, str, named| {
            assert_eq!(constant, language.id_for_node_kind(str, named))
        };
        check(super::COMMA, ",", false);
        check(super::DOT, "dot", true);
        check(super::EXPOSED_OPERATOR, "exposed_operator", true);
        check(super::EXPOSED_TYPE, "exposed_type", true);
        check(super::EXPOSED_VALUE, "exposed_value", true);
        check(
            super::EXPOSED_UNION_CONSTRUCTORS,
            "exposed_union_constructors",
            true,
        );
        check(super::DOUBLE_DOT, "double_dot", true);
        check(super::EXPOSING_LIST, "exposing_list", true);
        check(super::LOWER_CASE_IDENTIFIER, "lower_case_identifier", true);
        check(super::MODULE_NAME_SEGMENT, "module_name_segment", true);
        check(super::UPPER_CASE_IDENTIFIER, "upper_case_identifier", true);
    }
}

pub(crate) struct RefactorEngine {
    pub(crate) kb: KnowledgeBase,
    query_for_imports: ImportsQuery,
    query_for_unqualified_values: UnqualifiedValuesQuery,
    query_for_qualified_value: QualifiedValueQuery,
}

impl RefactorEngine {
    pub(crate) fn new() -> Result<RefactorEngine, Error> {
        let language = tree_sitter_elm::language();
        let engine = RefactorEngine {
            kb: KnowledgeBase::new(),
            query_for_imports: ImportsQuery::init(language)?,
            query_for_unqualified_values: UnqualifiedValuesQuery::init(
                language,
            )?,
            query_for_qualified_value: QualifiedValueQuery::init(language)?,
        };

        Ok(engine)
    }

    // TODO: try to return an Iterator instead of a Vector.
    // TODO: Try remove Vector from TreeChanges type.
    pub(crate) fn respond_to_change<'a>(
        &mut self,
        diff: &SourceFileDiff,
        changes: TreeChanges<'a>,
    ) -> Result<Option<Vec<Edit>>, Error> {
        // debug_print_tree_changes(diff, &changes);
        let edits = match (
            attach_kinds(&changes.old_removed).as_slice(),
            attach_kinds(&changes.new_added).as_slice(),
        ) {
            (
                [EXPOSED_VALUE | EXPOSED_TYPE, ..]
                | [COMMA, EXPOSED_VALUE | EXPOSED_TYPE, ..]
                | [DOUBLE_DOT],
                [] | [EXPOSED_VALUE],
            ) => on_removed_values_from_exposing_list(
                &mut self.kb,
                &self.query_for_unqualified_values,
                &self.query_for_imports,
                diff,
                changes,
            )?,
            (
                [UPPER_CASE_IDENTIFIER],
                [MODULE_NAME_SEGMENT, DOT, .., UPPER_CASE_IDENTIFIER],
            )
            | (
                [LOWER_CASE_IDENTIFIER],
                [MODULE_NAME_SEGMENT, DOT, .., LOWER_CASE_IDENTIFIER],
            ) => on_added_module_qualifier_to_value(
                &mut self.kb,
                &self.query_for_unqualified_values,
                &self.query_for_imports,
                &self.query_for_qualified_value,
                diff,
                changes,
            )?,
            // Remove entire exposing list.
            ([EXPOSING_LIST], []) => on_removed_exposing_list_from_import(
                &mut self.kb,
                &self.query_for_unqualified_values,
                &self.query_for_imports,
                diff,
                changes,
            )?,
            ([EXPOSED_UNION_CONSTRUCTORS], []) => {
                on_removed_constructors_from_exposing_list(
                    &mut self.kb,
                    &self.query_for_unqualified_values,
                    &self.query_for_imports,
                    diff,
                    changes,
                )?
            }
            _ => Vec::new(),
        };
        if edits.is_empty() {
            Ok(None)
        } else {
            Ok(Some(sort_edits(edits)))
        }
    }
}

fn on_removed_constructors_from_exposing_list(
    kb: &mut KnowledgeBase,
    query_for_unqualified_values: &UnqualifiedValuesQuery,
    query_for_imports: &ImportsQuery,
    diff: &SourceFileDiff,
    changes: TreeChanges,
) -> Result<Vec<Edit>, Error> {
    // TODO: remove unwrap()'s, clone()'s, and otherwise clean up.
    let node = changes.old_removed.first().unwrap();
    let exposed_type_node = node.parent().unwrap();
    let type_name = diff
        .old
        .slice(&exposed_type_node.child(0).unwrap().byte_range());
    let old_import_node = exposed_type_node.parent().unwrap().parent().unwrap();
    let mut cursor = QueryCursor::new();
    let old_import = query_for_imports
        .run_in(&mut cursor, &diff.old, old_import_node)
        .next()
        .ok_or(Error::TreeSitterExpectedNodeDoesNotExist)?;
    let mut edits = Vec::new();
    let mut cursor2 = QueryCursor::new();
    for (_, exposed) in old_import.exposing_list() {
        if let Exposed::Type(type_) = exposed {
            if type_.name == type_name {
                add_qualifier_to_constructors(
                    query_for_unqualified_values,
                    &mut edits,
                    &mut cursor2,
                    &diff.new,
                    &old_import,
                    type_.constructors(kb)?,
                );
                break;
            }
        }
    }
    Ok(edits)
}

fn on_removed_values_from_exposing_list(
    kb: &mut KnowledgeBase,
    query_for_unqualified_values: &UnqualifiedValuesQuery,
    query_for_imports: &ImportsQuery,
    diff: &SourceFileDiff,
    changes: TreeChanges,
) -> Result<Vec<Edit>, Error> {
    match changes.new_added.as_slice() {
        [] => {}
        [node] if diff.new.slice(&node.byte_range()) == "" => {}
        _ => return Ok(Vec::new()),
    }

    // TODO: Figure out better approach to tree-traversal.
    let old_import_node = changes
        .old_removed
        .first()
        .ok_or(Error::TreeSitterExpectedNodeDoesNotExist)?
        .parent()
        .ok_or(Error::TreeSitterExpectedNodeDoesNotExist)?
        .parent()
        .ok_or(Error::TreeSitterExpectedNodeDoesNotExist)?;
    let mut cursor = QueryCursor::new();
    let old_import = query_for_imports
        .run_in(&mut cursor, &diff.old, old_import_node)
        .next()
        .ok_or(Error::TreeSitterExpectedNodeDoesNotExist)?;

    // Modify the import in the new code with the name just found.
    let mut edits = Vec::new();
    let mut cursor2 = QueryCursor::new();
    let import = query_for_imports
        .run(&mut cursor2, &diff.new)
        .find(|import| import.name() == old_import.name())
        .ok_or(Error::TreeSitterExpectedNodeDoesNotExist)?;
    let new_exposed_count = import.exposing_list().count();
    if new_exposed_count == 0 {
        remove_exposing_list(&mut edits, &diff.new, &import);
    }
    let removed_range = changes.old_removed.first().unwrap().start_byte()
        ..changes.old_removed.last().unwrap().end_byte();
    let mut exposed_cursor = QueryCursor::new();
    old_import
        .exposing_list()
        .into_iter()
        .for_each(|(node, exposed)| {
            if node.start_byte() >= removed_range.start
                && node.end_byte() <= removed_range.end
            {
                add_qualifier_to_name(
                    kb,
                    query_for_unqualified_values,
                    &mut edits,
                    &mut exposed_cursor,
                    &diff.new,
                    &import,
                    &exposed,
                )
            }
        });
    Ok(edits)
}

fn on_added_module_qualifier_to_value(
    kb: &mut KnowledgeBase,
    query_for_unqualified_values: &UnqualifiedValuesQuery,
    query_for_imports: &ImportsQuery,
    query_for_qualified_value: &QualifiedValueQuery,
    diff: &SourceFileDiff,
    changes: TreeChanges,
) -> Result<Vec<Edit>, Error> {
    let name_before = diff.old.slice(
        &changes
            .old_removed
            .first()
            .ok_or(Error::TreeSitterExpectedNodeDoesNotExist)?
            .byte_range(),
    );
    let parent = changes
        .new_added
        .first()
        .ok_or(Error::TreeSitterExpectedNodeDoesNotExist)?
        .parent()
        .ok_or(Error::TreeSitterExpectedNodeDoesNotExist)?;
    let mut cursor = QueryCursor::new();
    let QualifiedValue { qualifier, name } =
        query_for_qualified_value.run_in(&mut cursor, &diff.new, parent)?;
    if name_before != name {
        return Ok(Vec::new());
    }
    let mut edits = Vec::new();
    let import = get_import_by_aliased_name(
        query_for_imports,
        &mut cursor,
        diff,
        &qualifier,
    )?;

    let exposing_list_length = import.exposing_list().count();
    for (node, exposed) in import.exposing_list() {
        match &exposed {
            Exposed::Operator(_) => panic!("unimplemented"),
            Exposed::Type(type_) => {
                if type_.name == name {
                    if exposing_list_length == 1 {
                        remove_exposing_list(&mut edits, &diff.new, &import);
                    } else {
                        remove_from_exposing_list(&mut edits, diff, &node)?;
                    }
                    let mut cursor2 = QueryCursor::new();
                    add_qualifier_to_type(
                        query_for_unqualified_values,
                        &mut edits,
                        &mut cursor2,
                        &diff.new,
                        &import,
                        type_,
                    );
                    break;
                }

                let constructors = type_.constructors(kb)?;
                if constructors.clone().any(|ctor| *ctor == name) {
                    // Remove `(..)` behind type from constructor this.
                    edits.push(Edit::new(
                        diff.new.buffer,
                        &mut diff.new.bytes.clone(),
                        &node.child(1).unwrap().byte_range(),
                        String::new(),
                    ));

                    // We're qualifying a constructor. In Elm you can only
                    // expose either all constructors of a type or none of them,
                    // so if the programmer qualifies one constructor assume
                    // intend to do them all.
                    let mut cursor2 = QueryCursor::new();
                    add_qualifier_to_constructors(
                        query_for_unqualified_values,
                        &mut edits,
                        &mut cursor2,
                        &diff.new,
                        &import,
                        constructors,
                    );
                    break;
                }
            }
            Exposed::Value(val) => {
                if val.name == name {
                    if exposing_list_length == 1 {
                        remove_exposing_list(&mut edits, &diff.new, &import);
                    } else {
                        remove_from_exposing_list(&mut edits, diff, &node)?;
                    }
                    let mut cursor2 = QueryCursor::new();
                    add_qualifier_to_value(
                        query_for_unqualified_values,
                        &mut edits,
                        &mut cursor2,
                        &diff.new,
                        &import,
                        val,
                    );
                    break;
                }
            }
            Exposed::All(_) => {
                // The programmer qualified a value coming from a module that
                // exposes everything. We could interpret this to mean that the
                // programmer wishes to qualify all values of this module. That
                // would potentially result in a lot of changes though, so we're
                // going to be more conservative and qualify only other
                // occurences of the same value. If the programmer really wishes
                // to qualify everything they can indicate so by removing the
                // `exposing (..)` clause.
                //
                // TODO: distinguish between the between types and constructors
                // here, so we don't qualify the wrong one if they have the
                // same names.
                let mut cursor2 = QueryCursor::new();
                add_qualifier_to_value(
                    query_for_unqualified_values,
                    &mut edits,
                    &mut cursor2,
                    &diff.new,
                    &import,
                    &ExposedValue { name },
                );
                break;
            }
        }
    }
    Ok(edits)
}

fn on_removed_exposing_list_from_import(
    kb: &mut KnowledgeBase,
    query_for_unqualified_values: &UnqualifiedValuesQuery,
    query_for_imports: &ImportsQuery,
    diff: &SourceFileDiff,
    changes: TreeChanges,
) -> Result<Vec<Edit>, Error> {
    let import_node = changes
        .old_removed
        .first()
        .and_then(Node::parent)
        .ok_or(Error::TreeSitterExpectedNodeDoesNotExist)?;
    let mut cursor = QueryCursor::new();
    let import = query_for_imports
        .run_in(&mut cursor, &diff.old, import_node)
        .next()
        .ok_or(Error::TreeSitterExpectedNodeDoesNotExist)?;
    let qualifier = import.aliased_name();
    let mut cursor_2 = QueryCursor::new();
    let mut edits = Vec::new();
    query_for_imports
        .run(&mut cursor_2, &diff.old)
        .find(|import| import.aliased_name() == qualifier)
        .ok_or(Error::TreeSitterExpectedNodeDoesNotExist)?
        .exposing_list()
        .for_each(|(_, exposed)| {
            let mut val_cursor = QueryCursor::new();
            add_qualifier_to_name(
                kb,
                query_for_unqualified_values,
                &mut edits,
                &mut val_cursor,
                &diff.new,
                &import,
                &exposed,
            )
        });
    Ok(edits)
}

fn remove_exposing_list(
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

fn get_import_by_aliased_name<'a>(
    query_for_imports: &'a ImportsQuery,
    cursor: &'a mut QueryCursor,
    diff: &'a SourceFileDiff,
    qualifier: &'a RopeSlice,
) -> Result<Import<'a>, Error> {
    query_for_imports
        .run(cursor, &diff.new)
        .find(|import| import.aliased_name() == *qualifier)
        .ok_or(Error::TreeSitterExpectedNodeDoesNotExist)
}

fn remove_from_exposing_list(
    edits: &mut Vec<Edit>,
    diff: &SourceFileDiff,
    node: &Node,
) -> Result<(), Error> {
    // TODO: Automatically clean up extra or missing comma's.
    let range_including_comma_and_whitespace = |exposed_node: &Node| {
        let next = exposed_node.next_sibling();
        if let Some(node) = next {
            if node.kind_id() == COMMA {
                let end_byte = match node.next_sibling() {
                    Some(next) => next.start_byte(),
                    None => node.end_byte(),
                };
                return exposed_node.start_byte()..end_byte;
            }
        }
        let prev = exposed_node.prev_sibling();
        if let Some(node) = prev {
            if node.kind_id() == COMMA {
                let start_byte = match node.prev_sibling() {
                    Some(prev) => prev.end_byte(),
                    None => node.start_byte(),
                };
                return start_byte..exposed_node.end_byte();
            }
        }
        exposed_node.byte_range()
    };
    edits.push(Edit::new(
        diff.new.buffer,
        &mut diff.new.bytes.clone(),
        &range_including_comma_and_whitespace(node),
        String::new(),
    ));
    Ok(())
}

fn add_qualifier_to_name(
    kb: &mut KnowledgeBase,
    query_for_unqualified_values: &UnqualifiedValuesQuery,
    edits: &mut Vec<Edit>,
    cursor: &mut QueryCursor,
    code: &SourceFileSnapshot,
    import: &Import,
    exposed: &Exposed,
) {
    match exposed {
        Exposed::Operator(_) => panic!("unimplemented"),
        Exposed::Type(type_) => {
            add_qualifier_to_type(
                query_for_unqualified_values,
                edits,
                cursor,
                code,
                import,
                type_,
            );
            let ctors = match type_.constructors(kb) {
                Ok(ctors_) => ctors_,
                Err(err) => {
                    return eprintln!(
                        "[error] failed to read constructors of {}: {:?}",
                        type_.name.to_string(),
                        err
                    );
                }
            };
            add_qualifier_to_constructors(
                query_for_unqualified_values,
                edits,
                cursor,
                code,
                import,
                ctors,
            );
        }
        Exposed::Value(val) => {
            add_qualifier_to_value(
                query_for_unqualified_values,
                edits,
                cursor,
                code,
                import,
                val,
            );
        }
        Exposed::All(_) => {
            let exports = match kb.module_exports(code.buffer, import.name()) {
                Ok(exports_) => exports_,
                Err(err) => {
                    return eprintln!(
                        "[error] failed to read exports of {}: {:?}",
                        import.name().to_string(),
                        err
                    );
                }
            };
            for export in exports {
                match export {
                    ElmExport::Value { name } => add_qualifier_to_value(
                        query_for_unqualified_values,
                        edits,
                        cursor,
                        code,
                        import,
                        &ExposedValue {
                            name: Rope::from_str(name).slice(..),
                        },
                    ),
                    ElmExport::Type { name, constructors } => {
                        add_qualifier_to_type(
                            query_for_unqualified_values,
                            edits,
                            cursor,
                            code,
                            import,
                            &ExposedType {
                                buffer: code.buffer,
                                exposing_constructors: false,
                                module_name: import.name(),
                                name: Rope::from_str(name).slice(..),
                            },
                        );
                        add_qualifier_to_constructors(
                            query_for_unqualified_values,
                            edits,
                            cursor,
                            code,
                            import,
                            ExposedTypeConstructors::All {
                                names: constructors,
                            },
                        );
                    }
                }
            }
        }
    }
}

fn add_qualifier_to_constructors(
    query_for_unqualified_values: &UnqualifiedValuesQuery,
    edits: &mut Vec<Edit>,
    cursor: &mut QueryCursor,
    code: &SourceFileSnapshot,
    import: &Import,
    constructors: ExposedTypeConstructors,
) {
    for ctor in constructors {
        query_for_unqualified_values
            .run(cursor, code)
            .for_each(|node| {
                if *ctor == code.slice(&node.byte_range()) {
                    edits.push(Edit::new(
                        code.buffer,
                        &mut code.bytes.clone(),
                        &(node.start_byte()..node.start_byte()),
                        format!("{}.", import.aliased_name()),
                    ))
                }
            })
    }
}

fn add_qualifier_to_type(
    query_for_unqualified_values: &UnqualifiedValuesQuery,
    edits: &mut Vec<Edit>,
    cursor: &mut QueryCursor,
    code: &SourceFileSnapshot,
    import: &Import,
    exposed: &ExposedType,
) {
    let exposed_name = exposed.name;
    query_for_unqualified_values
        .run(cursor, code)
        .for_each(|node| {
            if exposed_name == code.slice(&node.byte_range()) {
                edits.push(Edit::new(
                    code.buffer,
                    &mut code.bytes.clone(),
                    &(node.start_byte()..node.start_byte()),
                    format!("{}.", import.aliased_name()),
                ))
            }
        })
}

fn add_qualifier_to_value(
    query_for_unqualified_values: &UnqualifiedValuesQuery,
    edits: &mut Vec<Edit>,
    cursor: &mut QueryCursor,
    code: &SourceFileSnapshot,
    import: &Import,
    exposed: &ExposedValue,
) {
    let exposed_name = exposed.name;
    query_for_unqualified_values
        .run(cursor, code)
        .for_each(|node| {
            if exposed_name == code.slice(&node.byte_range()) {
                edits.push(Edit::new(
                    code.buffer,
                    &mut code.bytes.clone(),
                    &(node.start_byte()..node.start_byte()),
                    format!("{}.", import.aliased_name()),
                ))
            }
        })
}

struct QualifiedValueQuery {
    query: Query,
    qualifier_index: u32,
    name_index: u32,
}

impl QualifiedValueQuery {
    fn init(lang: Language) -> Result<QualifiedValueQuery, Error> {
        let query_str = r#"
            (_
              (
                (module_name_segment) @qualifier
                (dot)
              )+
              [ (lower_case_identifier) (upper_case_identifier) ] @name
            )
            "#;
        let query = Query::new(lang, query_str)
            .map_err(Error::TreeSitterFailedToParseQuery)?;
        let qualified_value_query = QualifiedValueQuery {
            qualifier_index: index_for_name(&query, "qualifier")?,
            name_index: index_for_name(&query, "name")?,
            query,
        };
        Ok(qualified_value_query)
    }

    fn run_in<'a>(
        &self,
        cursor: &mut QueryCursor,
        code: &'a SourceFileSnapshot,
        node: Node<'a>,
    ) -> Result<QualifiedValue<'a>, Error> {
        let mut qualifier_range = None;
        let mut name = None;
        cursor
            .matches(&self.query, node, code)
            .next()
            .ok_or(Error::TreeSitterQueryReturnedNotEnoughMatches)?
            .captures
            .iter()
            .for_each(|capture| {
                if capture.index == self.qualifier_index {
                    match &qualifier_range {
                        None => {
                            qualifier_range = Some(capture.node.byte_range())
                        }
                        Some(existing_range) => {
                            qualifier_range = Some(
                                existing_range.start..capture.node.end_byte(),
                            )
                        }
                    }
                } else if capture.index == self.name_index {
                    name = Some(code.slice(&capture.node.byte_range()))
                }
            });
        let val = QualifiedValue {
            name: name.ok_or(Error::TreeSitterQueryReturnedNotEnoughMatches)?,
            qualifier: code.slice(
                &qualifier_range
                    .ok_or(Error::TreeSitterQueryReturnedNotEnoughMatches)?,
            ),
        };
        Ok(val)
    }
}

struct QualifiedValue<'a> {
    qualifier: RopeSlice<'a>,
    name: RopeSlice<'a>,
}

struct UnqualifiedValuesQuery {
    query: Query,
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
        let query = Query::new(lang, query_str)
            .map_err(Error::TreeSitterFailedToParseQuery)?;
        let unqualified_values_query = UnqualifiedValuesQuery { query };
        Ok(unqualified_values_query)
    }

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

struct ImportsQuery {
    query: Query,
    root_index: u32,
    name_index: u32,
    as_clause_index: u32,
    exposing_list_index: u32,
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
        let query = Query::new(lang, query_str)
            .map_err(Error::TreeSitterFailedToParseQuery)?;
        let imports_query = ImportsQuery {
            root_index: index_for_name(&query, "root")?,
            name_index: index_for_name(&query, "name")?,
            as_clause_index: index_for_name(&query, "as_clause")?,
            exposing_list_index: index_for_name(&query, "exposing_list")?,
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
            _root_node: nodes[self.query.root_index as usize]?,
            name_node: nodes[self.query.name_index as usize]?,
            as_clause_node: nodes[self.query.as_clause_index as usize],
            exposing_list_node: nodes[self.query.exposing_list_index as usize],
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

    fn exposing_list(&self) -> ExposedList<'_> {
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
            module_name: self.code.slice(&self.name_node.byte_range()),
            cursor,
        }
    }
}

struct ExposedList<'a> {
    code: &'a SourceFileSnapshot,
    cursor: Option<TreeCursor<'a>>,
    module_name: RopeSlice<'a>,
}

impl<'a> Iterator for ExposedList<'a> {
    type Item = (Node<'a>, Exposed<'a>);

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
                let exposed = match node.kind_id() {
                    EXPOSED_VALUE => Exposed::Value(ExposedValue {
                        name: self.code.slice(&node.byte_range()),
                    }),
                    EXPOSED_OPERATOR => Exposed::Operator(ExposedOperator {
                        _name: self.code.slice(&node.byte_range()),
                    }),
                    EXPOSED_TYPE => Exposed::Type(ExposedType {
                        name: self
                            .code
                            .slice(&node.child(0).unwrap().byte_range()),
                        exposing_constructors: node.child(1).is_some(),
                        buffer: self.code.buffer,
                        module_name: self.module_name,
                    }),
                    DOUBLE_DOT => Exposed::All(node),
                    _ => panic!("unexpected exposed kind"),
                };
                return Some((node, exposed));
            }
        }
        None
    }
}

enum Exposed<'a> {
    Operator(ExposedOperator<'a>),
    Value(ExposedValue<'a>),
    Type(ExposedType<'a>),
    All(Node<'a>),
}

struct ExposedOperator<'a> {
    _name: RopeSlice<'a>,
}

struct ExposedValue<'a> {
    name: RopeSlice<'a>,
}

struct ExposedType<'a> {
    name: RopeSlice<'a>,
    buffer: Buffer,
    module_name: RopeSlice<'a>,
    exposing_constructors: bool,
}

impl ExposedType<'_> {
    fn constructors<'a>(
        &'a self,
        kb: &'a mut KnowledgeBase,
    ) -> Result<ExposedTypeConstructors, Error> {
        if !self.exposing_constructors {
            return Ok(ExposedTypeConstructors::None);
        }
        let names =
            kb.constructors_for_type(self.buffer, self.module_name, self.name)?;
        Ok(ExposedTypeConstructors::All { names })
    }
}

#[derive(Clone)]
enum ExposedTypeConstructors<'a> {
    None,
    All { names: &'a [String] },
}

impl<'a> Iterator for ExposedTypeConstructors<'a> {
    type Item = &'a String;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            ExposedTypeConstructors::None => None,
            ExposedTypeConstructors::All { names } => match names {
                [] => None,
                [head, tail @ ..] => {
                    *names = tail;
                    Some(head)
                }
            },
        }
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

fn attach_kinds(nodes: &[Node]) -> Vec<u16> {
    nodes.iter().map(|node| node.kind_id()).collect()
}

fn index_for_name(query: &Query, name: &str) -> Result<u32, Error> {
    query
        .capture_index_for_name(name)
        .ok_or(Error::TreeSitterQueryDoesNotHaveExpectedIndex)
}

// TODO: remove debug helper when it's no longer needed.
#[allow(dead_code)]
fn debug_print_tree_changes(diff: &SourceFileDiff, changes: &TreeChanges) {
    println!("REMOVED NODES:");
    for node in &changes.old_removed {
        crate::debug_print_node(&diff.old, 2, node);
    }
    println!("ADDED NODES:");
    for node in &changes.new_added {
        crate::debug_print_node(&diff.new, 2, node);
    }
}

#[cfg(test)]
mod tests {
    use crate::analysis_thread::{diff_trees, SourceFileDiff};
    use crate::languages::elm::RefactorEngine;
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
                let module_name = stringify!($name);
                path.push(module_name.to_owned() + ".elm");
                println!("Run simulation {:?}", &path);
                run_simulation_test(&path);
            }
        };
    }

    fn run_simulation_test(path: &Path) {
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
        let mut refactor_engine = RefactorEngine::new()?;
        refactor_engine
            .kb
            .insert_buffer_path(buffer, path.to_owned());
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

    simulation_test!(add_module_alias_as_qualifier_to_variable);
    simulation_test!(add_module_qualifier_to_constructor);
    simulation_test!(
        add_module_qualifier_to_constructor_from_expose_all_import
    );
    simulation_test!(add_module_qualifier_to_type);
    simulation_test!(add_module_qualifier_to_type_with_same_name);
    simulation_test!(add_module_qualifier_to_value_from_exposing_all_import);
    simulation_test!(add_module_qualifier_to_variable);
    simulation_test!(remove_constructor_from_exposing_list_of_import);
    simulation_test!(remove_double_dot_from_exposing_list_of_import);
    simulation_test!(remove_exposing_all_clause_from_import);
    simulation_test!(remove_exposing_clause_from_import);
    simulation_test!(remove_exposing_clause_from_import_with_as_clause);
    simulation_test!(remove_all_values_from_exposing_list_of_import);
    simulation_test!(remove_multiple_values_from_exposing_list_of_import);
    simulation_test!(remove_type_with_constructor_from_exposing_list_of_import);
    simulation_test!(remove_value_from_exposing_list_of_import_with_as_clause);
    simulation_test!(remove_variable_from_exposing_list_of_import);

    #[derive(Debug)]
    enum Error {
        RunningSimulation(crate::test_support::simulation::Error),
        ElmPair(crate::Error),
    }

    impl From<crate::test_support::simulation::Error> for Error {
        fn from(err: crate::test_support::simulation::Error) -> Error {
            Error::RunningSimulation(err)
        }
    }

    impl From<crate::Error> for Error {
        fn from(err: crate::Error) -> Error {
            Error::ElmPair(err)
        }
    }
}
