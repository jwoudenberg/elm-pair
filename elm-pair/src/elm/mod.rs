use crate::analysis_thread::{SourceFileDiff, TreeChanges};
use crate::elm::dependencies::{
    index_for_name, load_dependencies, ElmExport, ElmModule, ProjectInfo,
    QueryForExports,
};
use crate::support::log;
use crate::support::log::Error;
use crate::support::source_code::{Buffer, Edit, SourceFileSnapshot};
use core::ops::Range;
use ropey::{Rope, RopeSlice};
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tree_sitter::{Language, Node, Query, QueryCursor, QueryMatch, TreeCursor};

pub mod compiler;
pub mod dependencies;
pub mod idat;
pub mod query;

// These constants come from the tree-sitter-elm grammar. They might need to
// be changed when tree-sitter-elm updates.
const AS_CLAUSE: u16 = 101;
const BLOCK_COMMENT: u16 = 86;
const COMMA: u16 = 6;
const CONSTRUCTOR_IDENTIFIER: u16 = 8;
const CONSTRUCTOR_QID: u16 = 96;
const DOT: u16 = 55;
const DOUBLE_DOT: u16 = 49;
const EXPOSED_OPERATOR: u16 = 94;
const EXPOSED_TYPE: u16 = 92;
const EXPOSED_UNION_CONSTRUCTORS: u16 = 93;
const EXPOSED_VALUE: u16 = 91;
const EXPOSING_LIST: u16 = 90;
const LOWER_CASE_IDENTIFIER: u16 = 1;
const MODULE_NAME_SEGMENT: u16 = 201;
const MODULE_DECLARATION: u16 = 87;
const TYPE_IDENTIFIER: u16 = 33;
const TYPE_QID: u16 = 97;
const VALUE_QID: u16 = 98;

#[cfg(test)]
mod kind_constants {
    #[test]
    fn check_kind_constants() {
        let language = tree_sitter_elm::language();
        let check = |constant, str, named| {
            assert_eq!(constant, language.id_for_node_kind(str, named))
        };
        check(super::AS_CLAUSE, "as_clause", true);
        check(super::BLOCK_COMMENT, "block_comment", true);
        check(super::COMMA, ",", false);
        check(
            super::CONSTRUCTOR_IDENTIFIER,
            "constructor_identifier",
            true,
        );
        check(super::CONSTRUCTOR_QID, "constructor_qid", true);
        check(super::DOT, "dot", true);
        check(super::DOUBLE_DOT, "double_dot", true);
        check(super::EXPOSED_OPERATOR, "exposed_operator", true);
        check(super::EXPOSED_TYPE, "exposed_type", true);
        check(
            super::EXPOSED_UNION_CONSTRUCTORS,
            "exposed_union_constructors",
            true,
        );
        check(super::EXPOSED_VALUE, "exposed_value", true);
        check(super::EXPOSING_LIST, "exposing_list", true);
        check(super::LOWER_CASE_IDENTIFIER, "lower_case_identifier", true);
        check(super::MODULE_NAME_SEGMENT, "module_name_segment", true);
        check(super::MODULE_DECLARATION, "module_declaration", true);
        check(super::TYPE_IDENTIFIER, "type_identifier", true);
        check(super::TYPE_QID, "type_qid", true);
        check(super::VALUE_QID, "value_qid", true);
    }
}

const IMPLICIT_ELM_IMPORTS: [&str; 10] = [
    "Basics", "Char", "Cmd", "List", "Maybe", "Platform", "Result", "String",
    "Sub", "Tuple",
];

pub(crate) struct RefactorEngine {
    buffers: HashMap<Buffer, BufferInfo>,
    projects: HashMap<PathBuf, ProjectInfo>,
    query_for_imports: QueryForImports,
    query_for_unqualified_values: QueryForUnqualifiedValues,
    query_for_qualified_values: QueryForQualifiedValues,
    query_for_exports: QueryForExports,
}

pub struct BufferInfo {
    pub project_root: PathBuf,
    pub path: PathBuf,
}

pub(crate) struct Refactor {
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
    pub(crate) fn new() -> Result<RefactorEngine, Error> {
        let language = tree_sitter_elm::language();
        let engine = RefactorEngine {
            buffers: HashMap::new(),
            projects: HashMap::new(),
            query_for_imports: QueryForImports::init(language)?,
            query_for_unqualified_values: QueryForUnqualifiedValues::init(
                language,
            )?,
            query_for_qualified_values: QueryForQualifiedValues::init(
                language,
            )?,
            query_for_exports: QueryForExports::init(language)?,
        };

        Ok(engine)
    }

