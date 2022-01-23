use crate::analysis_thread::{SourceFileDiff, TreeChanges};
use crate::elm::compiler::Compiler;
use crate::elm::dependencies::DataflowComputation;
use crate::elm::io::ExportedName;
use crate::elm::queries::imports;
use crate::elm::queries::imports::{ExposedConstructors, ExposedName, Import};
use crate::elm::queries::qualified_values;
use crate::elm::queries::qualified_values::QualifiedName;
use crate::elm::queries::unqualified_values;
use crate::lib::log;
use crate::lib::log::Error;
use crate::lib::source_code::{Buffer, Edit, SourceFileSnapshot};
use core::ops::Range;
use ropey::{Rope, RopeSlice};
use std::collections::HashMap;
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
        // debug_print_tree_changes(diff, &changes);
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

fn _is_elm_file(path: &Path) -> bool {
    path.extension() == Some(std::ffi::OsStr::new("elm"))
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

fn remove_qualifier_from_references(
    engine: &Queries,
    computation: &mut DataflowComputation,
    refactor: &mut Refactor,
    code: &SourceFileSnapshot,
    qualifier: &RopeSlice,
    references: HashSet<Name>,
    // If we're removing qualifiers because the programmer started by removing
    // the qualifier from a single node, this is that node.
    // Our logic renaming a pre-existing variable of the same name should not
    // rename this node.
    node_stripped_of_qualifier: Option<Node>,
) -> Result<(), Error> {
    // Find existing unqualified references, so we can check whether removing
    // a qualifier from a qualified reference will introduce a naming conflict.
    let mut cursor = QueryCursor::new();
    let names_in_use: HashSet<Name> = engine
        .query_for_unqualified_values
        .run_in(&mut cursor, code, code.tree.root_node())
        .map(|r| r.map(|(_, reference)| reference))
        .collect::<Result<HashSet<Name>, Error>>()?;

    let mut names_from_other_modules: HashMap<Name, Rope> = HashMap::new();
    let imports = engine.query_for_imports.run(&mut cursor, code);
    for import in imports {
        if &import.aliased_name() == qualifier {
            continue;
        } else {
            for res in import.exposing_list() {
                let (_, exposed) = res?;
                let mut cursor = computation.exports_cursor(
                    code.buffer,
                    import.unaliased_name().to_string(),
                );
                exposed.for_each_name(cursor.iter(), |reference| {
                    names_from_other_modules
                        .insert(reference, import.aliased_name().into());
                });
            }
        }
    }

    for reference in names_in_use.intersection(&references) {
        // if another module is exposing a variable by this name, un-expose it
        if let Some(other_qualifier) = names_from_other_modules.get(reference) {
            qualify_value(
                engine,
                computation,
                refactor,
                code,
                node_stripped_of_qualifier,
                other_qualifier,
                reference,
                true,
            )?;
            continue;
        }

        // If an unqualified variable with this name already exists, rename it
        if names_in_use.contains(reference) {
            let new_name = names_with_digit(reference)
                .find(|name| !names_in_use.contains(name))
                .ok_or_else(|| {
                    log::mk_err!(
                        "names_with_digit unexpectedly ran out of names."
                    )
                })?;
            rename(
                engine,
                refactor,
                code,
                node_stripped_of_qualifier,
                reference,
                &new_name,
            )?;
        }
    }

    let qualified_references = engine.query_for_qualified_values.run_in(
        &mut cursor,
        code,
        code.tree.root_node(),
    );
    for reference_or_error in qualified_references {
        let (node, qualified) = reference_or_error?;
        if references.contains(&qualified.unqualified_name) {
            refactor.add_change(
                // The +1 makes it include the trailing dot between qualifier
                // and qualified value.
                node.start_byte()
                    ..(node.start_byte() + qualifier.len_bytes() + 1),
                String::new(),
            );
        }
    }
    Ok(())
}

struct NamesWithDigit<'a> {
    base_reference: &'a Name,
    next_digit: usize,
}

impl<'a> Iterator for NamesWithDigit<'a> {
    type Item = Name;

    fn next(&mut self) -> Option<Self::Item> {
        let mut new_name = self.base_reference.name.clone();
        new_name.append(Rope::from_str(self.next_digit.to_string().as_str()));
        let next_ref = Name {
            name: new_name,
            kind: self.base_reference.kind,
        };
        self.next_digit += 1;
        Some(next_ref)
    }
}

fn names_with_digit(reference: &Name) -> NamesWithDigit {
    NamesWithDigit {
        base_reference: reference,
        next_digit: 2,
    }
}

#[cfg(test)]
mod names_with_digit {
    use super::*;

