use crate::analysis_thread::{SourceFileDiff, TreeChanges};
use crate::editors;
use crate::elm::compiler::Compiler;
use crate::elm::dependencies::DataflowComputation;
use crate::elm::io::ExportedName;
use crate::elm::module_name::ModuleName;
use crate::elm::queries::imports::{ExposedConstructors, Import};
use crate::elm::queries::qualified_values::QualifiedName;
use crate::lib::log;
use crate::lib::log::Error;
use crate::lib::source_code::{Buffer, Edit, SourceFileSnapshot};
use core::ops::Range;
use ropey::Rope;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
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
        $(
            #[allow(dead_code)]
            const $name: u16 = $kind_id;
        )+

        #[cfg(test)]
        mod kind_constants {
            use super::*;

            #[test]
            fn check_kind_constants() {
                let language = tree_sitter_elm::language();
                $(
                    let name = stringify!($name);
                    assert_eq!(
                        (name, $name),
                        (name, node_constants!(@id language, $name $(,$kind_name)?)),
                    );
                )+
            }
        }
    };
}

// These constants come from the tree-sitter-elm grammar. They might need to
// be changed when tree-sitter-elm updates.
node_constants!(
    ARROW = 51;
    AS_CLAUSE = 101;
    BIN_OP_EXPR = 121;
    BLOCK_COMMENT = 86;
    CASE_OF_EXPR = 152;
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
    FILE = 85;
    FUNCTION_CALL_EXPR = 126;
    FUNCTION_DECLARATION_LEFT = 103;
    IF_ELSE_EXPR = 148;
    IMPORT_CLAUSE = 100;
    INFIX_DECLARATION = 170;
    LET_IN_EXPR = 155;
    LINE_COMMENT = 4;
    LOWER_CASE_IDENTIFIER = 1;
    LOWER_PATTERN = 161;
    MODULE_DECLARATION = 87;
    MODULE_NAME_SEGMENT = 201;
    NUMBER_CONSTANT_EXPR = 136;
    OPERATOR = 122;
    PARENTHESIZED_EXPR = 133;
    PATTERN = 157;
    PORT_ANNOTATION = 119;
    RECORD_PATTERN = 163;
    RECORD_TYPE = 115;
    STRING_CONSTANT_EXPR = 137;
    TYPE_ALIAS_DECLARATION = 108;
    TYPE_ANNOTATION = 118;
    TYPE_DECLARATION = 104;
    TYPE_IDENTIFIER = 33;
    TYPE_REF = 111;
    TYPE_QID = 97;
    UNION_VARIANT = 106;
    VALUE_EXPR = 139;
    VALUE_DECLARATION = 102;
    VALUE_QID = 98;
);

pub struct RefactorEngine {
    dataflow_computation: DataflowComputation,
    queries: Queries,
}

pub struct Queries {
    query_for_imports: queries::imports::Query,
    query_for_exports: queries::exports::Query,
    query_for_module_declaration: queries::module_declaration::Query,
    query_for_unqualified_values: queries::unqualified_values::Query,
    query_for_qualified_values: queries::qualified_values::Query,
    query_for_scopes: queries::scopes::Query,
}

pub struct Refactor {
    pub description: &'static str,
    replacements: Vec<(Buffer, Range<usize>, String)>,
    files_to_open: Vec<PathBuf>,
}

impl Refactor {
    fn new(description: &'static str) -> Refactor {
        Refactor {
            description,
            replacements: Vec::new(),
            files_to_open: Vec::new(),
        }
    }

    fn add_change(
        &mut self,
        buffer: Buffer,
        range: Range<usize>,
        new_bytes: String,
    ) {
        self.replacements.push((buffer, range, new_bytes))
    }

    fn open_files(&mut self, files: Vec<PathBuf>) {
        self.files_to_open = files;
    }

    pub fn changed_buffers(&self) -> HashSet<Buffer> {
        self.replacements
            .iter()
            .map(|(buffer, _, _)| buffer)
            .copied()
            .collect()
    }

    pub fn edits(
        mut self,
        code_by_buffer: &mut HashMap<Buffer, SourceFileSnapshot>,
    ) -> Result<(Vec<Edit>, Vec<PathBuf>), Error> {
        // Sort edits in reverse order of where they change the source file. This
        // ensures when we apply the edits in sorted order that earlier edits don't
        // move the area of affect of later edits.
        //
        // We're assuming here that the areas of operation of different edits never
        // overlap.
        self.replacements
            .sort_by(|(_, x, _), (_, y, _)| y.start.cmp(&x.end));

        let mut edits = Vec::with_capacity(self.replacements.len());
        for (buffer, range, new_bytes) in self.replacements {
            let code = if let Some(code_) = code_by_buffer.get_mut(&buffer) {
                code_
            } else {
                log::error!(
                    "could not find code to edit for buffer {:?}",
                    buffer
                );
                continue;
            };
            let edit =
                Edit::new(code.buffer, &mut code.bytes, &range, new_bytes);
            code.apply_edit(edit.input_edit)?;
            edits.push(edit);
        }
        Ok((edits, self.files_to_open))
    }
}

