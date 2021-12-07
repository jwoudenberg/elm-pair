use crate::compilation_thread;
use crate::editor_listener_thread;
use crate::{
    byte_to_point, debug_code_slice, Buffer, Edit, MVar, MsgLoop,
    SourceFileSnapshot,
};
use std::collections::HashMap;
use std::sync::mpsc::Receiver;
use tree_sitter::{Node, TreeCursor};

pub(crate) mod elm;

pub(crate) enum Msg {
    SourceCodeModified,
    ThreadFailed(Error),
    EditorConnected(u32, Box<dyn EditorDriver>),
    EditorDisconnected(u32),
    CompilationSucceeded(SourceFileSnapshot),
}

impl From<editor_listener_thread::Error> for Msg {
    fn from(err: editor_listener_thread::Error) -> Msg {
        Msg::ThreadFailed(Error::EditorListenerThreadFailed(err))
    }
}

impl From<compilation_thread::Error> for Msg {
    fn from(err: compilation_thread::Error) -> Msg {
        Msg::ThreadFailed(Error::CompilationThreadFailed(err))
    }
}

#[derive(Debug)]
pub(crate) enum Error {
    EditorListenerThreadFailed(editor_listener_thread::Error),
    CompilationThreadFailed(compilation_thread::Error),
    InvalidQuery(tree_sitter::QueryError),
}

pub(crate) fn run(
    latest_code: &MVar<SourceFileSnapshot>,
    analysis_receiver: Receiver<Msg>,
) -> Result<(), Error>
where
{
    AnalysisLoop {
        latest_code,
        last_compiling_code: HashMap::new(),
        editor_driver: HashMap::new(),
        refactor_engine: elm::RefactorEngine::new()?,
    }
    .start(analysis_receiver)
}

struct AnalysisLoop<'a> {
    latest_code: &'a MVar<SourceFileSnapshot>,
    last_compiling_code: HashMap<Buffer, SourceFileSnapshot>,
    editor_driver: HashMap<u32, Box<dyn EditorDriver>>,
    refactor_engine: elm::RefactorEngine,
}

impl<'a> MsgLoop<Error> for AnalysisLoop<'a> {
    type Msg = Msg;

    fn on_idle(&mut self) -> Result<(), Error> {
        if let Some((diff, editor_driver)) = self.source_file_diff() {
            eprintln!(
                "[info] diffing revision {:?} against {:?} for buffer {:?}",
                diff.new.revision, diff.old.revision, diff.old.buffer
            );
            if let Some(elm_change) = analyze_diff(&diff) {
                match self.refactor_engine.respond_to_change(&diff, elm_change)
                {
                    Ok(refactor) => {
                        eprintln!("[info] applying refactor to editor");
                        editor_driver.apply_edits(refactor);
                    }
                    Err(err) => {
                        eprintln!("[warn] failed to create refactor: {:?}", err)
                    }
                }
            }
        };
        Ok(())
    }

    fn on_msg(&mut self, msg: Msg) -> Result<bool, Error> {
        match msg {
            Msg::SourceCodeModified => {}
            Msg::ThreadFailed(err) => return Err(err),
            Msg::EditorConnected(editor_id, editor_driver) => {
                self.editor_driver.insert(editor_id, editor_driver);
            }
            Msg::EditorDisconnected(editor_id) => {
                self.editor_driver.remove(&editor_id);
                self.last_compiling_code
                    .retain(|buffer, _| buffer.editor_id != editor_id);
                if self.editor_driver.is_empty() {
                    return Ok(false);
                }
            }
            Msg::CompilationSucceeded(snapshot) => {
                if self.editor_driver.contains_key(&snapshot.buffer.editor_id) {
                    self.last_compiling_code.insert(snapshot.buffer, snapshot);
                }
            }
        }
        Ok(true)
    }
}

impl<'a> AnalysisLoop<'a> {
    fn source_file_diff(&self) -> Option<(SourceFileDiff, &dyn EditorDriver)> {
        let new = self.latest_code.try_read()?;
        let old = self.last_compiling_code.get(&new.buffer)?.clone();
        if new.revision <= old.revision {
            return None;
        }
        let editor_driver = self.editor_driver.get(&new.buffer.editor_id)?;
        let diff = SourceFileDiff { old, new };
        Some((diff, editor_driver.as_ref()))
    }
}

// An API for sending commands to an editor. This is defined as a trait to
// support different kinds of editors.
pub(crate) trait EditorDriver: 'static + Send {
    fn apply_edits(&self, edits: Vec<Edit>) -> bool;
}

pub(crate) struct SourceFileDiff {
    pub old: SourceFileSnapshot,
    pub new: SourceFileSnapshot,
}

pub(crate) fn analyze_diff(diff: &SourceFileDiff) -> Option<elm::ElmChange> {
    let tree_changes = diff_trees(diff);
    elm::interpret_change(&tree_changes)
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
