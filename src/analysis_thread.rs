use crate::{
    byte_to_point, debug_code_slice, Edit, EditorDriver, Error, MVar, MsgLoop,
    SourceFileSnapshot,
};
use core::ops::Range;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use tree_sitter::{Node, Query, QueryCursor, TreeCursor};

pub(crate) enum Msg {
    SourceCodeModified,
    ThreadFailed(Error),
    AllEditorsDisconnected,
    EditorConnected(Box<dyn EditorDriver>),
    CompilationSucceeded(SourceFileSnapshot),
}

pub(crate) fn run(
    latest_code: Arc<MVar<SourceFileSnapshot>>,
    analysis_receiver: Receiver<Msg>,
) -> Result<(), Error>
where
{
    AnalysisLoop {
        latest_code,
        last_compiling_code: None,
        editor_driver: None,
    }
    .start(analysis_receiver)
}

struct AnalysisLoop {
    latest_code: Arc<MVar<SourceFileSnapshot>>,
    last_compiling_code: Option<SourceFileSnapshot>,
    editor_driver: Option<Box<dyn EditorDriver>>,
}

impl MsgLoop<Error> for AnalysisLoop {
    type Msg = Msg;

    fn on_idle(&mut self) -> Result<(), Error> {
        if let (Some(old), Some(new), Some(editor_driver)) = (
            self.last_compiling_code.clone(),
            self.latest_code.try_read(),
            &self.editor_driver,
        ) {
            let diff = SourceFileDiff { old, new };
            if let Some(change) = analyze_diff(&diff) {
                let refactor = elm_refactor(&diff, &change);
                editor_driver.apply_edits(refactor)?;
            }
        };
        Ok(())
    }

    fn on_msg(&mut self, msg: Msg) -> Result<bool, Error> {
        match msg {
            Msg::SourceCodeModified => {}
            Msg::ThreadFailed(err) => return Err(err),
            Msg::AllEditorsDisconnected => return Ok(false),
            Msg::EditorConnected(editor_driver) => {
                self.editor_driver = Some(editor_driver);
            }
            Msg::CompilationSucceeded(snapshot) => {
                self.last_compiling_code = Some(snapshot);
            }
        }
        Ok(true)
    }
}

pub struct SourceFileDiff {
    old: SourceFileSnapshot,
    new: SourceFileSnapshot,
}

pub fn analyze_diff(diff: &SourceFileDiff) -> Option<ElmChange> {
    let tree_changes = diff_trees(diff);
    interpret_change(&tree_changes)
}

// TODO: use kind ID's instead of names for pattern matching.
fn interpret_change(changes: &TreeChanges) -> Option<ElmChange> {
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

#[derive(Debug)]
pub enum ElmChange {
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

struct TreeChanges<'a> {
    old_code: &'a SourceFileSnapshot,
    new_code: &'a SourceFileSnapshot,
    old_removed: Vec<Node<'a>>,
    new_added: Vec<Node<'a>>,
}

fn diff_trees(diff: &SourceFileDiff) -> TreeChanges<'_> {
    let old_code = &diff.old;
    let new_code = &diff.new;
    let mut old = diff.old.tree.walk();
    let mut new = diff.new.tree.walk();
    loop {
        match goto_first_changed_sibling(old_code, new_code, &mut old, &mut new)
        {
            FirstChangedSibling::NoneFound => {
                return TreeChanges {
                    old_code,
                    new_code,
                    old_removed: Vec::new(),
                    new_added: Vec::new(),
                }
            }
            FirstChangedSibling::OldAtFirstAdditional => {
                return TreeChanges {
                    old_code,
                    new_code,
                    old_removed: collect_remaining_siblings(old),
                    new_added: Vec::new(),
                }
            }
            FirstChangedSibling::NewAtFirstAdditional => {
                return TreeChanges {
                    old_code,
                    new_code,
                    old_removed: Vec::new(),
                    new_added: collect_remaining_siblings(new),
                }
            }
            FirstChangedSibling::OldAndNewAtFirstChanged => {}
        };
        let first_old_changed = old.node();
        let first_new_changed = new.node();
        let (old_removed_count, new_added_count) =
            count_changed_siblings(old_code, new_code, &old, &new);

        // If only a single sibling changed and it's kind remained the same,
        // then we descend into that child.
        if old_removed_count == 1
            && new_added_count == 1
            && first_old_changed.kind_id() == first_new_changed.kind_id()
            && first_old_changed.child_count() > 0
            && first_new_changed.child_count() > 0
        {
            old.goto_first_child();
            new.goto_first_child();
            continue;
        }

        let mut old_removed = Vec::with_capacity(old_removed_count);
        while old_removed.len() < old_removed_count {
            old_removed.push(old.node());
            old.goto_next_sibling();
        }

        let mut new_added = Vec::with_capacity(new_added_count);
        while new_added.len() < new_added_count {
            new_added.push(new.node());
            new.goto_next_sibling();
        }

        return TreeChanges {
            old_code,
            new_code,
            old_removed,
            new_added,
        };
    }
}