    // TODO: try to return an Iterator instead of a Vector.
    // TODO: Try remove Vector from TreeChanges type.
    pub(crate) fn respond_to_change<'a>(
        &self,
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
        match (before.as_slice(), after.as_slice()) {
            (
                [EXPOSED_VALUE | EXPOSED_TYPE, ..]
                | [COMMA, EXPOSED_VALUE | EXPOSED_TYPE, ..]
                | [DOUBLE_DOT]
                | [],
                [EXPOSED_VALUE | EXPOSED_TYPE, ..]
                | [COMMA, EXPOSED_VALUE | EXPOSED_TYPE, ..]
                | [DOUBLE_DOT]
                | [],
            ) => on_changed_values_in_exposing_list(
                self,
                &mut refactor,
                diff,
                changes.old_parent,
                changes.new_parent,
            )?,
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
            ) => on_added_module_qualifier_to_value(
                self,
                &mut refactor,
                diff,
                changes.old_parent,
                changes.new_parent,
            )?,
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
            ) => on_removed_module_qualifier_from_value(
                self,
                &mut refactor,
                diff,
                changes.old_parent,
                changes.new_parent,
            )?,
            ([], [EXPOSING_LIST]) => on_added_exposing_list_to_import(
                self,
                &mut refactor,
                &diff.new,
                changes.new_parent,
            )?,
            ([EXPOSING_LIST], []) => on_removed_exposing_list_from_import(
                self,
                &mut refactor,
                diff,
                changes.old_parent,
            )?,
            ([], [EXPOSED_UNION_CONSTRUCTORS]) => {
                on_added_constructors_to_exposing_list(
                    self,
                    &mut refactor,
                    diff,
                    changes.new_parent,
                )?
            }
            ([EXPOSED_UNION_CONSTRUCTORS], []) => {
                on_removed_constructors_from_exposing_list(
                    self,
                    &mut refactor,
                    diff,
                    changes.old_parent,
                )?
            }
            ([] | [AS_CLAUSE], [AS_CLAUSE] | []) => on_changed_as_clause(
                self,
                &mut refactor,
                diff,
                changes.old_parent,
                changes.new_parent,
            )?,
            ([.., MODULE_NAME_SEGMENT], [.., MODULE_NAME_SEGMENT]) => {
                on_changed_module_name(
                    self,
                    &mut refactor,
                    diff,
                    changes.old_parent,
                    changes.new_parent,
                )?
            }
            _ => on_unrecognized_change(
                self,
                &mut refactor,
                &diff.new,
                changes.new_parent,
            )?,
        };
        Ok(refactor)
    }

    pub(crate) fn module_exports(
        &self,
        buffer: Buffer,
        module: RopeSlice,
    ) -> Result<&Vec<ElmExport>, Error> {
        let project = self.buffer_project(buffer)?;
        match project.modules.get(&module.to_string()) {
            None => Err(log::mk_err!("did not find module")),
            Some(ElmModule { exports }) => Ok(exports),
        }
    }

    fn buffer_project(&self, buffer: Buffer) -> Result<&ProjectInfo, Error> {
        let buffer_info = self.buffers.get(&buffer).ok_or_else(|| {
            log::mk_err!("no project on file for buffer {:?}", buffer)
        })?;
        let project_info = self
            .projects
            .get(&buffer_info.project_root)
            .ok_or_else(|| {
                log::mk_err!("did not find project for buffer {:?}", buffer)
            })?;
        Ok(project_info)
    }

    pub(crate) fn init_buffer<W>(
        &mut self,
        buffer: Buffer,
        path: PathBuf,
        watch_path: &mut W,
    ) -> Result<(), Error>
    where
        W: FnMut(&Path),
    {
        let project_root = project_root_for_path(&path)?.to_owned();
        if !self.projects.contains_key(&project_root) {
            let project_info = get_project_info(
                &self.query_for_exports,
                &project_root,
                watch_path,
            )?;
            self.projects.insert(project_root.to_owned(), project_info);
        }
        let buffer_info = BufferInfo { path, project_root };
        self.buffers.insert(buffer, buffer_info);
        Ok(())
    }

    pub(crate) fn on_files_changed<W>(
        &mut self,
        paths: &HashSet<PathBuf>,
        watch_path: &mut W,
    ) -> Result<(), Error>
    where
        W: FnMut(&Path),
    {
        let RefactorEngine {
            projects,
            query_for_exports,
            ..
        } = self;
        projects
            .iter_mut()
            .try_for_each(|(project_root, project_info)| {
                let project_changed = paths
                    .contains(&project_info.elm_json_path)
                    || paths.contains(&project_info.idat_path)
                    || paths
                        .iter()
                        .any(|path| is_project_source_file(project_info, path));
                // TODO: Don't reparse entire project when single file changes.
                if project_changed {
                    log::info!(
                        "changed files cause reparsing of project {:?}",
                        project_root
                    );
                    *project_info = get_project_info(
                        query_for_exports,
                        project_root,
                        watch_path,
                    )?;
                }
                Ok(())
            })
    }
}

fn is_elm_file(path: &Path) -> bool {
    path.extension() == Some(std::ffi::OsStr::new("elm"))
}

fn is_project_source_file(project_info: &ProjectInfo, path: &Path) -> bool {
    is_elm_file(path)
        && project_info
            .source_directories
            .iter()
            .any(|dir| path.starts_with(dir))
}

fn get_project_info<W>(
    query_for_exports: &QueryForExports,
    project_root: &Path,
    watch_path: &mut W,
) -> Result<ProjectInfo, Error>
where
    W: FnMut(&Path),
{
    let now = std::time::Instant::now();
    let project_info = load_dependencies(query_for_exports, project_root)?;
    // TODO: deal with possibility of elm-stuff/i.dat being out of date
    watch_path(&project_info.elm_json_path);
    watch_path(&project_info.idat_path);
    for dir in project_info.source_directories.iter() {
        watch_path(dir);
    }
    let elapsed_time = now.elapsed();
    log::info!(
        "parsed project {:?} in {}ms",
        project_root,
        elapsed_time.as_millis()
    );
    Ok(project_info)
}

#[allow(clippy::needless_collect)]
fn on_unrecognized_change(
    engine: &RefactorEngine,
    refactor: &mut Refactor,
    code: &SourceFileSnapshot,
    parent: Node,
) -> Result<(), Error> {
    // Check for new qualified variables, which might indicate imports we need
    // to add.
    let mut cursor = QueryCursor::new();
    let existing_imports: Vec<Rope> = engine
        .query_for_imports
        .run(&mut cursor, code)
        .map(|import| import.aliased_name().into())
        .collect();
    let mut new_import_names = HashSet::new();
    for result in
        engine
            .query_for_qualified_values
            .run_in(&mut cursor, code, parent)
    {
        let (_, reference) = result?;
        if reference.reference.name.len_bytes() > 0
            && !existing_imports.contains(&reference.qualifier)
        {
            new_import_names.insert(reference.qualifier.to_string());
        }
    }
    if !new_import_names.is_empty() {
        let project_info = engine.buffer_project(code.buffer)?;
        let mut tree_cursor = code.tree.root_node().walk();
        tree_cursor.goto_first_child();
        while (tree_cursor.node().kind_id() == MODULE_DECLARATION
            || tree_cursor.node().kind_id() == BLOCK_COMMENT)
            && tree_cursor.goto_next_sibling()
        {}
        let insert_at_byte = tree_cursor.node().start_byte();
        for new_import_name in new_import_names {
            if project_info.modules.contains_key(&new_import_name)
                && !IMPLICIT_ELM_IMPORTS.contains(&new_import_name.as_str())
            {
                refactor.add_change(
                    insert_at_byte..insert_at_byte,
                    format!("import {}\n", new_import_name),
                );
            }
        }
    }
    Ok(())
}

