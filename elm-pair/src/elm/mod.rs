use crate::analysis_thread::{SourceFileDiff, TreeChanges};
use crate::elm::compiler::Compiler;
use crate::elm::dependencies::DataflowComputation;
use crate::elm::io::ExportedName;
use crate::elm::queries::imports;
use crate::elm::queries::imports::{ExposedConstructors, Import};
use crate::elm::queries::qualified_values;
use crate::elm::queries::qualified_values::QualifiedName;
use crate::elm::queries::unqualified_values;
use crate::lib::log;
use crate::lib::log::Error;
use crate::lib::source_code::{Buffer, Edit, SourceFileSnapshot};
use core::ops::Range;
use ropey::Rope;
use std::collections::HashSet;
use std::path::Path;
use tree_sitter::{Node, QueryCursor};

pub mod compiler;
pub mod dependencies;
pub mod io;
pub mod module_name;
pub mod project;
pub mod queries;
pub mod refactors;

// Macro for defining constants for the elm tree-sitter node kinds. This macro
// ensures a test is added checking each constant is correct.
macro_rules! node_constants {
    (@id $language:expr, $name:ident) => {
        $language.id_for_node_kind(&stringify!($name).to_lowercase(), true)
    };

    (@id $language:expr, $_:ident, $name:expr) => {
        $language.id_for_node_kind($name, false)
    };

    ($($name:ident $(($kind_name:expr))? = $kind_id:expr;)+) => {
        $(const $name: u16 = $kind_id;)+

        #[cfg(test)]
        mod kind_constants {
            use super::*;

            #[test]
            fn check_kind_constants() {
                let language = tree_sitter_elm::language();
                $(
                    assert_eq!(
                        $name,
                        node_constants!(@id language, $name $(,$kind_name)?),
                    );
                )+
            }
        }
    };
}

// These constants come from the tree-sitter-elm grammar. They might need to
// be changed when tree-sitter-elm updates.
node_constants!(
    AS_CLAUSE = 101;
    BLOCK_COMMENT = 86;
    COMMA (",") = 6;
    CONSTRUCTOR_IDENTIFIER = 8;
    CONSTRUCTOR_QID = 96;
    DOT = 55;
    DOUBLE_DOT = 49;
    EXPOSED_OPERATOR = 94;
    EXPOSED_TYPE = 92;
    EXPOSED_UNION_CONSTRUCTORS = 93;
    EXPOSED_VALUE = 91;
    EXPOSING_LIST = 90;
    LOWER_CASE_IDENTIFIER = 1;
    MODULE_NAME_SEGMENT = 201;
    MODULE_DECLARATION = 87;
    TYPE_IDENTIFIER = 33;
    TYPE_QID = 97;
    VALUE_QID = 98;
);

pub struct RefactorEngine {
    dataflow_computation: DataflowComputation,
    queries: Queries,
}

pub struct Queries {
    query_for_imports: imports::Query,
    query_for_unqualified_values: unqualified_values::Query,
    query_for_qualified_values: qualified_values::Query,
}

pub struct Refactor {
    replacements: Vec<(Range<usize>, String)>,
}

impl Refactor {
    fn new() -> Refactor {
        Refactor {
            replacements: Vec::new(),
        }
    }

    fn add_change(&mut self, range: Range<usize>, new_bytes: String) {
        self.replacements.push((range, new_bytes))
    }

    pub fn edits(
        mut self,
        code: &mut SourceFileSnapshot,
    ) -> Result<Vec<Edit>, Error> {
        // Sort edits in reverse order of where they change the source file. This
        // ensures when we apply the edits in sorted order that earlier edits don't
        // move the area of affect of later edits.
        //
        // We're assuming here that the areas of operation of different edits never
        // overlap.
        self.replacements
            .sort_by(|(x, _), (y, _)| y.start.cmp(&x.end));

        let mut edits = Vec::with_capacity(self.replacements.len());
        for (range, new_bytes) in self.replacements {
            let edit =
                Edit::new(code.buffer, &mut code.bytes, &range, new_bytes);
            code.apply_edit(edit.input_edit)?;
            edits.push(edit);
        }
        Ok(edits)
    }
}

impl RefactorEngine {
    pub fn new(compiler: Compiler) -> Result<RefactorEngine, Error> {
        let language = tree_sitter_elm::language();
        let engine = RefactorEngine {
            dataflow_computation: DataflowComputation::new(compiler)?,
            queries: Queries {
                query_for_imports: imports::Query::init(language)?,
                query_for_unqualified_values: unqualified_values::Query::init(
                    language,
                )?,
                query_for_qualified_values: qualified_values::Query::init(
                    language,
                )?,
            },
        };

        Ok(engine)
    }

