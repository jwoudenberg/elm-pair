use crate::analysis_thread::{SourceFileDiff, TreeChanges};
use crate::languages::elm::dependencies::{
    load_dependencies, ElmExport, ElmModule, ExportsQuery, ProjectInfo,
};
use crate::support::source_code::{Buffer, Edit, SourceFileSnapshot};
use crate::Error;
use ropey::{Rope, RopeSlice};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tree_sitter::{Language, Node, Query, QueryCursor, TreeCursor};

pub mod dependencies;
pub mod idat;

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
const MODULE_NAME_SEGMENT: u16 = 201;
const TYPE_IDENTIFIER: u16 = 33;
const CONSTRUCTOR_IDENTIFIER: u16 = 8;

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
        check(super::TYPE_IDENTIFIER, "type_identifier", true);
        check(
            super::CONSTRUCTOR_IDENTIFIER,
            "constructor_identifier",
            true,
        );
    }
}

pub(crate) struct RefactorEngine {
    buffers: HashMap<Buffer, BufferInfo>,
    projects: HashMap<PathBuf, ProjectInfo>,
    query_for_imports: ImportsQuery,
    query_for_unqualified_values: UnqualifiedValuesQuery,
    query_for_qualified_values: QualifiedValuesQuery,
    query_for_exports: ExportsQuery,
}

pub struct BufferInfo {
    pub project_root: PathBuf,
    pub path: PathBuf,
}

impl RefactorEngine {
    pub(crate) fn new() -> Result<RefactorEngine, Error> {
        let language = tree_sitter_elm::language();
        let engine = RefactorEngine {
            buffers: HashMap::new(),
            projects: HashMap::new(),
            query_for_imports: ImportsQuery::init(language)?,
            query_for_unqualified_values: UnqualifiedValuesQuery::init(
                language,
            )?,
            query_for_qualified_values: QualifiedValuesQuery::init(language)?,
            query_for_exports: ExportsQuery::init(language)?,
        };

        Ok(engine)
    }

    // TODO: try to return an Iterator instead of a Vector.
    // TODO: Try remove Vector from TreeChanges type.
    pub(crate) fn respond_to_change<'a>(
        &self,
        diff: &SourceFileDiff,
        changes: TreeChanges<'a>,
    ) -> Result<Option<Vec<Edit>>, Error> {
        // debug_print_tree_changes(diff, &changes);
        let before = attach_kinds(&changes.old_removed);
        let after = attach_kinds(&changes.new_added);
        let edits = match (before.as_slice(), after.as_slice()) {
            (
                [EXPOSED_VALUE | EXPOSED_TYPE, ..]
                | [COMMA, EXPOSED_VALUE | EXPOSED_TYPE, ..]
                | [DOUBLE_DOT]
                | [],
                [EXPOSED_VALUE | EXPOSED_TYPE, ..]
                | [COMMA, EXPOSED_VALUE | EXPOSED_TYPE, ..]
                | [DOUBLE_DOT]
                | [],
            ) if !(before.is_empty() && after.is_empty()) => {
                on_changed_values_in_exposing_list(
                    self,
                    diff,
                    changes.old_parent,
                    changes.new_parent,
                )?
            }
            (
                [TYPE_IDENTIFIER],
                [MODULE_NAME_SEGMENT, DOT, .., TYPE_IDENTIFIER],
            )
            | (
                [CONSTRUCTOR_IDENTIFIER],
                [MODULE_NAME_SEGMENT, DOT, .., CONSTRUCTOR_IDENTIFIER],
            )
            | (
                [LOWER_CASE_IDENTIFIER],
                [MODULE_NAME_SEGMENT, DOT, .., LOWER_CASE_IDENTIFIER],
            ) => on_added_module_qualifier_to_value(self, diff, changes)?,
            (
                [MODULE_NAME_SEGMENT, DOT, .., TYPE_IDENTIFIER],
                [TYPE_IDENTIFIER],
            )
            | (
                [MODULE_NAME_SEGMENT, DOT, .., CONSTRUCTOR_IDENTIFIER],
                [CONSTRUCTOR_IDENTIFIER],
            )
            | (
                [MODULE_NAME_SEGMENT, DOT, .., LOWER_CASE_IDENTIFIER],
                [LOWER_CASE_IDENTIFIER],
            ) => on_removed_module_qualifier_from_value(self, diff, changes)?,
            ([], [EXPOSING_LIST]) => {
                on_added_exposing_list_to_import(self, diff, changes)?
            }
            ([EXPOSING_LIST], []) => {
                on_removed_exposing_list_from_import(self, diff, changes)?
            }
            ([], [EXPOSED_UNION_CONSTRUCTORS]) => {
                on_added_constructors_to_exposing_list(self, diff, changes)?
            }
            ([EXPOSED_UNION_CONSTRUCTORS], []) => {
                on_removed_constructors_from_exposing_list(self, diff, changes)?
            }
            _ => Vec::new(),
        };
        if edits.is_empty() {
            Ok(None)
        } else {
            Ok(Some(sort_edits(edits)))
        }
    }

    pub(crate) fn constructors_for_type<'a, 'b>(
        &'a self,
        buffer: Buffer,
        module_name: RopeSlice<'b>,
        type_name: RopeSlice<'b>,
    ) -> Result<&'a Vec<String>, Error> {
        self.module_exports(buffer, module_name)?
            .iter()
            .find_map(|export| match export {
                ElmExport::Value { .. } => None,
                ElmExport::Type { name, constructors } => {
                    if type_name.eq(name) {
                        Some(constructors)
                    } else {
                        None
                    }
                }
            })
            .ok_or_else(|| Error::ElmNoSuchTypeInModule {
                module_name: module_name.to_string(),
                type_name: type_name.to_string(),
            })
    }

    pub(crate) fn module_exports(
        &self,
        buffer: Buffer,
        module: RopeSlice,
    ) -> Result<&Vec<ElmExport>, Error> {
        let project = self.buffer_project(buffer)?;
        match project.modules.get(&module.to_string()) {
            None => panic!("no such module"),
            Some(ElmModule { exports }) => Ok(exports),
        }
    }

    fn buffer_project(&self, buffer: Buffer) -> Result<&ProjectInfo, Error> {
        let buffer_info = self
            .buffers
            .get(&buffer)
            .ok_or(Error::ElmNoProjectStoredForBuffer(buffer))?;
        let project_info =
            self.projects.get(&buffer_info.project_root).unwrap();
        Ok(project_info)
    }

    pub(crate) fn init_buffer(
        &mut self,
        buffer: Buffer,
        path: PathBuf,
    ) -> Result<(), Error> {
        let project_root = project_root_for_path(&path)?.to_owned();
        self.init_project(&project_root)?;
        let buffer_info = BufferInfo { path, project_root };
        self.buffers.insert(buffer, buffer_info);
        Ok(())
    }

    fn init_project(&mut self, project_root: &Path) -> Result<(), Error> {
        if self.projects.contains_key(project_root) {
            return Ok(());
        }
        let project_info =
            load_dependencies(&self.query_for_exports, project_root)?;
        self.projects.insert(project_root.to_owned(), project_info);
        Ok(())
    }
}