fn on_added_constructors_to_exposing_list(
    engine: &RefactorEngine,
    refactor: &mut Refactor,
    diff: &SourceFileDiff,
    parent: Node,
) -> Result<(), Error> {
    let type_name_node = parent.child(0).ok_or_else(|| {
        log::mk_err!("did not find node with type name of exposed constructor")
    })?;
    let type_name = diff.new.slice(&type_name_node.byte_range());
    let import_node =
        parent.parent().and_then(|n| n.parent()).ok_or_else(|| {
            log::mk_err!("did not find parent import of exposed constructor")
        })?;
    let import = parse_import_node(engine, &diff.new, import_node)?;
    let project_info = engine.buffer_project(diff.new.buffer)?;
    let module = get_elm_module(project_info, &import.unaliased_name())?;
    let mut references_to_unqualify = HashSet::new();
    for result in import.exposing_list() {
        let (_, exposed) = result?;
        if let Exposed::Type(type_) = &exposed {
            if type_.name == type_name {
                exposed.for_each_reference(module, |reference| {
                    if reference.kind == ReferenceKind::Constructor {
                        references_to_unqualify.insert(reference);
                    }
                });
                break;
            }
        }
    }
    remove_qualifier_from_references(
        engine,
        refactor,
        &diff.new,
        &import.aliased_name(),
        references_to_unqualify,
        None,
    )?;
    Ok(())
}

fn get_elm_module<'a>(
    project_info: &'a ProjectInfo,
    name: &RopeSlice,
) -> Result<&'a ElmModule, Error> {
    project_info.modules.get(&name.to_string()).ok_or_else(|| {
        log::mk_err!("could not find module named {}", name.to_string())
    })
}

fn on_changed_module_name(
    engine: &RefactorEngine,
    refactor: &mut Refactor,
    diff: &SourceFileDiff,
    old_parent_node: Node,
    new_parent_node: Node,
) -> Result<(), Error> {
    match old_parent_node.kind_id() {
        AS_CLAUSE => {
            let old_import_node =
                old_parent_node.parent().ok_or_else(|| {
                    log::mk_err!("found an unexpected root as_clause node")
                })?;
            let new_import_node =
                new_parent_node.parent().ok_or_else(|| {
                    log::mk_err!("found an unexpected root as_clause node")
                })?;
            on_changed_as_clause(
                engine,
                refactor,
                diff,
                old_import_node,
                new_import_node,
            )?;
        }
        VALUE_QID | TYPE_QID | CONSTRUCTOR_QID => {
            on_changed_module_qualifier(
                engine,
                refactor,
                diff,
                old_parent_node,
                new_parent_node,
            )?;
        }
        _ => {}
    };
    Ok(())
}

fn on_changed_module_qualifier(
    engine: &RefactorEngine,
    refactor: &mut Refactor,
    diff: &SourceFileDiff,
    old_parent_node: Node,
    new_parent_node: Node,
) -> Result<(), Error> {
    let mut cursor = QueryCursor::new();
    let (_, old_reference) = engine
        .query_for_qualified_values
        .run_in(&mut cursor, &diff.old, old_parent_node)
        .next()
        .ok_or_else(|| {
            log::mk_err!("parsing qualified value node using query failed")
        })??;
    let (_, new_reference) = engine
        .query_for_qualified_values
        .run_in(&mut cursor, &diff.new, new_parent_node)
        .next()
        .ok_or_else(|| {
            log::mk_err!("parsing qualified value node using query failed")
        })??;

    let import = engine
        .query_for_imports
        .run(&mut cursor, &diff.new)
        .find(|import| import.aliased_name() == old_reference.qualifier)
        .ok_or_else(|| {
            log::mk_err!(
                "did not find import statement with the expected aliased name"
            )
        })?;
    match import.as_clause_node {
        Some(as_clause_name_node) => {
            if import.unaliased_name() == new_reference.qualifier {
                let as_clause_node =
                    as_clause_name_node.parent().ok_or_else(|| {
                        log::mk_err!(
                            "found unexpected root as clause name nood"
                        )
                    })?;
                refactor.add_change(
                    (as_clause_node.start_byte() - 1)
                        ..as_clause_node.end_byte(),
                    String::new(),
                )
            } else {
                refactor.add_change(
                    as_clause_name_node.byte_range(),
                    new_reference.qualifier.to_string(),
                )
            }
        }
        None => {
            let insert_point = import.name_node.end_byte();
            refactor.add_change(
                insert_point..insert_point,
                format!(" as {}", new_reference.qualifier),
            );
        }
    }

    change_qualifier(
        engine,
        refactor,
        diff,
        old_reference.qualifier.slice(..),
        new_reference.qualifier.slice(..),
    )?;
    Ok(())
}

fn on_changed_as_clause(
    engine: &RefactorEngine,
    refactor: &mut Refactor,
    diff: &SourceFileDiff,
    old_import_node: Node,
    new_import_node: Node,
) -> Result<(), Error> {
    let new_import = parse_import_node(engine, &diff.new, new_import_node)?;
    let new_aliased_name = new_import.aliased_name();
    let old_import = parse_import_node(engine, &diff.old, old_import_node)?;
    let old_aliased_name = old_import.aliased_name();
    change_qualifier(engine, refactor, diff, old_aliased_name, new_aliased_name)
}

fn change_qualifier(
    engine: &RefactorEngine,
    refactor: &mut Refactor,
    diff: &SourceFileDiff,
    old_aliased_name: RopeSlice,
    new_aliased_name: RopeSlice,
) -> Result<(), Error> {
    let mut cursor = QueryCursor::new();
    for result in engine
        .query_for_qualified_values
        .run(&mut cursor, &diff.new)
    {
        let (node, reference) = result?;
        let old_qualifier_len = 1 + old_aliased_name.len_bytes();
        if reference.qualifier == old_aliased_name {
            refactor.add_change(
                node.start_byte()..(node.start_byte() + old_qualifier_len),
                format!("{}.", new_aliased_name),
            );
        }
    }
    Ok(())
}