// This type solely exists to list the potential results of calling
// `goto_first_changed_sibling`. The comments below each show one example of
// a set of old and new nodes that would lead to that particular value being
// returned. The arrows indicate which nodes the TreeCursors will end up
// pointing at.
enum FirstChangedSibling {
    //            v
    // old: [ a b c ]
    // new: [ a b c ]
    //            ^
    NoneFound,
    //            v
    // old: [ a b c d e ]
    // new: [ a b x y ]
    //            ^
    OldAndNewAtFirstChanged,
    //            v
    // old: [ a b c d e ]
    // new: [ a b ]
    //          ^
    OldAtFirstAdditional,
    //          v
    // old: [ a b ]
    // new: [ a b c d e ]
    //            ^
    NewAtFirstAdditional,
}

// Move both cursors forward through sibbling nodes in lock step, stopping when
// we encounter a difference between the old and new node.
fn goto_first_changed_sibling(
    old_code: &SourceFileSnapshot,
    new_code: &SourceFileSnapshot,
    old: &mut TreeCursor,
    new: &mut TreeCursor,
) -> FirstChangedSibling {
    loop {
        if has_node_changed(old_code, new_code, &old.node(), &new.node()) {
            return FirstChangedSibling::OldAndNewAtFirstChanged;
        } else {
            match (old.goto_next_sibling(), new.goto_next_sibling()) {
                (true, true) => continue,
                (false, false) => return FirstChangedSibling::NoneFound,
                (true, false) => {
                    return FirstChangedSibling::OldAtFirstAdditional
                }
                (false, true) => {
                    return FirstChangedSibling::NewAtFirstAdditional
                }
            }
        }
    }
}

fn collect_remaining_siblings(mut cursor: TreeCursor) -> Vec<Node> {
    let mut acc = vec![cursor.node()];
    while cursor.goto_next_sibling() {
        acc.push(cursor.node());
    }
    acc
}