    #[test]
    fn iterator_returns_values_with_increasing_trailing_digit() {
        let base_reference = Name {
            name: Rope::from_str("hi"),
            kind: NameKind::Value,
        };
        let first_tree: Vec<Rope> = names_with_digit(&base_reference)
            .map(|reference| reference.name)
            .take(3)
            .collect();
        assert_eq!(first_tree, vec!["hi2", "hi3", "hi4"]);
    }
}

fn rename(
    engine: &Queries,
    refactor: &mut Refactor,
    code: &SourceFileSnapshot,
    node_stripped_of_qualifier: Option<Node>,
    from: &Name,
    to: &Name,
) -> Result<(), Error> {
    let mut cursor = QueryCursor::new();
    let unqualified_values = engine.query_for_unqualified_values.run_in(
        &mut cursor,
        code,
        code.tree.root_node(),
    );
    for res in unqualified_values {
        let (node, reference) = res?;
        if &reference == from
            && Some(node.id()) != node_stripped_of_qualifier.map(|n| n.id())
        {
            refactor.add_change(node.byte_range(), to.name.to_string())
        }
    }
    Ok(())
}

// Add a name to the list of values exposed from a particular module.
fn add_to_exposing_list(
    import: &Import,
    reference: &Name,
    ctor_type: Option<&String>,
    refactor: &mut Refactor,
) -> Result<(), Error> {
    let (target_exposed_name, insert_str) = match ctor_type {
        Some(type_name) => (type_name.to_owned(), format!("{}(..)", type_name)),
        None => (reference.name.to_string(), reference.name.to_string()),
    };

    let mut last_node = None;

    // Find the first node in the existing exposing list alphabetically
    // coming after the node we're looking to insert, then insert in
    // front of that node.
    for result in import.exposing_list() {
        let (node, exposed) = result?;
        let exposed_name = match exposed {
            ExposedName::Operator(op) => op.name,
            ExposedName::Value(val) => val.name,
            ExposedName::Type(type_) => type_.name,
            ExposedName::All => {
                return Ok(());
            }
        };
        last_node = Some(node);
        // Insert right before this item to maintain alphabetic order.
        // If the exposing list wasn't ordered alphabetically the insert
        // place might appear random.
        match std::cmp::Ord::cmp(
            &target_exposed_name,
            &exposed_name.to_string(),
        ) {
            std::cmp::Ordering::Equal => {
                if ctor_type.is_some() {
                    // node.child(1) is the node corresponding to the exposed
                    // contructors: `(..)`.
                    if node.child(1).is_none() {
                        let insert_at = node.end_byte();
                        refactor.add_change(
                            insert_at..insert_at,
                            "(..)".to_string(),
                        );
                    }
                };
                return Ok(());
            }
            std::cmp::Ordering::Less => {
                let insert_at = node.start_byte();
                refactor.add_change(
                    insert_at..insert_at,
                    format!("{}, ", insert_str),
                );
                return Ok(());
            }
            std::cmp::Ordering::Greater => {}
        }
    }

    // We didn't find anything in the exposing list alphabetically
    // after us. Either we come alphabetically after all currently
    // exposed elements, or there is no exposing list at all.
    match last_node {
        None => {
            refactor.add_change(
                import.root_node.end_byte()..import.root_node.end_byte(),
                format!(" exposing ({})", insert_str),
            );
        }
        Some(node) => {
            let insert_at = node.end_byte();
            refactor
                .add_change(insert_at..insert_at, format!(", {}", insert_str));
        }
    }
    Ok(())
}