fn on_removed_constructors_from_exposing_list(
    engine: &RefactorEngine,
    refactor: &mut Refactor,
    diff: &SourceFileDiff,
    old_parent: Node,
) -> Result<(), Error> {
    let type_name_node = old_parent.child(0).ok_or_else(|| {
        log::mk_err!("could not find name node of exposed type node")
    })?;
    let type_name = diff.old.slice(&type_name_node.byte_range());
    let old_import_node = old_parent
        .parent()
        .and_then(|n| n.parent())
        .ok_or_else(|| {
            log::mk_err!(
                "could not find import parent node of exposed type node"
            )
        })?;
    let old_import = parse_import_node(engine, &diff.old, old_import_node)?;
    let mut references_to_qualify = HashSet::new();
    for result in old_import.exposing_list() {
        let (_, exposed) = result?;
        if let Exposed::Type(type_) = exposed {
            if type_.name == type_name {
                match type_.constructors(engine)? {
                    ExposedConstructors::FromTypeAlias(ctor) => {
                        references_to_qualify.insert(Reference {
                            name: Rope::from_str(ctor),
                            kind: ReferenceKind::Constructor,
                        });
                    }
                    ExposedConstructors::FromCustomType(ctors) => {
                        for ctor in ctors {
                            references_to_qualify.insert(Reference {
                                name: Rope::from_str(ctor),
                                kind: ReferenceKind::Constructor,
                            });
                        }
                    }
                }
                break;
            }
        }
    }
    add_qualifier_to_references(
        engine,
        refactor,
        &mut QueryCursor::new(),
        &diff.new,
        None,
        &old_import,
        references_to_qualify,
    )?;
    Ok(())
}

fn on_changed_values_in_exposing_list(
    engine: &RefactorEngine,
    refactor: &mut Refactor,
    diff: &SourceFileDiff,
    old_parent: Node,
    new_parent: Node,
) -> Result<(), Error> {
    let old_import_node = old_parent.parent().ok_or_else(|| {
        log::mk_err!("could not find parent import node of exposing list")
    })?;
    let old_import = parse_import_node(engine, &diff.old, old_import_node)?;
    let project_info = engine.buffer_project(diff.new.buffer)?;
    let module = get_elm_module(project_info, &old_import.unaliased_name())?;
    let mut old_references = HashSet::new();
    for result in old_import.exposing_list() {
        let (_, exposed) = result?;
        exposed.for_each_reference(module, |reference| {
            old_references.insert(reference);
        });
    }

    let new_import_node = new_parent.parent().ok_or_else(|| {
        log::mk_err!(
            "could not find import node as parent of exposing list node"
        )
    })?;
    let new_import = parse_import_node(engine, &diff.new, new_import_node)?;
    let mut new_references = HashSet::new();
    for result in new_import.exposing_list() {
        let (_, exposed) = result?;
        exposed.for_each_reference(module, |reference| {
            new_references.insert(reference);
        });
    }

    let references_to_qualify = old_references
        .clone()
        .into_iter()
        .filter(|reference| !new_references.contains(reference))
        .collect();

    let references_to_unqualify = new_references
        .into_iter()
        .filter(|reference| !old_references.contains(reference))
        .collect();

    add_qualifier_to_references(
        engine,
        refactor,
        &mut QueryCursor::new(),
        &diff.new,
        None,
        &new_import,
        references_to_qualify,
    )?;

    remove_qualifier_from_references(
        engine,
        refactor,
        &diff.new,
        &new_import.aliased_name(),
        references_to_unqualify,
        None,
    )?;

    Ok(())
}

fn on_removed_module_qualifier_from_value(
    engine: &RefactorEngine,
    refactor: &mut Refactor,
    diff: &SourceFileDiff,
    old_parent: Node,
    new_parent: Node,
) -> Result<(), Error> {
    let mut cursor = QueryCursor::new();
    let (node_stripped_of_qualifier, new_reference) = engine
        .query_for_unqualified_values
        .run_in(&mut cursor, &diff.new, new_parent)
        .next()
        .ok_or_else(|| {
            log::mk_err!("parsing unqualified value node using query failed")
        })??;
    let mut cursor2 = QueryCursor::new();
    let (
        _,
        QualifiedReference {
            reference,
            qualifier,
        },
    ) = engine
        .query_for_qualified_values
        .run_in(&mut cursor2, &diff.old, old_parent)
        .next()
        .ok_or_else(|| {
            log::mk_err!("parsing qualified value node using query failed")
        })??;
    if new_reference.name != reference.name {
        return Ok(());
    }
    let import =
        get_import_by_aliased_name(engine, &diff.new, &qualifier.slice(..))?;
    let mut references_to_unqualify = HashSet::new();
    if reference.kind == ReferenceKind::Constructor {
        let project_info = engine.buffer_project(diff.new.buffer)?;
        let module = get_elm_module(project_info, &import.unaliased_name())?;
        for export in module.exports.iter() {
            match export {
                ElmExport::Value { .. } => {}
                ElmExport::RecordTypeAlias { name } => {
                    // We're dealing here with a type alias being used as a
                    // constructor. For example, given a type alias like:
                    //
                    //     type alias Point = { x : Int, y : Int }
                    //
                    // constructor usage would be doing this:
                    //
                    //     point = Point 7 2
                    if name == &reference.name.to_string() {
                        references_to_unqualify.insert(Reference {
                            kind: ReferenceKind::Constructor,
                            name: Rope::from_str(name),
                        });
                        add_to_exposing_list(
                            &import,
                            &Reference {
                                kind: ReferenceKind::Type,
                                name: reference.name,
                            },
                            None,
                            refactor,
                        )?;
                        break;
                    }
                }
                ElmExport::Type { name, constructors } => {
                    if constructors.contains(&reference.name.to_string()) {
                        for ctor in constructors.iter() {
                            references_to_unqualify.insert(Reference {
                                kind: ReferenceKind::Constructor,
                                name: Rope::from_str(ctor),
                            });
                        }
                        add_to_exposing_list(
                            &import,
                            &reference,
                            Some(name),
                            refactor,
                        )?;
                        references_to_unqualify.insert(reference);
                        break;
                    }
                }
            }
        }
    } else {
        add_to_exposing_list(&import, &reference, None, refactor)?;
        references_to_unqualify.insert(reference);
    };
    remove_qualifier_from_references(
        engine,
        refactor,
        &diff.new,
        &qualifier.slice(..),
        references_to_unqualify,
        Some(node_stripped_of_qualifier),
    )?;
    Ok(())
}