impl RefactorEngine {
    pub fn new(compiler: Compiler) -> Result<RefactorEngine, Error> {
        let language = tree_sitter_elm::language();
        let engine = RefactorEngine {
            dataflow_computation: DataflowComputation::new(compiler)?,
            queries: Queries {
                query_for_imports: queries::imports::Query::init(language)?,
                query_for_exports: queries::exports::Query::init(language)?,
                query_for_module_declaration:
                    queries::module_declaration::Query::init(language)?,
                query_for_unqualified_values:
                    queries::unqualified_values::Query::init(language)?,
                query_for_qualified_values:
                    queries::qualified_values::Query::init(language)?,
                query_for_scopes: queries::scopes::Query::init(language)?,
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
        buffers: &HashMap<Buffer, SourceFileSnapshot>,
        buffers_by_path: &HashMap<(editors::Id, PathBuf), Buffer>,
    ) -> Result<Refactor, Error> {
        #[cfg(debug_assertions)]
        debug_print_tree_changes(diff, &changes);
        #[cfg(not(debug_assertions))]
        if changes.old_removed.is_empty() && changes.new_added.is_empty() {
            return Ok(Refactor::new("empty refactor"));
        }
        let before = attach_kinds(&changes.old_removed);
        let after = attach_kinds(&changes.new_added);
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
                let old_import_node = changes.old_parent.parent().ok_or_else(|| {
                    log::mk_err!("could not find parent import node of exposing list")
                })?;
                let old_import = parse_import_node(
                    &self.queries,
                    &diff.old,
                    old_import_node,
                )?;
                let new_import_node = changes.new_parent.parent().ok_or_else(|| {
                    log::mk_err!("could not find import node as parent of exposing list node")
                })?;
                let new_import = parse_import_node(
                    &self.queries,
                    &diff.new,
                    new_import_node,
                )?;
                let mut refactor =
                    Refactor::new("changed exposing list of import");
                refactors::changed_values_in_exposing_list::refactor(
                    &self.queries,
                    &mut self.dataflow_computation,
                    &mut refactor,
                    &diff.new,
                    old_import,
                    new_import,
                )?;
                Ok(refactor)
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
                let old_name = self
                    .queries
                    .query_for_unqualified_values
                    .parse_single(&diff.old, changes.old_parent)?;
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
                let mut refactor = Refactor::new("added module qualifier");
                if old_name.name == qualified_name.unqualified_name.name {
                    refactors::added_module_qualifier_to_name::refactor(
                        &self.queries,
                        &mut self.dataflow_computation,
                        &mut refactor,
                        &diff.new,
                        qualified_name,
                    )?;
                }
                Ok(refactor)
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
                let (node, _, new_reference) = self
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
                let mut refactor = Refactor::new("removed module qualifier");
                if new_reference.name == qualified_name.unqualified_name.name {
                    refactors::removed_module_qualifier_from_name::refactor(
                        &self.queries,
                        &mut self.dataflow_computation,
                        &mut refactor,
                        &diff.new,
                        node,
                        qualified_name,
                    )?;
                }
                Ok(refactor)
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
                let mut refactor =
                    Refactor::new("added exposing list to import");
                refactors::added_exposing_list_to_import::refactor(
                    &self.queries,
                    &mut self.dataflow_computation,
                    &mut refactor,
                    &diff.new,
                    import,
                )?;
                Ok(refactor)
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
                let mut refactor =
                    Refactor::new("removed exposing list from import");
                refactors::removed_exposing_list_from_import::refactor(
                    &self.queries,
                    &mut self.dataflow_computation,
                    &mut refactor,
                    &diff.new,
                    import,
                )?;
                Ok(refactor)
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
                let mut refactor =
                    Refactor::new("exposed constructors in import");
                refactors::added_constructors_to_exposing_list::refactor(
                    &self.queries,
                    &mut self.dataflow_computation,
                    &mut refactor,
                    &diff.new,
                    import,
                    type_name,
                )?;
                Ok(refactor)
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
                        log::mk_err!("could not find import parent node of exposed type node")
                    })?;
                let old_import = parse_import_node(
                    &self.queries,
                    &diff.old,
                    old_import_node,
                )?;
                let mut refactor =
                    Refactor::new("stopped exposing constructors in import");
                refactors::removed_constructors_from_exposing_list::refactor(
                    &self.queries,
                    &mut self.dataflow_computation,
                    &mut refactor,
                    &diff.new,
                    old_import,
                    type_name,
                )?;
                Ok(refactor)
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
                let mut refactor = Refactor::new("changed as-clause of import");
                refactors::changed_as_clause::refactor(
                    &self.queries,
                    &mut refactor,
                    &diff.new,
                    old_aliased_name,
                    new_aliased_name,
                )?;
                Ok(refactor)
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
                let mut refactor = Refactor::new("changed module qualifier");
                refactors::changed_module_qualifier::refactor(
                    &self.queries,
                    &mut refactor,
                    &diff.new,
                    old_name,
                    new_name,
                )?;
                Ok(refactor)
            }
            Change {
                before: [LOWER_CASE_IDENTIFIER],
                after: [LOWER_CASE_IDENTIFIER],
                parent:
                    FUNCTION_DECLARATION_LEFT | LOWER_PATTERN | TYPE_ANNOTATION,
            } => {
                let old_name = Name {
                    name: diff
                        .old
                        .slice(&changes.old_removed[0].byte_range())
                        .into(),
                    kind: NameKind::Value,
                };
                let new_name = Name {
                    name: diff
                        .new
                        .slice(&changes.new_added[0].byte_range())
                        .into(),
                    kind: NameKind::Value,
                };
                let mut refactor = Refactor::new("changed name of value");
                refactors::changed_name::refactor(
                    &self.queries,
                    &mut self.dataflow_computation,
                    &mut refactor,
                    &diff.new,
                    &diff.old,
                    buffers,
                    buffers_by_path,
                    old_name,
                    new_name,
                    &changes.new_parent,
                )?;
                Ok(refactor)
            }
            Change {
                before: [TYPE_IDENTIFIER],
                after: [TYPE_IDENTIFIER],
                parent: TYPE_DECLARATION | TYPE_ALIAS_DECLARATION,
            } => {
                let old_name = Name {
                    name: diff
                        .old
                        .slice(&changes.old_removed[0].byte_range())
                        .into(),
                    kind: NameKind::Type,
                };
                let new_name = Name {
                    name: diff
                        .new
                        .slice(&changes.new_added[0].byte_range())
                        .into(),
                    kind: NameKind::Type,
                };
                let mut refactor = Refactor::new("changed name of type");
                refactors::changed_name::refactor(
                    &self.queries,
                    &mut self.dataflow_computation,
                    &mut refactor,
                    &diff.new,
                    &diff.old,
                    buffers,
                    buffers_by_path,
                    old_name,
                    new_name,
                    &changes.new_parent,
                )?;
                Ok(refactor)
            }
            Change {
                before: [CONSTRUCTOR_IDENTIFIER],
                after: [CONSTRUCTOR_IDENTIFIER],
                parent: UNION_VARIANT,
            } => {
                let old_name = Name {
                    name: diff
                        .old
                        .slice(&changes.old_removed[0].byte_range())
                        .into(),
                    kind: NameKind::Constructor,
                };
                let new_name = Name {
                    name: diff
                        .new
                        .slice(&changes.new_added[0].byte_range())
                        .into(),
                    kind: NameKind::Constructor,
                };
                let mut refactor = Refactor::new("changed name of constructor");
                refactors::changed_name::refactor(
                    &self.queries,
                    &mut self.dataflow_computation,
                    &mut refactor,
                    &diff.new,
                    &diff.old,
                    buffers,
                    buffers_by_path,
                    old_name,
                    new_name,
                    &changes.new_parent,
                )?;
                Ok(refactor)
            }
            _ => {
                let unimported_qualifiers = find_unimported_qualifiers(
                    &self.queries,
                    &diff.new,
                    changes.new_parent,
                )?;
                let mut refactor = Refactor::new(
                    "added qualified value from unimported module",
                );
                if !unimported_qualifiers.is_empty() {
                    refactors::typed_unimported_qualified_value::refactor(
                        &mut self.dataflow_computation,
                        &mut refactor,
                        &diff.new,
                        unimported_qualifiers,
                    )?;
                }
                Ok(refactor)
            }
        }
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
    queries: &Queries,
    code: &SourceFileSnapshot,
    parent: Node,
) -> Result<HashSet<ModuleName>, Error> {
    let mut cursor = QueryCursor::new();
    let existing_imports: Vec<Rope> = queries
        .query_for_imports
        .run(&mut cursor, code)
        .map(|import| import.aliased_name().into())
        .collect();
    let mut unimported_qualifiers = HashSet::new();
    for result in
        queries
            .query_for_qualified_values
            .run_in(&mut cursor, code, parent)
    {
        let (_, reference) = result?;
        if reference.unqualified_name.name.len_bytes() > 0
            && !existing_imports.contains(&reference.qualifier)
        {
            unimported_qualifiers
                .insert(ModuleName(reference.qualifier.to_string()));
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
    queries: &'a Queries,
    code: &'a SourceFileSnapshot,
    node: Node<'a>,
) -> Result<Import<'a>, Error> {
    let mut cursor = QueryCursor::new();
    queries
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
    println!("PARENT:");
    debug_print_node(&diff.old, 2, &changes.old_parent);
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