fn qualify_value(
    engine: &Queries,
    computation: &mut DataflowComputation,
    refactor: &mut Refactor,
    code: &SourceFileSnapshot,
    node_to_skip: Option<Node>,
    qualifier: &Rope,
    reference: &Name,
    // If the qualified value is coming from an import that exposing everything,
    // then this boolean decides whether to keep the `exposing (..)` clause as
    // is, or whether to replace it with an explicit list of currently used
    // values minus the now qualified value.
    remove_expose_all_if_necessary: bool,
) -> Result<(), Error> {
    let import =
        get_import_by_aliased_name(engine, code, &qualifier.slice(..))?;

    let exposing_list_length = import.exposing_list().count();
    let mut references_to_qualify = HashSet::new();
    for result in import.exposing_list() {
        let (node, exposed) = result?;
        match &exposed {
            ExposedName::Operator(op) => {
                if op.name == reference.name
                    && reference.kind == NameKind::Operator
                {
                    return Err(log::mk_err!(
                        "cannot qualify operator, Elm doesn't allow it!"
                    ));
                }
            }
            ExposedName::Type(type_) => {
                if type_.name == reference.name
                    && reference.kind == NameKind::Type
                {
                    if exposing_list_length == 1 {
                        remove_exposing_list(refactor, &import);
                    } else {
                        remove_from_exposing_list(refactor, &node)?;
                    }
                    references_to_qualify.insert(Name {
                        name: type_.name.into(),
                        kind: NameKind::Type,
                    });
                }

                let mut cursor = computation.exports_cursor(
                    code.buffer,
                    import.unaliased_name().to_string(),
                );
                match constructors_of_exports(cursor.iter(), type_.name)? {
                    ExposedConstructors::FromTypeAlias(ctor) => {
                        if ctor == &reference.name {
                            // Ensure we don't remove the item from the exposing
                            // list twice (see code above).
                            // TODO: Clean this up.
                            if reference.kind != NameKind::Type {
                                if exposing_list_length == 1 {
                                    remove_exposing_list(refactor, &import);
                                } else {
                                    remove_from_exposing_list(refactor, &node)?;
                                }
                            }
                            references_to_qualify.insert(Name {
                                name: Rope::from_str(ctor),
                                kind: NameKind::Type,
                            });
                            references_to_qualify.insert(Name {
                                name: Rope::from_str(ctor),
                                kind: NameKind::Constructor,
                            });
                        }
                    }
                    ExposedConstructors::FromCustomType(ctors) => {
                        if ctors.iter().any(|ctor| *ctor == reference.name) {
                            // Remove `(..)` behind type from constructor this.
                            let exposing_ctors_node = node.child(1).ok_or_else(|| {
                                log::mk_err!("could not find `(..)` node behind exposed type")
                            })?;
                            refactor.add_change(
                                exposing_ctors_node.byte_range(),
                                String::new(),
                            );

                            // We're qualifying a constructor. In Elm you can only
                            // expose either all constructors of a type or none of them,
                            // so if the programmer qualifies one constructor assume
                            // intend to do them all.
                            let constructor_references =
                                ctors.iter().map(|ctor| Name {
                                    name: Rope::from_str(ctor),
                                    kind: NameKind::Constructor,
                                });
                            references_to_qualify
                                .extend(constructor_references);
                        }
                    }
                }
            }
            ExposedName::Value(val) => {
                if val.name == reference.name
                    && reference.kind == NameKind::Value
                {
                    if exposing_list_length == 1 {
                        remove_exposing_list(refactor, &import);
                    } else {
                        remove_from_exposing_list(refactor, &node)?;
                    }
                    references_to_qualify.insert(Name {
                        name: val.name.into(),
                        kind: NameKind::Value,
                    });
                    break;
                }
            }
            ExposedName::All => {
                if remove_expose_all_if_necessary {
                    let mut exposed_names: HashMap<Name, &ExportedName> =
                        HashMap::new();
                    let mut cursor = computation.exports_cursor(
                        code.buffer,
                        import.unaliased_name().to_string(),
                    );
                    cursor.iter().for_each(|export| match export {
                        ExportedName::Value { name } => {
                            exposed_names.insert(
                                Name {
                                    name: Rope::from_str(name),
                                    kind: NameKind::Value,
                                },
                                export,
                            );
                        }
                        ExportedName::RecordTypeAlias { name } => {
                            exposed_names.insert(
                                Name {
                                    name: Rope::from_str(name),
                                    kind: NameKind::Type,
                                },
                                export,
                            );
                            exposed_names.insert(
                                Name {
                                    name: Rope::from_str(name),
                                    kind: NameKind::Constructor,
                                },
                                export,
                            );
                        }
                        ExportedName::Type { name, constructors } => {
                            exposed_names.insert(
                                Name {
                                    name: Rope::from_str(name),
                                    kind: NameKind::Type,
                                },
                                export,
                            );
                            for ctor in constructors {
                                exposed_names.insert(
                                    Name {
                                        name: Rope::from_str(ctor),
                                        kind: NameKind::Constructor,
                                    },
                                    export,
                                );
                            }
                        }
                    });
                    let mut cursor = QueryCursor::new();
                    let mut unqualified_names_in_use: HashSet<Name> = engine
                        .query_for_unqualified_values
                        .run(&mut cursor, code)
                        .map(|r| r.map(|(_, reference)| reference))
                        .collect::<Result<HashSet<Name>, Error>>()?;
                    unqualified_names_in_use.remove(reference);
                    let mut new_exposed: String = String::new();
                    exposed_names.into_iter().for_each(
                        |(reference, export)| {
                            if unqualified_names_in_use.contains(&reference) {
                                if !new_exposed.is_empty() {
                                    new_exposed.push_str(", ")
                                }
                                match export {
                                    ExportedName::Value { name } => {
                                        new_exposed.push_str(name);
                                    }
                                    ExportedName::RecordTypeAlias { name } => {
                                        new_exposed.push_str(name);
                                    }
                                    ExportedName::Type { name, .. } => {
                                        if reference.kind
                                            == NameKind::Constructor
                                        {
                                            new_exposed.push_str(&format!(
                                                "{}(..)",
                                                name
                                            ));
                                        } else {
                                            new_exposed.push_str(name);
                                        }
                                    }
                                }
                            }
                        },
                    );
                    refactor.add_change(node.byte_range(), new_exposed);
                }

                match reference.kind {
                    NameKind::Operator => {
                        return Err(log::mk_err!(
                            "cannot qualify operator, Elm doesn't allow it!"
                        ));
                    }
                    NameKind::Value | NameKind::Type => {
                        references_to_qualify.insert(reference.clone());
                    }
                    NameKind::Constructor => {
                        // We know a constructor got qualified, but not which
                        // type it belongs too. To find it, we iterate over all
                        // the exports from the module matching the qualifier we
                        // added. The type must be among them!
                        let mut cursor = computation.exports_cursor(
                            code.buffer,
                            import.unaliased_name().to_string(),
                        );
                        for export in cursor.iter() {
                            match export {
                                ExportedName::Value { .. } => {}
                                ExportedName::RecordTypeAlias { .. } => {}
                                ExportedName::Type { constructors, .. } => {
                                    if constructors
                                        .iter()
                                        .any(|ctor| *ctor == reference.name)
                                    {
                                        let constructor_references =
                                            constructors.iter().map(|ctor| {
                                                Name {
                                                    name: Rope::from_str(ctor),
                                                    kind: NameKind::Constructor,
                                                }
                                            });
                                        references_to_qualify
                                            .extend(constructor_references);
                                    }
                                }
                            }
                        }
                        break;
                    }
                }
            }
        }
    }
    add_qualifier_to_references(
        engine,
        refactor,
        &mut QueryCursor::new(),
        code,
        node_to_skip,
        &import,
        references_to_qualify,
    )?;
    Ok(())
}