fn on_added_constructors_to_exposing_list(
    engine: &RefactorEngine,
    diff: &SourceFileDiff,
    changes: TreeChanges,
) -> Result<Vec<Edit>, Error> {
    // TODO: remove unwrap()'s, clone()'s, and otherwise clean up.
    let node = changes.new_added.first().unwrap();
    let exposed_type_node = node.parent().unwrap();
    let type_name = diff
        .new
        .slice(&exposed_type_node.child(0).unwrap().byte_range());
    let import_node = exposed_type_node.parent().unwrap().parent().unwrap();
    let mut cursor = QueryCursor::new();
    let import = engine
        .query_for_imports
        .run_in(&mut cursor, &diff.new, import_node)
        .next()
        .ok_or(Error::TreeSitterExpectedNodeDoesNotExist)?;
    let mut edits = Vec::new();
    let project_info = engine.buffer_project(diff.new.buffer).unwrap();
    for (_, exposed) in import.exposing_list() {
        if let Exposed::Type(type_) = &exposed {
            if type_.name == type_name {
                remove_qualifier_for_exposed(
                    engine,
                    &diff.new,
                    project_info,
                    &import,
                    &exposed,
                    &mut edits,
                );
                break;
            }
        }
    }
    Ok(edits)
}

fn on_removed_constructors_from_exposing_list(
    engine: &RefactorEngine,
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
    let old_import = engine
        .query_for_imports
        .run_in(&mut cursor, &diff.old, old_import_node)
        .next()
        .ok_or(Error::TreeSitterExpectedNodeDoesNotExist)?;
    let mut edits = Vec::new();
    let mut cursor2 = QueryCursor::new();
    for (_, exposed) in old_import.exposing_list() {
        if let Exposed::Type(type_) = exposed {
            if type_.name == type_name {
                add_qualifier_to_constructors(
                    engine,
                    &mut edits,
                    &mut cursor2,
                    &diff.new,
                    &old_import,
                    type_.constructors(engine)?,
                );
                break;
            }
        }
    }
    Ok(edits)
}