fn remove_qualifier_from_references(
    engine: &RefactorEngine,
    refactor: &mut Refactor,
    code: &SourceFileSnapshot,
    qualifier: &RopeSlice,
    references: HashSet<Reference>,
    // If we're removing qualifiers because the programmer started by removing
    // the qualifier from a single node, this is that node.
    // Our logic renaming a pre-existing variable of the same name should not
    // rename this node.
    node_stripped_of_qualifier: Option<Node>,
) -> Result<(), Error> {
    // Find existing unqualified references, so we can check whether removing
    // a qualifier from a qualified reference will introduce a naming conflict.
    let mut cursor = QueryCursor::new();
    let names_in_use: HashSet<Reference> = engine
        .query_for_unqualified_values
        .run_in(&mut cursor, code, code.tree.root_node())
        .map(|r| r.map(|(_, reference)| reference))
        .collect::<Result<HashSet<Reference>, Error>>()?;

    let mut names_from_other_modules: HashMap<Reference, Rope> = HashMap::new();
    let imports = engine.query_for_imports.run(&mut cursor, code);
    let project_info = engine.buffer_project(code.buffer)?;
    for import in imports {
        if &import.aliased_name() == qualifier {
            continue;
        } else {
            for res in import.exposing_list() {
                let (_, exposed) = res?;
                let module =
                    get_elm_module(project_info, &import.unaliased_name())?;
                exposed.for_each_reference(module, |reference| {
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
                refactor,
                code,
                node_stripped_of_qualifier,
                other_qualifier,
                reference,
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
        if references.contains(&qualified.reference) {
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
    base_reference: &'a Reference,
    next_digit: usize,
}

impl<'a> Iterator for NamesWithDigit<'a> {
    type Item = Reference;

    fn next(&mut self) -> Option<Self::Item> {
        let mut new_name = self.base_reference.name.clone();
        new_name.append(Rope::from_str(self.next_digit.to_string().as_str()));
        let next_ref = Reference {
            name: new_name,
            kind: self.base_reference.kind,
        };
        self.next_digit += 1;
        Some(next_ref)
    }
}

fn names_with_digit(reference: &Reference) -> NamesWithDigit {
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
        let base_reference = Reference {
            name: Rope::from_str("hi"),
            kind: ReferenceKind::Value,
        };
        let first_tree: Vec<Rope> = names_with_digit(&base_reference)
            .map(|reference| reference.name)
            .take(3)
            .collect();
        assert_eq!(first_tree, vec!["hi2", "hi3", "hi4"]);
    }
}

fn rename(
    engine: &RefactorEngine,
    refactor: &mut Refactor,
    code: &SourceFileSnapshot,
    node_stripped_of_qualifier: Option<Node>,
    from: &Reference,
    to: &Reference,
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
    reference: &Reference,
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
            Exposed::Operator(op) => op.name,
            Exposed::Value(val) => val.name,
            Exposed::Type(type_) => type_.name,
            Exposed::All => {
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

fn on_added_module_qualifier_to_value(
    engine: &RefactorEngine,
    refactor: &mut Refactor,
    diff: &SourceFileDiff,
    old_parent: Node,
    new_parent: Node,
) -> Result<(), Error> {
    let mut cursor = QueryCursor::new();
    let (_, old_reference) = engine
        .query_for_unqualified_values
        .run_in(&mut cursor, &diff.old, old_parent)
        .next()
        .ok_or_else(|| {
            log::mk_err!("parsing unqualified value node using query failed")
        })??;
    let (
        _,
        QualifiedReference {
            qualifier,
            reference,
        },
    ) = engine
        .query_for_qualified_values
        .run_in(&mut cursor, &diff.new, new_parent)
        .next()
        .ok_or_else(|| {
            log::mk_err!("parsing qualified value node using query failed")
        })??;
    if old_reference.name == reference.name {
        qualify_value(engine, refactor, &diff.new, None, &qualifier, &reference)
    } else {
        Ok(())
    }
}

fn qualify_value(
    engine: &RefactorEngine,
    refactor: &mut Refactor,
    code: &SourceFileSnapshot,
    node_to_skip: Option<Node>,
    qualifier: &Rope,
    reference: &Reference,
) -> Result<(), Error> {
    let import =
        get_import_by_aliased_name(engine, code, &qualifier.slice(..))?;

    let exposing_list_length = import.exposing_list().count();
    let mut references_to_qualify = HashSet::new();
    for result in import.exposing_list() {
        let (node, exposed) = result?;
        match &exposed {
            Exposed::Operator(op) => {
                if op.name == reference.name
                    && reference.kind == ReferenceKind::Operator
                {
                    return Err(log::mk_err!(
                        "cannot qualify operator, Elm doesn't allow it!"
                    ));
                }
            }
            Exposed::Type(type_) => {
                if type_.name == reference.name
                    && reference.kind == ReferenceKind::Type
                {
                    if exposing_list_length == 1 {
                        remove_exposing_list(refactor, &import);
                    } else {
                        remove_from_exposing_list(refactor, &node)?;
                    }
                    references_to_qualify.insert(Reference {
                        name: type_.name.into(),
                        kind: ReferenceKind::Type,
                    });
                }

                match type_.constructors(engine)? {
                    ExposedConstructors::FromTypeAlias(ctor) => {
                        if ctor == &reference.name {
                            // Ensure we don't remove the item from the exposing
                            // list twice (see code above).
                            // TODO: Clean this up.
                            if reference.kind != ReferenceKind::Type {
                                if exposing_list_length == 1 {
                                    remove_exposing_list(refactor, &import);
                                } else {
                                    remove_from_exposing_list(refactor, &node)?;
                                }
                            }
                            references_to_qualify.insert(Reference {
                                name: Rope::from_str(ctor),
                                kind: ReferenceKind::Type,
                            });
                            references_to_qualify.insert(Reference {
                                name: Rope::from_str(ctor),
                                kind: ReferenceKind::Constructor,
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
                                ctors.iter().map(|ctor| Reference {
                                    name: Rope::from_str(ctor),
                                    kind: ReferenceKind::Constructor,
                                });
                            references_to_qualify
                                .extend(constructor_references);
                        }
                    }
                }
            }
            Exposed::Value(val) => {
                if val.name == reference.name
                    && reference.kind == ReferenceKind::Value
                {
                    if exposing_list_length == 1 {
                        remove_exposing_list(refactor, &import);
                    } else {
                        remove_from_exposing_list(refactor, &node)?;
                    }
                    references_to_qualify.insert(Reference {
                        name: val.name.into(),
                        kind: ReferenceKind::Value,
                    });
                    break;
                }
            }
            Exposed::All => {
                // The programmer qualified a value coming from a module that
                // exposes everything. We could interpret this to mean that the
                // programmer wishes to qualify all values of this module. That
                // would potentially result in a lot of changes though, so we're
                // going to be more conservative and qualify only other
                // occurences of the same value. If the programmer really wishes
                // to qualify everything they can indicate so by removing the
                // `exposing (..)` clause.
                match reference.kind {
                    ReferenceKind::Operator => {
                        return Err(log::mk_err!(
                            "cannot qualify operator, Elm doesn't allow it!"
                        ));
                    }
                    ReferenceKind::Value | ReferenceKind::Type => {
                        references_to_qualify.insert(reference.clone());
                    }
                    ReferenceKind::Constructor => {
                        // We know a constructor got qualified, but not which
                        // type it belongs too. To find it, we iterate over all
                        // the exports from the module matching the qualifier we
                        // added. The type must be among them!
                        let exports = match engine.module_exports(
                            code.buffer,
                            import.unaliased_name(),
                        ) {
                            Ok(exports_) => exports_,
                            Err(err) => {
                                log::error!(
                                    "failed to read exports of {}: {:?}",
                                    import.unaliased_name().to_string(),
                                    err
                                );
                                break;
                            }
                        };
                        for export in exports {
                            match export {
                                ElmExport::Value { .. } => {}
                                ElmExport::RecordTypeAlias { .. } => {}
                                ElmExport::Type { constructors, .. } => {
                                    if constructors
                                        .iter()
                                        .any(|ctor| *ctor == reference.name)
                                    {
                                        let constructor_references =
                                            constructors.iter().map(|ctor| Reference {
                                                name: Rope::from_str(ctor),
                                                kind: ReferenceKind::Constructor,
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

fn on_added_exposing_list_to_import(
    engine: &RefactorEngine,
    refactor: &mut Refactor,
    code: &SourceFileSnapshot,
    new_parent: Node,
) -> Result<(), Error> {
    let import = parse_import_node(engine, code, new_parent)?;
    let project_info = engine.buffer_project(code.buffer)?;
    let module = get_elm_module(project_info, &import.unaliased_name())?;
    let mut references_to_unqualify = HashSet::new();
    for result in import.exposing_list() {
        let (_, exposed) = result?;
        exposed.for_each_reference(module, |reference| {
            references_to_unqualify.insert(reference);
        })
    }
    remove_qualifier_from_references(
        engine,
        refactor,
        code,
        &import.aliased_name(),
        references_to_unqualify,
        None,
    )?;
    Ok(())
}

fn on_removed_exposing_list_from_import(
    engine: &RefactorEngine,
    refactor: &mut Refactor,
    diff: &SourceFileDiff,
    old_parent: Node,
) -> Result<(), Error> {
    let import = parse_import_node(engine, &diff.old, old_parent)?;
    let qualifier = import.aliased_name();
    let mut val_cursor = QueryCursor::new();
    let project_info = engine.buffer_project(diff.new.buffer)?;
    let module = get_elm_module(project_info, &import.unaliased_name())?;
    let mut references_to_qualify = HashSet::new();
    let import =
        get_import_by_aliased_name(engine, &diff.old, &qualifier.slice(..))?;
    for result in import.exposing_list() {
        let (_, exposed) = result?;
        exposed.for_each_reference(module, |reference| {
            references_to_qualify.insert(reference);
        });
    }
    add_qualifier_to_references(
        engine,
        refactor,
        &mut val_cursor,
        &diff.new,
        None,
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
    engine: &RefactorEngine,
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
    engine: &RefactorEngine,
    refactor: &mut Refactor,
    cursor: &mut QueryCursor,
    code: &SourceFileSnapshot,
    node_to_skip: Option<Node>,
    import: &Import,
    references: HashSet<Reference>,
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

query::query!(
    QueryForQualifiedValues,
    query_for_qualified_values,
    "./queries/qualified_values",
    root,
    qualifier,
    value,
    type_,
    constructor,
);

impl QueryForQualifiedValues {
    fn run<'a, 'tree>(
        &'a self,
        cursor: &'a mut QueryCursor,
        code: &'tree SourceFileSnapshot,
    ) -> QualifiedReferences<'a, 'tree> {
        self.run_in(cursor, code, code.tree.root_node())
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
    query: &'a QueryForQualifiedValues,
    code: &'tree SourceFileSnapshot,
    matches: tree_sitter::QueryMatches<'a, 'tree, &'a SourceFileSnapshot>,
}

impl<'a, 'tree> Iterator for QualifiedReferences<'a, 'tree> {
    type Item = Result<(Node<'a>, QualifiedReference), Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let match_ = self.matches.next()?;
        Some(self.parse_match(match_))
    }
}

impl<'a, 'tree> QualifiedReferences<'a, 'tree> {
    fn parse_match(
        &self,
        match_: QueryMatch<'a, 'tree>,
    ) -> Result<(Node<'a>, QualifiedReference), Error> {
        let mut qualifier_range = None;
        let mut root_node = None;
        let mut opt_name_capture = None;
        match_.captures.iter().for_each(|capture| {
            if capture.index == self.query.root {
                root_node = Some(capture.node);
            }
            if capture.index == self.query.qualifier {
                match &qualifier_range {
                    None => qualifier_range = Some(capture.node.byte_range()),
                    Some(existing_range) => {
                        qualifier_range =
                            Some(existing_range.start..capture.node.end_byte())
                    }
                }
            } else {
                opt_name_capture = Some(capture)
            }
        });
        let name_capture = opt_name_capture.ok_or_else(|| {
            log::mk_err!("match of qualified reference did not include name")
        })?;
        let qualifier_range = qualifier_range.ok_or_else(|| {
            log::mk_err!(
                "match of qualified reference did not include qualifier"
            )
        })?;
        let qualifier = self.code.slice(&qualifier_range);
        let kind = match name_capture.index {
            index if index == self.query.value => ReferenceKind::Value,
            index if index == self.query.type_ => ReferenceKind::Type,
            index if index == self.query.constructor => ReferenceKind::Constructor,
            index => {
                return Err(log::mk_err!(
                    "name in match of qualified reference has unexpected index {:?}",
                    index,
                ))
            }
        };
        let reference = Reference {
            name: self.code.slice(&name_capture.node.byte_range()).into(),
            kind,
        };
        let qualified = QualifiedReference {
            qualifier: qualifier.into(),
            reference,
        };
        Ok((
            root_node.ok_or_else(|| {
                log::mk_err!(
                    "match of qualified reference did not include root node"
                )
            })?,
            qualified,
        ))
    }
}

#[derive(PartialEq)]
struct QualifiedReference {
    qualifier: Rope,
    reference: Reference,
}

#[derive(Clone, Debug)]
struct Reference {
    name: Rope,
    kind: ReferenceKind,
}

impl PartialEq for Reference {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.kind == other.kind
    }
}

impl std::hash::Hash for Reference {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.name.to_string().hash(state);
        self.kind.hash(state);
    }
}

impl Eq for Reference {}

#[derive(PartialEq, Clone, Copy, Debug, Hash)]
enum ReferenceKind {
    Value,
    Type,
    Constructor,
    Operator,
}

impl Eq for ReferenceKind {}

query::query!(
    QueryForUnqualifiedValues,
    query_for_unqualified_values,
    "./queries/unqualified_values",
    value,
    type_,
    constructor,
);

impl QueryForUnqualifiedValues {
    fn run<'a, 'tree>(
        &'a self,
        cursor: &'a mut QueryCursor,
        code: &'tree SourceFileSnapshot,
    ) -> UnqualifiedValues<'a, 'tree> {
        self.run_in(cursor, code, code.tree.root_node())
    }

    fn run_in<'a, 'tree>(
        &'a self,
        cursor: &'a mut QueryCursor,
        code: &'tree SourceFileSnapshot,
        node: Node<'tree>,
    ) -> UnqualifiedValues<'a, 'tree> {
        let matches = cursor.matches(&self.query, node, code);
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
    query: &'a QueryForUnqualifiedValues,
}

impl<'a, 'tree> Iterator for UnqualifiedValues<'a, 'tree> {
    type Item = Result<(Node<'a>, Reference), Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let match_ = self.matches.next()?;
        let capture = match_.captures.first()?;
        let kind = match capture.index {
            index if index == self.query.value => ReferenceKind::Value,
            index if index == self.query.type_ => ReferenceKind::Type,
            index if index == self.query.constructor => ReferenceKind::Constructor,
            index => {
                return Some(Err(log::mk_err!(
                    "query for unqualified values captured name with unexpected index {:?}",
                    index
                )))
            }
        };
        let node = capture.node;
        let name = self.code.slice(&node.byte_range());
        let reference = Reference {
            name: name.into(),
            kind,
        };
        Some(Ok((node, reference)))
    }
}

query::query!(
    QueryForImports,
    query_for_imports,
    "./queries/imports",
    root,
    name,
    as_clause,
    exposing_list
);

impl QueryForImports {
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
    query: &'a QueryForImports,
}

impl<'a, 'tree> Iterator for Imports<'a, 'tree> {
    type Item = Import<'tree>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut nodes: [Option<Node>; 4] = [None; 4];
        self.matches.next()?.captures.iter().for_each(|capture| {
            nodes[capture.index as usize] = Some(capture.node)
        });
        Some(Import {
            code: self.code,
            root_node: nodes[self.query.root as usize]?,
            name_node: nodes[self.query.name as usize]?,
            as_clause_node: nodes[self.query.as_clause as usize],
            exposing_list_node: nodes[self.query.exposing_list as usize],
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
    fn unaliased_name(&self) -> RopeSlice {
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
    type Item = Result<(Node<'a>, Exposed<'a>), Error>;

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
                    EXPOSED_TYPE => {
                        let type_name_node = match node.child(0) {
                            Some(node) => node,
                            None => {
                                return Some(Err(log::mk_err!(
                                    "did not find name node for type in exposing list"
                                )));
                            }
                        };
                        Exposed::Type(ExposedType {
                            name: self.code.slice(&type_name_node.byte_range()),
                            exposing_constructors: node.child(1).is_some(),
                            buffer: self.code.buffer,
                            module_name: self.module_name,
                        })
                    }
                    DOUBLE_DOT => Exposed::All,
                    _ => {
                        return Some(Err(log::mk_err!(
                            "capture in query for exposing list has unexpected kind {:?}",
                            node.kind()
                        )))
                    }
                };
                return Some(Ok((node, exposed)));
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
    All,
}

impl<'a> Exposed<'a> {
    fn for_each_reference<F>(&self, import: &ElmModule, mut f: F)
    where
        F: FnMut(Reference),
    {
        match self {
            Exposed::Value(val) => f(Reference {
                kind: ReferenceKind::Value,
                name: val.name.into(),
            }),
            Exposed::Type(type_) => {
                f(Reference {
                    kind: ReferenceKind::Type,
                    name: type_.name.into(),
                });
                import.exports.iter().for_each(|export| match export {
                    ElmExport::Value { .. } => {}
                    ElmExport::RecordTypeAlias { name } => {
                        if name == &type_.name {
                            f(Reference {
                                kind: ReferenceKind::Constructor,
                                name: Rope::from_str(name),
                            });
                        }
                    }
                    ElmExport::Type { name, constructors } => {
                        if type_.exposing_constructors && name == &type_.name {
                            for ctor in constructors.iter() {
                                f(Reference {
                                    kind: ReferenceKind::Constructor,
                                    name: Rope::from_str(ctor),
                                })
                            }
                        }
                    }
                });
            }
            Exposed::All => {
                import.exports.iter().for_each(|export| match export {
                    ElmExport::Value { name } => f(Reference {
                        kind: ReferenceKind::Value,
                        name: Rope::from_str(name),
                    }),
                    ElmExport::RecordTypeAlias { name } => {
                        f(Reference {
                            kind: ReferenceKind::Value,
                            name: Rope::from_str(name),
                        });
                        f(Reference {
                            kind: ReferenceKind::Type,
                            name: Rope::from_str(name),
                        });
                    }
                    ElmExport::Type { name, constructors } => {
                        f(Reference {
                            kind: ReferenceKind::Type,
                            name: Rope::from_str(name),
                        });
                        for ctor in constructors.iter() {
                            f(Reference {
                                kind: ReferenceKind::Constructor,
                                name: Rope::from_str(ctor),
                            });
                        }
                    }
                });
            }
            Exposed::Operator(op) => f(Reference {
                kind: ReferenceKind::Operator,
                name: op.name.into(),
            }),
        }
    }
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
    ) -> Result<ExposedConstructors, Error> {
        for export in engine.module_exports(self.buffer, self.module_name)? {
            match export {
                ElmExport::Value { .. } => {}
                ElmExport::RecordTypeAlias { name } => {
                    return Ok(ExposedConstructors::FromTypeAlias(name));
                }
                ElmExport::Type { name, constructors } => {
                    if self.name.eq(name) {
                        return Ok(ExposedConstructors::FromCustomType(
                            constructors,
                        ));
                    }
                }
            }
        }
        Err(log::mk_err!("did not find type in module"))
    }
}

#[derive(Clone)]
enum ExposedConstructors<'a> {
    FromTypeAlias(&'a String),
    FromCustomType(&'a Vec<String>),
}

pub(crate) fn project_root_for_path(path: &Path) -> Result<&Path, Error> {
    let mut maybe_root = path;
    loop {
        if maybe_root.join("elm.json").exists() {
            return Ok(maybe_root);
        } else {
            match maybe_root.parent() {
                None => {
                    return Err(log::mk_err!(
                        "Did not find elm.json file in any ancestor directory of module path"
                    ));
                }
                Some(parent) => {
                    maybe_root = parent;
                }
            }
        }
    }
}

fn parse_import_node<'a>(
    engine: &'a RefactorEngine,
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

#[cfg(test)]
mod simulations {
    use crate::analysis_thread::{diff_trees, SourceFileDiff};
    use crate::elm::RefactorEngine;
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
            Err(Error::ElmPair(err)) => {
                eprintln!("{:?}", err);
                panic!();
            }
            Err(Error::RunningSimulation(err)) => {
                eprintln!("{:?}", err);
                panic!();
            }
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
        let mut diff = SourceFileDiff { old, new };
        let tree_changes = diff_trees(&diff);
        let mut refactor_engine = RefactorEngine::new()?;
        refactor_engine.init_buffer(buffer, path.to_owned(), &mut |_| {})?;
        let edits = refactor_engine
            .respond_to_change(&diff, tree_changes)?
            .edits(&mut diff.new)?;
        if edits.is_empty() || diff.old.bytes == diff.new.bytes {
            Ok("No refactor for this change.".to_owned())
        } else if diff.new.tree.root_node().has_error() {
            Ok(format!(
                "Refactor produced invalid code:\n{}",
                diff.new.bytes.to_string()
            ))
        } else {
            Ok(diff.new.bytes.to_string())
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
    simulation_test!(
        add_module_qualifier_to_record_type_alias_in_type_declaration
    );
    simulation_test!(
        add_module_qualifier_to_record_type_alias_used_as_constructor
    );
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
    simulation_test!(remove_record_type_alias_from_exposing_list_of_import);
    simulation_test!(add_record_type_alias_to_exposing_list_of_import);

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
    simulation_test!(
        remove_module_qualifier_from_record_type_alias_used_as_function
    );
    simulation_test!(add_value_to_exposing_list);
    simulation_test!(add_type_to_exposing_list);
    simulation_test!(add_constructors_for_type_to_exposing_list);
    simulation_test!(add_type_exposing_constructors_to_exposing_list);
    simulation_test!(add_record_type_alias_with_same_name_as_local_constructor_to_exposing_list_of_import);
    simulation_test!(add_non_record_type_alias_with_same_name_as_local_constructor_to_exposing_list_of_import);
    simulation_test!(add_exposing_list);
    simulation_test!(add_exposing_all_list);
    simulation_test!(add_and_remove_items_in_exposing_list);
    simulation_test!(replace_exposing_list_with_double_dot);
    simulation_test!(replace_double_dot_with_exposing_list);
    simulation_test!(
        add_value_to_exposing_list_with_same_name_as_local_variable
    );
    simulation_test!(
        add_value_to_exposing_list_with_same_name_as_top_level_function
    );
    simulation_test!(
        remove_module_qualifier_from_variable_with_same_name_as_local_variable
    );
    simulation_test!(
        expose_value_with_same_name_as_exposed_value_from_other_module
    );
    simulation_test!(
        expose_value_with_same_name_as_value_from_other_module_exposing_all
    );
    simulation_test!(
        remove_module_qualifier_from_variable_with_same_name_as_value_exposed_from_other_module
    );
    simulation_test!(
        remove_module_qualifier_from_type_with_same_name_as_local_type_alias
    );
    simulation_test!(
        remove_module_qualifier_from_type_with_same_name_as_local_type
    );
    simulation_test!(remove_module_qualifier_from_constructor_with_same_name_as_local_constructor);

    // Changing as-clauses
    simulation_test!(add_as_clause_to_import);
    simulation_test!(change_as_clause_of_import);
    simulation_test!(remove_as_clause_from_import);
    simulation_test!(change_module_qualifier_of_value);
    simulation_test!(change_module_qualifier_of_type);
    simulation_test!(change_module_qualifier_of_constructor);
    simulation_test!(
        change_module_qualifier_of_variable_from_unaliased_import_name
    );
    simulation_test!(change_module_qualifier_to_match_unaliased_import_name);
    simulation_test!(change_module_qualifier_to_invalid_name);

    // Adding import statements
    simulation_test!(use_qualifier_of_unimported_module_in_new_code);
    simulation_test!(use_qualifier_of_non_existing_module_in_new_code);
    simulation_test!(use_qualifier_of_implicitly_imported_module_in_new_code);
    simulation_test!(use_qualifier_of_unimported_module_while_in_the_middle_of_writing_identifier);

    // --- TESTS DEMONSTRATING CURRENT BUGS ---

    // The exposing lists in these tests contained an operator. It doesn't get a
    // qualifier because Elm doesn't allow qualified operators, and as a result
    // this refactor doesn't produce compiling code.
    // Potential fix: Add the exposing list back containing just the operator.
    simulation_test!(remove_exposing_clause_containing_operator_from_import);
    simulation_test!(
        remove_exposing_all_clause_containing_operator_from_import
    );
    // When we expose a value with the same name as a local variable the local
    // variable gets renamed to something else. This test demonstrates an edge
    // case in this logic where the renaming logic is failing. When we expose
    // multiple variables at the same time, one of which has the same name as
    // a local variable and the other which has the name we would rename the
    // local variable too, then we still end up with a naming conflict when all
    // is done.
    simulation_test!(
        add_value_to_exposing_list_of_import_with_same_name_as_local_variable_and_another_with_the_same_name_plus_trailing_2
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