    // TODO: try to return an Iterator instead of a Vector.
    // TODO: Try remove Vector from TreeChanges type.
    pub fn respond_to_change<'a>(
        &mut self,
        diff: &SourceFileDiff,
        changes: TreeChanges<'a>,
    ) -> Result<Refactor, Error> {
        #[cfg(debug_assertions)]
        debug_print_tree_changes(diff, &changes);
        #[cfg(not(debug_assertions))]
        if changes.old_removed.is_empty() && changes.new_added.is_empty() {
            return Ok(Refactor::new());
        }
        let before = attach_kinds(&changes.old_removed);
        let after = attach_kinds(&changes.new_added);
        let mut refactor = Refactor::new();

        match (Change {
            before: before.as_slice(),
            after: after.as_slice(),
            parent: changes.old_parent.kind_id(),
        }) {
            Change {
                before:
                    [EXPOSED_VALUE | EXPOSED_TYPE, ..]
                    | [COMMA, EXPOSED_VALUE | EXPOSED_TYPE, ..]
                    | [DOUBLE_DOT]
                    | [],
                after:
                    [EXPOSED_VALUE | EXPOSED_TYPE, ..]
                    | [COMMA, EXPOSED_VALUE | EXPOSED_TYPE, ..]
                    | [DOUBLE_DOT]
                    | [],
                parent: _,
            } => {
                let old_import_node =
                    changes.old_parent.parent().ok_or_else(|| {
                        log::mk_err!(
                        "could not find parent import node of exposing list"
                    )
                    })?;
                let old_import = parse_import_node(
                    &self.queries,
                    &diff.old,
                    old_import_node,
                )?;
                let new_import_node =
                    changes.new_parent.parent().ok_or_else(|| {
                        log::mk_err!(
            "could not find import node as parent of exposing list node"
        )
                    })?;
                let new_import = parse_import_node(
                    &self.queries,
                    &diff.new,
                    new_import_node,
                )?;
                refactors::changed_values_in_exposing_list::refactor(
                    &self.queries,
                    &mut self.dataflow_computation,
                    &mut refactor,
                    &diff.new,
                    old_import,
                    new_import,
                )?
            }
            Change {
                before: [TYPE_IDENTIFIER],
                after: [MODULE_NAME_SEGMENT, DOT, .., TYPE_IDENTIFIER],
                parent: _,
            }
            | Change {
                before: [CONSTRUCTOR_IDENTIFIER],
                after: [MODULE_NAME_SEGMENT, DOT, .., CONSTRUCTOR_IDENTIFIER],
                parent: _,
            }
            | Change {
                before: [LOWER_CASE_IDENTIFIER],
                after: [MODULE_NAME_SEGMENT, DOT, .., LOWER_CASE_IDENTIFIER],
                parent: _,
            } => {
                let mut cursor = QueryCursor::new();
                let (_, old_reference) = self
                    .queries
                    .query_for_unqualified_values
                    .run_in(&mut cursor, &diff.old, changes.old_parent)
                    .next()
                    .ok_or_else(|| {
                        log::mk_err!(
                            "parsing unqualified value node using query failed"
                        )
                    })??;
                let (_, qualified_name) = self
                    .queries
                    .query_for_qualified_values
                    .run_in(&mut cursor, &diff.new, changes.new_parent)
                    .next()
                    .ok_or_else(|| {
                        log::mk_err!(
                            "parsing qualified value node using query failed"
                        )
                    })??;
                if old_reference.name == qualified_name.unqualified_name.name {
                    refactors::added_module_qualifier_to_name::refactor(
                        &self.queries,
                        &mut self.dataflow_computation,
                        &mut refactor,
                        &diff.new,
                        qualified_name,
                    )?
                }
            }
            Change {
                before: [MODULE_NAME_SEGMENT, DOT, .., TYPE_IDENTIFIER],
                after: [TYPE_IDENTIFIER],
                parent: _,
            }
            | Change {
                before: [MODULE_NAME_SEGMENT, DOT, .., CONSTRUCTOR_IDENTIFIER],
                after: [CONSTRUCTOR_IDENTIFIER],
                parent: _,
            }
            | Change {
                before: [MODULE_NAME_SEGMENT, DOT, .., LOWER_CASE_IDENTIFIER],
                after: [LOWER_CASE_IDENTIFIER],
                parent: _,
            } => {
                let mut cursor = QueryCursor::new();
                let (node, new_reference) = self
                    .queries
                    .query_for_unqualified_values
                    .run_in(&mut cursor, &diff.new, changes.new_parent)
                    .next()
                    .ok_or_else(|| {
                        log::mk_err!(
                            "parsing unqualified value node using query failed"
                        )
                    })??;
                let mut cursor2 = QueryCursor::new();
                let (_, qualified_name) = self
                    .queries
                    .query_for_qualified_values
                    .run_in(&mut cursor2, &diff.old, changes.old_parent)
                    .next()
                    .ok_or_else(|| {
                        log::mk_err!(
                            "parsing qualified value node using query failed"
                        )
                    })??;
                if new_reference.name == qualified_name.unqualified_name.name {
                    refactors::removed_module_qualifier_from_name::refactor(
                        &self.queries,
                        &mut self.dataflow_computation,
                        &mut refactor,
                        &diff.new,
                        node,
                        qualified_name,
                    )?
                }
            }
            Change {
                before: [],
                after: [EXPOSING_LIST],
                parent: _,
            } => {
                let import = parse_import_node(
                    &self.queries,
                    &diff.new,
                    changes.new_parent,
                )?;
                refactors::added_exposing_list_to_import::refactor(
                    &self.queries,
                    &mut self.dataflow_computation,
                    &mut refactor,
                    &diff.new,
                    import,
                )?
            }
            Change {
                before: [EXPOSING_LIST],
                after: [],
                parent: _,
            } => {
                let import = parse_import_node(
                    &self.queries,
                    &diff.old,
                    changes.old_parent,
                )?;
                refactors::removed_exposing_list_from_import::refactor(
                    &self.queries,
                    &mut self.dataflow_computation,
                    &mut refactor,
                    &diff.new,
                    import,
                )?
            }
            Change {
                before: [],
                after: [EXPOSED_UNION_CONSTRUCTORS],
                parent: _,
            } => {
                let type_name_node = changes.new_parent.child(0).ok_or_else(|| {
        log::mk_err!("did not find node with type name of exposed constructor")
    })?;
                let type_name = diff.new.slice(&type_name_node.byte_range());
                let import_node = changes
                    .new_parent
                    .parent()
                    .and_then(|n| n.parent())
                    .ok_or_else(|| {
                        log::mk_err!(
                            "did not find parent import of exposed constructor"
                        )
                    })?;
                let import =
                    parse_import_node(&self.queries, &diff.new, import_node)?;
                refactors::added_constructors_to_exposing_list::refactor(
                    &self.queries,
                    &mut self.dataflow_computation,
                    &mut refactor,
                    &diff.new,
                    import,
                    type_name,
                )?;
            }
            Change {
                before: [EXPOSED_UNION_CONSTRUCTORS],
                after: [],
                parent: _,
            } => {
                let type_name_node =
                    changes.old_parent.child(0).ok_or_else(|| {
                        log::mk_err!(
                            "could not find name node of exposed type node"
                        )
                    })?;
                let type_name = diff.old.slice(&type_name_node.byte_range());
                let old_import_node = changes
                    .old_parent
                    .parent()
                    .and_then(|n| n.parent())
                    .ok_or_else(|| {
                        log::mk_err!(
                "could not find import parent node of exposed type node"
            )
                    })?;
                let old_import = parse_import_node(
                    &self.queries,
                    &diff.old,
                    old_import_node,
                )?;
                refactors::removed_constructors_from_exposing_list::refactor(
                    &self.queries,
                    &mut self.dataflow_computation,
                    &mut refactor,
                    &diff.new,
                    old_import,
                    type_name,
                )?
            }
            Change {
                before: [] | [AS_CLAUSE],
                after: [AS_CLAUSE] | [],
                parent: _,
            }
            | Change {
                before: [.., MODULE_NAME_SEGMENT],
                after: [.., MODULE_NAME_SEGMENT],
                parent: AS_CLAUSE,
            } => {
                let (old_import_node, new_import_node) =
                    if changes.old_parent.kind_id() == AS_CLAUSE {
                        (
                            changes.old_parent.parent().ok_or_else(|| {
                                log::mk_err!(
                                    "found an unexpected root as_clause node"
                                )
                            })?,
                            changes.new_parent.parent().ok_or_else(|| {
                                log::mk_err!(
                                    "found an unexpected root as_clause node"
                                )
                            })?,
                        )
                    } else {
                        (changes.old_parent, changes.new_parent)
                    };
                let new_import = parse_import_node(
                    &self.queries,
                    &diff.new,
                    new_import_node,
                )?;
                let new_aliased_name = new_import.aliased_name();
                let old_import = parse_import_node(
                    &self.queries,
                    &diff.old,
                    old_import_node,
                )?;
                let old_aliased_name = old_import.aliased_name();
                refactors::changed_as_clause::refactor(
                    &self.queries,
                    &mut refactor,
                    &diff.new,
                    old_aliased_name,
                    new_aliased_name,
                )?;
            }
            Change {
                before: [.., MODULE_NAME_SEGMENT],
                after: [.., MODULE_NAME_SEGMENT],
                parent: VALUE_QID | TYPE_QID | CONSTRUCTOR_QID,
            } => {
                let mut cursor = QueryCursor::new();
                let (_, old_name) = self
                    .queries
                    .query_for_qualified_values
                    .run_in(&mut cursor, &diff.old, changes.old_parent)
                    .next()
                    .ok_or_else(|| {
                        log::mk_err!(
                            "parsing qualified value node using query failed"
                        )
                    })??;
                let (_, new_name) = self
                    .queries
                    .query_for_qualified_values
                    .run_in(&mut cursor, &diff.new, changes.new_parent)
                    .next()
                    .ok_or_else(|| {
                        log::mk_err!(
                            "parsing qualified value node using query failed"
                        )
                    })??;
                refactors::changed_module_qualifier::refactor(
                    &self.queries,
                    &mut refactor,
                    &diff.new,
                    old_name,
                    new_name,
                )?;
            }
            _ => {
                let unimported_qualifiers = find_unimported_qualifiers(
                    &self.queries,
                    &diff.new,
                    changes.new_parent,
                )?;
                if !unimported_qualifiers.is_empty() {
                    refactors::typed_unimported_qualified_value::refactor(
                        &mut self.dataflow_computation,
                        &mut refactor,
                        &diff.new,
                        unimported_qualifiers,
                    )?;
                }
            }
        }
        Ok(refactor)
    }

    pub fn init_buffer(
        &mut self,
        buffer: Buffer,
        path: &Path,
    ) -> Result<(), Error> {
        self.dataflow_computation
            .track_buffer(buffer, path.to_owned());
        self.dataflow_computation.advance();
        Ok(())
    }
}