fn remove_qualifier_for_exposed(
    engine: &RefactorEngine,
    code: &SourceFileSnapshot,
    project_info: &ProjectInfo,
    import: &Import,
    exposed: &Exposed,
    edits: &mut Vec<Edit>,
) {
    match exposed {
        Exposed::Value(val) => remove_qualifier_from_name(
            engine,
            code,
            edits,
            &QualifiedReference {
                qualifier: import.aliased_name(),
                reference: Reference {
                    kind: ReferenceKind::Value,
                    name: val.name,
                },
            },
        ),
        Exposed::Type(type_) => {
            if type_.exposing_constructors {
                project_info
                    .modules
                    .get(&import.name().to_string())
                    .unwrap()
                    .exports
                    .iter()
                    .for_each(|export| match export {
                        ElmExport::Value { .. } => {}
                        ElmExport::Type { name, constructors } => {
                            if name == &type_.name {
                                for ctor in constructors.iter() {
                                    remove_qualifier_from_name(
                                        engine,
                                        code,
                                        edits,
                                        &QualifiedReference {
                                            qualifier: import.name(),
                                            reference: Reference {
                                                kind:
                                                    ReferenceKind::Constructor,
                                                name: Rope::from_str(ctor)
                                                    .slice(..),
                                            },
                                        },
                                    );
                                }
                            }
                        }
                    });
            }
            remove_qualifier_from_name(
                engine,
                code,
                edits,
                &QualifiedReference {
                    qualifier: import.aliased_name(),
                    reference: Reference {
                        kind: ReferenceKind::Type,
                        name: type_.name,
                    },
                },
            )
        }
        Exposed::All(_) => {
            project_info
                .modules
                .get(&import.name().to_string())
                .unwrap()
                .exports
                .iter()
                .for_each(|export| match export {
                    ElmExport::Value { name } => {
                        remove_qualifier_from_name(
                            engine,
                            code,
                            edits,
                            &QualifiedReference {
                                qualifier: import.name(),
                                reference: Reference {
                                    kind: ReferenceKind::Value,
                                    name: Rope::from_str(name).slice(..),
                                },
                            },
                        );
                    }
                    ElmExport::Type { name, constructors } => {
                        remove_qualifier_from_name(
                            engine,
                            code,
                            edits,
                            &QualifiedReference {
                                qualifier: import.name(),
                                reference: Reference {
                                    kind: ReferenceKind::Type,
                                    name: Rope::from_str(name).slice(..),
                                },
                            },
                        );
                        for ctor in constructors.iter() {
                            remove_qualifier_from_name(
                                engine,
                                code,
                                edits,
                                &QualifiedReference {
                                    qualifier: import.name(),
                                    reference: Reference {
                                        kind: ReferenceKind::Constructor,
                                        name: Rope::from_str(ctor).slice(..),
                                    },
                                },
                            );
                        }
                    }
                });
        }
        // Operators cannot be qualified, so if we add one to an
        // exposed list there's nothing to _unqualify_.
        Exposed::Operator(_op) => {}
    }
}

fn on_changed_values_in_exposing_list(
    engine: &RefactorEngine,
    diff: &SourceFileDiff,
    old_parent: Node,
    new_parent: Node,
) -> Result<Vec<Edit>, Error> {
    println!("HIHI");

    // TODO: Figure out better approach to tree-traversal.
    let old_import_node = old_parent
        .parent()
        .ok_or(Error::TreeSitterExpectedNodeDoesNotExist)?;
    let mut cursor = QueryCursor::new();
    let old_import = engine
        .query_for_imports
        .run_in(&mut cursor, &diff.old, old_import_node)
        .next()
        .ok_or(Error::TreeSitterExpectedNodeDoesNotExist)?;

    let new_import_node = new_parent
        .parent()
        .ok_or(Error::TreeSitterExpectedNodeDoesNotExist)?;
    let mut cursor2 = QueryCursor::new();
    let new_import = engine
        .query_for_imports
        .run_in(&mut cursor2, &diff.new, new_import_node)
        .next()
        .ok_or(Error::TreeSitterExpectedNodeDoesNotExist)?;

    let mut edits = Vec::new();

    let mut cursor3 = QueryCursor::new();
    let new_exposed = new_import
        .exposing_list()
        .map(|(_, exposed)| exposed)
        .collect::<Vec<Exposed>>();
    old_import.exposing_list().for_each(|(_, exposed)| {
        if !new_exposed.contains(&exposed) {
            add_qualifier_to_name(
                engine,
                &mut edits,
                &mut cursor3,
                &diff.new,
                &new_import,
                &exposed,
            )
        }
    });

    let project_info = engine.buffer_project(diff.new.buffer).unwrap();
    let old_exposed = old_import
        .exposing_list()
        .map(|(_, exposed)| exposed)
        .collect::<Vec<Exposed>>();
    new_import.exposing_list().for_each(|(_, exposed)| {
        if !old_exposed.contains(&exposed) {
            remove_qualifier_for_exposed(
                engine,
                &diff.new,
                project_info,
                &new_import,
                &exposed,
                &mut edits,
            )
        }
    });
    Ok(edits)
}