fn remove_exposing_list(refactor: &mut Refactor, import: &Import) {
    match import.exposing_list_node {
        None => {}
        Some(node) => refactor.add_change(node.byte_range(), String::new()),
    };
}

fn get_import_by_aliased_name<'a>(
    engine: &Queries,
    code: &'a SourceFileSnapshot,
    qualifier: &RopeSlice,
) -> Result<Import<'a>, Error> {
    let mut cursor = QueryCursor::new();
    engine
        .query_for_imports
        .run(&mut cursor, code)
        .find(|import| import.aliased_name() == *qualifier)
        .ok_or_else(|| {
            log::mk_err!(
                "could not find an import with the requested aliased name"
            )
        })
}

fn remove_from_exposing_list(
    refactor: &mut Refactor,
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
    refactor
        .add_change(range_including_comma_and_whitespace(node), String::new());
    Ok(())
}

fn add_qualifier_to_references(
    engine: &Queries,
    refactor: &mut Refactor,
    cursor: &mut QueryCursor,
    code: &SourceFileSnapshot,
    node_to_skip: Option<Node>,
    import: &Import,
    references: HashSet<Name>,
) -> Result<(), Error> {
    let results = engine.query_for_unqualified_values.run(cursor, code);
    let should_skip = |node: Node| {
        if let Some(node_to_skip2) = node_to_skip {
            node.id() == node_to_skip2.id()
        } else {
            false
        }
    };
    for result in results {
        let (node, reference) = result?;
        if references.contains(&reference) && !should_skip(node) {
            refactor.add_change(
                node.start_byte()..node.start_byte(),
                format!("{}.", import.aliased_name()),
            );
        }
    }
    Ok(())
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

fn constructors_of_exports<'a, I>(
    exported_names: I,
    type_name: RopeSlice<'a>,
) -> Result<ExposedConstructors<'a>, Error>
where
    I: Iterator<Item = &'a ExportedName>,
{
    for export in exported_names {
        match export {
            ExportedName::Value { .. } => {}
            ExportedName::RecordTypeAlias { name } => {
                return Ok(ExposedConstructors::FromTypeAlias(name));
            }
            ExportedName::Type { name, constructors } => {
                if type_name.eq(name) {
                    return Ok(ExposedConstructors::FromCustomType(
                        constructors,
                    ));
                }
            }
        }
    }
    Err(log::mk_err!("did not find type in module"))
}

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