struct Change<'a> {
    before: &'a [u16],
    after: &'a [u16],
    parent: u16,
}

#[allow(clippy::needless_collect)]
fn find_unimported_qualifiers(
    engine: &Queries,
    code: &SourceFileSnapshot,
    parent: Node,
) -> Result<HashSet<String>, Error> {
    let mut cursor = QueryCursor::new();
    let existing_imports: Vec<Rope> = engine
        .query_for_imports
        .run(&mut cursor, code)
        .map(|import| import.aliased_name().into())
        .collect();
    let mut unimported_qualifiers = HashSet::new();
    for result in
        engine
            .query_for_qualified_values
            .run_in(&mut cursor, code, parent)
    {
        let (_, reference) = result?;
        if reference.unqualified_name.name.len_bytes() > 0
            && !existing_imports.contains(&reference.qualifier)
        {
            unimported_qualifiers.insert(reference.qualifier.to_string());
        }
    }
    Ok(unimported_qualifiers)
}

#[derive(Clone, Debug, Eq)]
pub struct Name {
    pub name: Rope,
    pub kind: NameKind,
}

impl PartialEq for Name {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.kind == other.kind
    }
}

impl std::hash::Hash for Name {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // TODO: try to implement this without allocating a string.
        self.name.to_string().hash(state);
        self.kind.hash(state);
    }
}