fn on_removed_module_qualifier_from_value(
    engine: &RefactorEngine,
    diff: &SourceFileDiff,
    changes: TreeChanges,
) -> Result<Vec<Edit>, Error> {
    let name_now = diff.new.slice(
        &changes
            .new_added
            .first()
            .ok_or(Error::TreeSitterExpectedNodeDoesNotExist)?
            .byte_range(),
    );
    let parent = changes
        .old_removed
        .first()
        .ok_or(Error::TreeSitterExpectedNodeDoesNotExist)?
        .parent()
        .ok_or(Error::TreeSitterExpectedNodeDoesNotExist)?;
    let mut cursor = QueryCursor::new();
    let (_, reference) = engine
        .query_for_qualified_values
        .run_in(&mut cursor, &diff.old, parent)
        .next()
        .ok_or(Error::TreeSitterQueryReturnedNotEnoughMatches)?;
    if name_now != reference.reference.name {
        return Ok(Vec::new());
    }
    let mut edits = Vec::new();
    let mut cursor2 = QueryCursor::new();
    let import = get_import_by_aliased_name(
        &engine.query_for_imports,
        &mut cursor2,
        &diff.new,
        &reference.qualifier,
    )?;
    if reference.reference.kind == ReferenceKind::Constructor {
        let project_info = engine.buffer_project(diff.new.buffer).unwrap();
        project_info
            .modules
            .get(&import.name().to_string())
            .unwrap()
            .exports
            .iter()
            .for_each(|export| match export {
                ElmExport::Value { .. } => {}
                ElmExport::Type { name, constructors } => {
                    if constructors
                        .contains(&reference.reference.name.to_string())
                    {
                        for ctor in constructors.iter() {
                            remove_qualifier_from_name(
                                engine,
                                &diff.new,
                                &mut edits,
                                &QualifiedReference {
                                    qualifier: reference.qualifier,
                                    reference: Reference {
                                        kind: ReferenceKind::Constructor,
                                        name: Rope::from_str(ctor).slice(..),
                                    },
                                },
                            );
                        }
                        add_to_exposing_list(
                            &import,
                            &reference.reference,
                            Some(name),
                            &diff.new,
                            &mut edits,
                        );
                    }
                }
            });
    } else {
        remove_qualifier_from_name(engine, &diff.new, &mut edits, &reference);
        add_to_exposing_list(
            &import,
            &reference.reference,
            None,
            &diff.new,
            &mut edits,
        );
    };
    Ok(edits)
}

fn remove_qualifier_from_name(
    engine: &RefactorEngine,
    code: &SourceFileSnapshot,
    edits: &mut Vec<Edit>,
    reference: &QualifiedReference,
) {
    let mut cursor = QueryCursor::new();
    for (node, qualified) in engine.query_for_qualified_values.run_in(
        &mut cursor,
        code,
        code.tree.root_node(),
    ) {
        if &qualified == reference {
            edits.push(Edit::new(
                code.buffer,
                &mut code.bytes.clone(),
                // The +1 makes it include the trailing dot between qualifier
                // and qualified value.
                &(node.start_byte()
                    ..(node.start_byte()
                        + reference.qualifier.len_bytes()
                        + 1)),
                String::new(),
            ));
        }
    }
}

// Add a name to the list of values exposed from a particular module.
fn add_to_exposing_list(
    import: &Import,
    reference: &Reference,
    ctor_type: Option<&String>,
    code: &SourceFileSnapshot,
    edits: &mut Vec<Edit>,
) {
    let (target_exposed_name, insert_str) = match ctor_type {
        Some(type_name) => (type_name.to_owned(), format!("{}(..)", type_name)),
        None => (reference.name.to_string(), reference.name.to_string()),
    };

    let mut last_node = None;

    // Find the first node in the existing exposing list alphabetically
    // coming after the node we're looking to insert, then insert in
    // front of that node.
    for (node, exposed) in import.exposing_list() {
        let exposed_name = match exposed {
            Exposed::Operator(op) => op.name,
            Exposed::Value(val) => val.name,
            Exposed::Type(type_) => type_.name,
            Exposed::All(_) => {
                return;
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
                return if ctor_type.is_some() {
                    // node.child(1) is the node corresponding to the exposed
                    // contructors: `(..)`.
                    if node.child(1).is_none() {
                        let insert_at = node.end_byte();
                        edits.push(Edit::new(
                            code.buffer,
                            &mut code.bytes.clone(),
                            &(insert_at..insert_at),
                            "(..)".to_string(),
                        ));
                    }
                };
            }
            std::cmp::Ordering::Less => {
                let insert_at = node.start_byte();
                return edits.push(Edit::new(
                    code.buffer,
                    &mut code.bytes.clone(),
                    &(insert_at..insert_at),
                    format!("{}, ", insert_str),
                ));
            }
            std::cmp::Ordering::Greater => {}
        }
    }

    // We didn't find anything in the exposing list alphabetically
    // after us. Either we come alphabetically after all currently
    // exposed elements, or there is no exposing list at all.
    match last_node {
        None => {
            edits.push(Edit::new(
                code.buffer,
                &mut code.bytes.clone(),
                &(import.root_node.end_byte()..import.root_node.end_byte()),
                format!(" exposing ({})", insert_str),
            ));
        }
        Some(node) => {
            let insert_at = node.end_byte();
            edits.push(Edit::new(
                code.buffer,
                &mut code.bytes.clone(),
                &(insert_at..insert_at),
                format!(", {}", insert_str),
            ));
        }
    }
}