// Find how many old siblings were replaced with how many new ones. For example,
// given the following examples:
//
//     old: [ a b c d e f g h ]
//            |         | | |
//     new: [ a x y     f g h ]
//
// This function would return `(4, 2)` when passed the old and new sibling nodes
// of this example, because 4 of the old sibling nodes were replaced with 2 new
// ones.
//
// We go about finding these counts by skipping to the end, then counting the
// amount of equal nodes we encounter as we move backwards from the last old
// and new node in lock step. We do this because on average it's less work to
// proof two node are the same than it is to proof they are different. By
// walking backwards we only need to proof two nodes are different ones.
fn count_changed_siblings<'a>(
    old_code: &'a SourceFileSnapshot,
    new_code: &'a SourceFileSnapshot,
    old: &'a TreeCursor,
    new: &'a TreeCursor,
) -> (usize, usize) {
    let mut old_sibling = old.node();
    let mut new_sibling = new.node();

    // We initialize the counts at 1, because we assume the node we're currenly
    // on is the first changed node.
    let mut old_siblings_removed = 1;
    let mut new_siblings_added = 1;

    // Walk forward, basically counting all remaining old and new siblings.
    loop {
        match old_sibling.next_sibling() {
            None => break,
            Some(next) => {
                old_siblings_removed += 1;
                old_sibling = next;
            }
        }
    }
    loop {
        match new_sibling.next_sibling() {
            None => break,
            Some(next) => {
                new_siblings_added += 1;
                new_sibling = next;
            }
        }
    }

    // Walk backwards again until we encounter a changed node.
    loop {
        if has_node_changed(old_code, new_code, &old_sibling, &new_sibling)
            || old_siblings_removed == 0
            || new_siblings_added == 0
        {
            break;
        }
        match (old_sibling.prev_sibling(), new_sibling.prev_sibling()) {
            (Some(next_old), Some(next_new)) => {
                old_sibling = next_old;
                new_sibling = next_new;
                old_siblings_removed -= 1;
                new_siblings_added -= 1;
            }
            (_, _) => {
                break;
            }
        }
    }

    (old_siblings_removed, new_siblings_added)
}

// Check if a node has changed. We have a couple of cheap checks that can
// confirm the node _hasn't_ changed, so we try those first.
//
// TODO: Incorporate tree-sitter's `has_changes` in here somehow, for bettter
// performance.
fn has_node_changed(
    old_code: &SourceFileSnapshot,
    new_code: &SourceFileSnapshot,
    old: &Node,
    new: &Node,
) -> bool {
    old.id() != new.id()
        && (old.kind_id() != new.kind_id()
            || have_node_contents_changed(old_code, new_code, old, new))
}

// Compare two nodes by comparing snippets of code covered by them. This is
// supposed to be a 100% accurate, albeit potentially slower equivalency check.
//
// TODO: code formatters can change code in ways that don't matter but would
// fail this check. Consider alternative approaches.
fn have_node_contents_changed(
    old_code: &SourceFileSnapshot,
    new_code: &SourceFileSnapshot,
    old: &Node,
    new: &Node,
) -> bool {
    let old_bytes = debug_code_slice(old_code, &old.byte_range());
    let new_bytes = debug_code_slice(new_code, &new.byte_range());
    old_bytes != new_bytes
}

fn elm_refactor(diff: &SourceFileDiff, change: &ElmChange) -> Vec<Edit> {
    let query_str = r#"
[(import_clause
    exposing:
      (exposing_list
        [
          (double_dot)
          (exposed_value)
          (exposed_type)
          (exposed_operator)
        ] @exposed_value
      )
 ) @import
]"#;
    let query = Query::new(diff.new.tree.language(), query_str).unwrap();
    match change {
        ElmChange::QualifierAdded(name, qualifier) => {
            // debug_print_tree(&diff.new);
            let mut cursor = QueryCursor::new();
            let exposed = cursor
                .matches(&query, diff.new.tree.root_node(), |node| {
                    debug_code_slice(&diff.new, &node.byte_range())
                })
                .find_map(|m| {
                    let (import, exposed_val) = match m.captures {
                        [x, y] => (x, y),
                        _ => panic!("wrong number of capures"),
                    };
                    let import_node =
                        import.node.child_by_field_name("moduleName")?;
                    let import_name =
                        debug_code_slice(&diff.new, &import_node.byte_range());
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
                .unwrap();
            let range = || {
                let next = exposed.next_sibling();
                if let Some(node) = next {
                    if node.kind() == "," {
                        return exposed.start_byte()..node.end_byte();
                    }
                }
                let prev = exposed.prev_sibling();
                if let Some(node) = prev {
                    if node.kind() == "," {
                        return node.start_byte()..exposed.end_byte();
                    }
                }
                exposed.byte_range()
            };
            vec![mk_edit(&diff.new, &range(), String::new())]
        }
        _ => Vec::new(),
    }
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