#[derive(PartialEq, Clone, Copy, Debug, Hash)]
pub enum NameKind {
    Value,
    Type,
    Constructor,
    Operator,
}

impl Eq for NameKind {}

fn parse_import_node<'a>(
    engine: &'a Queries,
    code: &'a SourceFileSnapshot,
    node: Node<'a>,
) -> Result<Import<'a>, Error> {
    let mut cursor = QueryCursor::new();
    engine
        .query_for_imports
        .run_in(&mut cursor, code, node)
        .next()
        .ok_or_else(|| {
            log::mk_err!("query of import node did not result in any matches")
        })
}

fn attach_kinds(nodes: &[Node]) -> Vec<u16> {
    nodes.iter().map(|node| node.kind_id()).collect()
}

#[cfg(debug_assertions)]
fn debug_print_tree_changes(diff: &SourceFileDiff, changes: &TreeChanges) {
    println!("REMOVED NODES:");
    for node in &changes.old_removed {
        debug_print_node(&diff.old, 2, node);
    }
    println!("ADDED NODES:");
    for node in &changes.new_added {
        debug_print_node(&diff.new, 2, node);
    }
}

#[cfg(debug_assertions)]
fn debug_print_node(code: &SourceFileSnapshot, indent: usize, node: &Node) {
    println!(
        "{}[{} {:?}] {:?}{}",
        "  ".repeat(indent),
        node.kind(),
        node.kind_id(),
        code.slice(&node.byte_range()).to_string(),
        if node.has_changes() { " (changed)" } else { "" },
    );
}