fn on_added_module_qualifier_to_value(
    engine: &RefactorEngine,
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
    let (
        _,
        QualifiedReference {
            qualifier,
            reference: Reference { kind, name, .. },
            ..
        },
    ) = engine
        .query_for_qualified_values
        .run_in(&mut cursor, &diff.new, parent)
        .next()
        .ok_or(Error::TreeSitterQueryReturnedNotEnoughMatches)?;
    if name_before != name {
        return Ok(Vec::new());
    }
    let mut edits = Vec::new();
    let mut cursor2 = QueryCursor::new();
    let import = get_import_by_aliased_name(
        &engine.query_for_imports,
        &mut cursor2,
        &diff.new,
        &qualifier,
    )?;

    let exposing_list_length = import.exposing_list().count();
    for (node, exposed) in import.exposing_list() {
        match &exposed {
            Exposed::Operator(op) => {
                if op.name == name && kind == ReferenceKind::Operator {
                    return Err(Error::ElmCannotQualifyOperator(
                        op.name.to_string(),
                    ));
                }
            }
            Exposed::Type(type_) => {
                if type_.name == name && kind == ReferenceKind::Type {
                    if exposing_list_length == 1 {
                        remove_exposing_list(&mut edits, &diff.new, &import);
                    } else {
                        remove_from_exposing_list(&mut edits, diff, &node)?;
                    }
                    let mut cursor2 = QueryCursor::new();
                    add_qualifier_to_type(
                        engine,
                        &mut edits,
                        &mut cursor2,
                        &diff.new,
                        &import,
                        type_,
                    );
                    break;
                }

                let constructors = type_.constructors(engine)?;
                if constructors.clone().any(|ctor| *ctor == name)
                    && kind == ReferenceKind::Constructor
                {
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
                        engine,
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
                if val.name == name && kind == ReferenceKind::Value {
                    if exposing_list_length == 1 {
                        remove_exposing_list(&mut edits, &diff.new, &import);
                    } else {
                        remove_from_exposing_list(&mut edits, diff, &node)?;
                    }
                    let mut cursor2 = QueryCursor::new();
                    add_qualifier_to_value(
                        engine,
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
                let mut cursor2 = QueryCursor::new();
                match kind {
                    ReferenceKind::Operator => {
                        return Err(Error::ElmCannotQualifyOperator(
                            name.to_string(),
                        ))
                    }
                    ReferenceKind::Value => add_qualifier_to_value(
                        engine,
                        &mut edits,
                        &mut cursor2,
                        &diff.new,
                        &import,
                        &ExposedValue { name },
                    ),
                    ReferenceKind::Type => add_qualifier_to_type(
                        engine,
                        &mut edits,
                        &mut cursor2,
                        &diff.new,
                        &import,
                        &ExposedType {
                            buffer: diff.new.buffer,
                            exposing_constructors: false,
                            module_name: qualifier,
                            name,
                        },
                    ),
                    ReferenceKind::Constructor => {
                        // We know a constructor got qualified, but not which
                        // type it belogns too. To find it, we iterate over all
                        // the exports from the module matching the qualifier we
                        // added. The type must be among them!
                        let exports = match engine
                            .module_exports(diff.new.buffer, import.name())
                        {
                            Ok(exports_) => exports_,
                            Err(err) => {
                                eprintln!(
                                    "[error] failed to read exports of {}: {:?}",
                                    import.name().to_string(),
                                    err
                                );
                                break;
                            }
                        };
                        for export in exports {
                            match export {
                                ElmExport::Value { .. } => {}
                                ElmExport::Type { constructors, .. } => {
                                    if constructors
                                        .iter()
                                        .any(|ctor| *ctor == name)
                                    {
                                        add_qualifier_to_constructors(
                                            engine,
                                            &mut edits,
                                            &mut cursor2,
                                            &diff.new,
                                            &import,
                                            ExposedTypeConstructors::All {
                                                names: constructors,
                                            },
                                        );
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
    Ok(edits)
}

fn on_added_exposing_list_to_import(
    engine: &RefactorEngine,
    diff: &SourceFileDiff,
    changes: TreeChanges,
) -> Result<Vec<Edit>, Error> {
    let import_node = changes
        .new_added
        .first()
        .and_then(Node::parent)
        .ok_or(Error::TreeSitterExpectedNodeDoesNotExist)?;
    let mut cursor = QueryCursor::new();
    let import = engine
        .query_for_imports
        .run_in(&mut cursor, &diff.new, import_node)
        .next()
        .ok_or(Error::TreeSitterExpectedNodeDoesNotExist)?;
    let mut edits = Vec::new();
    let project_info = engine.buffer_project(diff.new.buffer).unwrap();
    import.exposing_list().into_iter().for_each(|(_, exposed)| {
        remove_qualifier_for_exposed(
            engine,
            &diff.new,
            project_info,
            &import,
            &exposed,
            &mut edits,
        )
    });
    Ok(edits)
}

fn on_removed_exposing_list_from_import(
    engine: &RefactorEngine,
    diff: &SourceFileDiff,
    changes: TreeChanges,
) -> Result<Vec<Edit>, Error> {
    let import_node = changes
        .old_removed
        .first()
        .and_then(Node::parent)
        .ok_or(Error::TreeSitterExpectedNodeDoesNotExist)?;
    let mut cursor = QueryCursor::new();
    let import = engine
        .query_for_imports
        .run_in(&mut cursor, &diff.old, import_node)
        .next()
        .ok_or(Error::TreeSitterExpectedNodeDoesNotExist)?;
    let qualifier = import.aliased_name();
    let mut cursor_2 = QueryCursor::new();
    let mut edits = Vec::new();
    engine
        .query_for_imports
        .run(&mut cursor_2, &diff.old)
        .find(|import| import.aliased_name() == qualifier)
        .ok_or(Error::TreeSitterExpectedNodeDoesNotExist)?
        .exposing_list()
        .for_each(|(_, exposed)| {
            let mut val_cursor = QueryCursor::new();
            add_qualifier_to_name(
                engine,
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
    code: &'a SourceFileSnapshot,
    qualifier: &'a RopeSlice,
) -> Result<Import<'a>, Error> {
    query_for_imports
        .run(cursor, code)
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
    engine: &RefactorEngine,
    edits: &mut Vec<Edit>,
    cursor: &mut QueryCursor,
    code: &SourceFileSnapshot,
    import: &Import,
    exposed: &Exposed,
) {
    match exposed {
        Exposed::Operator(op) => {
            eprintln!(
                "[error] Cannot qualify operator {:?}",
                op.name.to_string(),
            );
        }
        Exposed::Type(type_) => {
            add_qualifier_to_type(engine, edits, cursor, code, import, type_);
            let ctors = match type_.constructors(engine) {
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
                engine, edits, cursor, code, import, ctors,
            );
        }
        Exposed::Value(val) => {
            add_qualifier_to_value(engine, edits, cursor, code, import, val);
        }
        Exposed::All(_) => {
            let exports =
                match engine.module_exports(code.buffer, import.name()) {
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
                        engine,
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
                            engine,
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
                            engine,
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
    engine: &RefactorEngine,
    edits: &mut Vec<Edit>,
    cursor: &mut QueryCursor,
    code: &SourceFileSnapshot,
    import: &Import,
    constructors: ExposedTypeConstructors,
) {
    for ctor in constructors {
        engine
            .query_for_unqualified_values
            .run(cursor, code)
            .for_each(|(node, Reference { name, kind })| {
                if *ctor == name && kind == ReferenceKind::Constructor {
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
    engine: &RefactorEngine,
    edits: &mut Vec<Edit>,
    cursor: &mut QueryCursor,
    code: &SourceFileSnapshot,
    import: &Import,
    exposed: &ExposedType,
) {
    let exposed_name = exposed.name;
    engine
        .query_for_unqualified_values
        .run(cursor, code)
        .for_each(|(node, Reference { name, kind })| {
            if exposed_name == name && kind == ReferenceKind::Type {
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
    engine: &RefactorEngine,
    edits: &mut Vec<Edit>,
    cursor: &mut QueryCursor,
    code: &SourceFileSnapshot,
    import: &Import,
    exposed: &ExposedValue,
) {
    let exposed_name = exposed.name;
    engine
        .query_for_unqualified_values
        .run(cursor, code)
        .for_each(|(node, Reference { name, kind })| {
            if exposed_name == name && kind == ReferenceKind::Value {
                edits.push(Edit::new(
                    code.buffer,
                    // TODO: remove need for clone()
                    &mut code.bytes.clone(),
                    &(node.start_byte()..node.start_byte()),
                    format!("{}.", import.aliased_name()),
                ))
            }
        })
}

struct QualifiedValuesQuery {
    query: Query,
    root_index: u32,
    qualifier_index: u32,
    value_index: u32,
    type_index: u32,
    constructor_index: u32,
}

impl QualifiedValuesQuery {
    fn init(lang: Language) -> Result<QualifiedValuesQuery, Error> {
        let query_str = r#"
            (_
              (
                (module_name_segment) @qualifier
                (dot)
              )+
              [
                (lower_case_identifier)  @value
                (type_identifier)        @type
                (constructor_identifier) @constructor
              ]
            ) @root
            "#;
        let query = Query::new(lang, query_str)
            .map_err(Error::TreeSitterFailedToParseQuery)?;
        let qualified_value_query = QualifiedValuesQuery {
            root_index: index_for_name(&query, "root")?,
            qualifier_index: index_for_name(&query, "qualifier")?,
            value_index: index_for_name(&query, "value")?,
            type_index: index_for_name(&query, "type")?,
            constructor_index: index_for_name(&query, "constructor")?,
            query,
        };
        Ok(qualified_value_query)
    }

    fn run_in<'a, 'tree>(
        &'a self,
        cursor: &'a mut QueryCursor,
        code: &'tree SourceFileSnapshot,
        node: Node<'tree>,
    ) -> QualifiedReferences<'a, 'tree> {
        QualifiedReferences {
            code,
            query: self,
            matches: cursor.matches(&self.query, node, code),
        }
    }
}

struct QualifiedReferences<'a, 'tree> {
    query: &'a QualifiedValuesQuery,
    code: &'tree SourceFileSnapshot,
    matches: tree_sitter::QueryMatches<'a, 'tree, &'a SourceFileSnapshot>,
}

impl<'a, 'tree> Iterator for QualifiedReferences<'a, 'tree> {
    type Item = (Node<'a>, QualifiedReference<'a>);

    fn next(&mut self) -> Option<Self::Item> {
        let mut qualifier_range = None;
        let mut root_node = None;
        let mut name_capture_index = None;
        let mut opt_name = None;
        let match_ = self.matches.next()?;
        match_.captures.iter().for_each(|capture| {
            if capture.index == self.query.root_index {
                root_node = Some(capture.node);
            }
            if capture.index == self.query.qualifier_index {
                match &qualifier_range {
                    None => qualifier_range = Some(capture.node.byte_range()),
                    Some(existing_range) => {
                        qualifier_range =
                            Some(existing_range.start..capture.node.end_byte())
                    }
                }
            } else {
                name_capture_index = Some(capture.index);
                opt_name = Some(self.code.slice(&capture.node.byte_range()))
            }
        });
        let name = opt_name
            .ok_or(Error::TreeSitterQueryReturnedNotEnoughMatches)
            .unwrap();

        let qualifier_range = qualifier_range
            .ok_or(Error::TreeSitterQueryReturnedNotEnoughMatches)
            .unwrap();
        let qualifier = self.code.slice(&qualifier_range);
        let kind = match name_capture_index
            .ok_or(Error::TreeSitterQueryReturnedNotEnoughMatches)
            .unwrap()
        {
            index if index == self.query.value_index => ReferenceKind::Value,
            index if index == self.query.type_index => ReferenceKind::Type,
            index if index == self.query.constructor_index => {
                ReferenceKind::Constructor
            }
            _ => panic!(),
        };
        let reference = Reference { name, kind };
        let qualified = QualifiedReference {
            qualifier,
            reference,
        };
        Some((root_node.unwrap(), qualified))
    }
}

#[derive(PartialEq)]
struct QualifiedReference<'a> {
    qualifier: RopeSlice<'a>,
    reference: Reference<'a>,
}

#[derive(PartialEq)]
struct Reference<'a> {
    name: RopeSlice<'a>,
    kind: ReferenceKind,
}

#[derive(PartialEq)]
enum ReferenceKind {
    Value,
    Type,
    Constructor,
    Operator,
}

struct UnqualifiedValuesQuery {
    query: Query,
    value_index: u32,
    type_index: u32,
    constructor_index: u32,
}

impl UnqualifiedValuesQuery {
    fn init(lang: Language) -> Result<UnqualifiedValuesQuery, Error> {
        let query_str = r#"
            [ (value_qid
                .
                (lower_case_identifier) @value
              )
              (type_qid
                .
                (type_identifier) @type
              )
              (constructor_qid
                .
                (constructor_identifier) @constructor
              )
            ]"#;
        let query = Query::new(lang, query_str)
            .map_err(Error::TreeSitterFailedToParseQuery)?;
        let unqualified_values_query = UnqualifiedValuesQuery {
            value_index: index_for_name(&query, "value")?,
            type_index: index_for_name(&query, "type")?,
            constructor_index: index_for_name(&query, "constructor")?,
            query,
        };
        Ok(unqualified_values_query)
    }

    fn run<'a, 'tree>(
        &'a self,
        cursor: &'a mut QueryCursor,
        code: &'tree SourceFileSnapshot,
    ) -> UnqualifiedValues<'a, 'tree> {
        let matches = cursor.matches(&self.query, code.tree.root_node(), code);
        UnqualifiedValues {
            matches,
            code,
            query: self,
        }
    }
}

struct UnqualifiedValues<'a, 'tree> {
    matches: tree_sitter::QueryMatches<'a, 'tree, &'a SourceFileSnapshot>,
    code: &'a SourceFileSnapshot,
    query: &'a UnqualifiedValuesQuery,
}

impl<'a, 'tree> Iterator for UnqualifiedValues<'a, 'tree> {
    type Item = (Node<'a>, Reference<'a>);

    fn next(&mut self) -> Option<Self::Item> {
        let match_ = self.matches.next()?;
        let capture = match_.captures.first()?;
        let kind = match capture.index {
            index if index == self.query.value_index => ReferenceKind::Value,
            index if index == self.query.type_index => ReferenceKind::Type,
            index if index == self.query.constructor_index => {
                ReferenceKind::Constructor
            }
            _ => panic!(),
        };
        let node = capture.node;
        let name = self.code.slice(&node.byte_range());
        let reference = Reference { name, kind };
        Some((node, reference))
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
            root_node: nodes[self.query.root_index as usize]?,
            name_node: nodes[self.query.name_index as usize]?,
            as_clause_node: nodes[self.query.as_clause_index as usize],
            exposing_list_node: nodes[self.query.exposing_list_index as usize],
        })
    }
}

struct Import<'a> {
    code: &'a SourceFileSnapshot,
    root_node: Node<'a>,
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
                        name: self.code.slice(&node.byte_range()),
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

#[derive(PartialEq)]
enum Exposed<'a> {
    Operator(ExposedOperator<'a>),
    Value(ExposedValue<'a>),
    Type(ExposedType<'a>),
    All(Node<'a>),
}

#[derive(PartialEq)]
struct ExposedOperator<'a> {
    name: RopeSlice<'a>,
}

#[derive(PartialEq)]
struct ExposedValue<'a> {
    name: RopeSlice<'a>,
}

#[derive(PartialEq)]
struct ExposedType<'a> {
    name: RopeSlice<'a>,
    buffer: Buffer,
    module_name: RopeSlice<'a>,
    exposing_constructors: bool,
}

impl ExposedType<'_> {
    fn constructors<'a>(
        &'a self,
        engine: &'a RefactorEngine,
    ) -> Result<ExposedTypeConstructors, Error> {
        if !self.exposing_constructors {
            return Ok(ExposedTypeConstructors::None);
        }
        let names = engine.constructors_for_type(
            self.buffer,
            self.module_name,
            self.name,
        )?;
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

pub(crate) fn project_root_for_path(path: &Path) -> Result<&Path, Error> {
    let mut maybe_root = path;
    loop {
        if maybe_root.join("elm.json").exists() {
            return Ok(maybe_root);
        } else {
            match maybe_root.parent() {
                None => {
                    return Err(Error::NoElmJsonFoundInAnyAncestorDirectory);
                }
                Some(parent) => {
                    maybe_root = parent;
                }
            }
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
                path.push("./tests/refactor-simulations");
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
        refactor_engine.init_buffer(buffer, path.to_owned())?;
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

    // Qualifying values
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
    simulation_test!(remove_exposing_all_clause_from_import);
    simulation_test!(remove_exposing_all_clause_from_local_import);
    simulation_test!(remove_exposing_clause_from_import);
    simulation_test!(remove_exposing_clause_from_import_with_as_clause);
    simulation_test!(remove_multiple_values_from_exposing_list_of_import);
    simulation_test!(remove_operator_from_exposing_list_of_import);
    simulation_test!(remove_type_with_constructor_from_exposing_list_of_import);
    simulation_test!(remove_value_from_exposing_list_of_import_with_as_clause);
    simulation_test!(remove_variable_from_exposing_list_of_import);

    // Removing module qualifiers from values
    simulation_test!(remove_module_qualifier_from_variable);
    simulation_test!(remove_module_qualifier_from_type);
    simulation_test!(
        remove_module_qualifier_inserting_variable_at_end_of_exposing_list
    );
    simulation_test!(remove_module_qualifier_for_module_without_exposing_list);
    simulation_test!(remove_module_qualifier_for_module_exposing_all);
    simulation_test!(remove_module_qualifier_from_constructor);
    simulation_test!(remove_module_qualifier_from_exposed_constructor);
    simulation_test!(remove_module_qualifier_from_constructor_of_exposed_type);
    simulation_test!(add_value_to_exposing_list);
    simulation_test!(add_type_to_exposing_list);
    simulation_test!(add_constructors_for_type_to_exposing_list);
    simulation_test!(add_type_exposing_constructors_to_exposing_list);
    simulation_test!(add_exposing_list);
    simulation_test!(add_exposing_all_list);
    simulation_test!(add_and_remove_items_in_exposing_list);

    // --- TESTS DEMONSTRATING CURRENT BUGS ---

    // The exposing lists in these tests contained an operator. It doesn't get a
    // qualifier because Elm doesn't allow qualified operators, and as a result
    // this refactor doesn't produce compiling code.
    // Potential fix: Add the exposing list back containing just the operator.
    simulation_test!(remove_exposing_clause_containing_operator_from_import);
    simulation_test!(
        remove_exposing_all_clause_containing_operator_from_import
    );

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
