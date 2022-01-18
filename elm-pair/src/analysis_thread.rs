use crate::elm;
use crate::elm::compiler::Compiler;
use crate::support::log;
use crate::support::source_code::{Buffer, Edit, SourceFileSnapshot};
use crate::{Error, MVar, MsgLoop};
use std::collections::hash_map;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use tree_sitter::{Node, TreeCursor};

pub enum Msg {
    SourceCodeModified,
    ThreadFailed(Error),
    EditorConnected(u32, Box<dyn EditorDriver>),
    EditorDisconnected(u32),
    OpenedNewSourceFile { buffer: Buffer, path: PathBuf },
    CompilationSucceeded(SourceFileSnapshot),
}

impl From<Error> for Msg {
    fn from(err: Error) -> Msg {
        Msg::ThreadFailed(err)
    }
}

pub fn run(
    latest_code: &MVar<SourceFileSnapshot>,
    analysis_receiver: Receiver<Msg>,
    compiler: Compiler,
) -> Result<(), Error> {
    AnalysisLoop {
        latest_code,
        last_compiling_code: HashMap::new(),
        editor_driver: HashMap::new(),
        refactor_engine: elm::RefactorEngine::new(compiler)?,
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
        if let Some(mut diff) = self.source_file_diff() {
            let AnalysisLoop {
                editor_driver,
                refactor_engine,
                ..
            } = self;
            if let Some(editor_driver) =
                editor_driver.get(&diff.new.buffer.editor_id)
            {
                log::info!(
                    "diffing revision {:?} against {:?} for buffer {:?}",
                    diff.new.revision,
                    diff.old.revision,
                    diff.old.buffer,
                );
                let tree_changes = diff_trees(&diff);
                let current_revision = diff.new.revision;
                let result = refactor_engine
                    .respond_to_change(&diff, tree_changes)
                    .and_then(|refactor| refactor.edits(&mut diff.new));
                match result {
                    Ok(edits) if !diff.new.tree.root_node().has_error() => {
                        if !edits.is_empty() {
                            log::info!("applying refactor to editor");
                            if editor_driver.apply_edits(edits) {
                                // Increment the revision by one compared to the
                                // unrefactored code. Code revisions coming from
                                // the editor are all even numbers, so the
                                // revisions created by refactors will be odd.
                                // This is intended to help debugging.
                                // The next revision coming from the editor,
                                // being the next even number, will take
                                // precendence over this one.
                                diff.new.revision = 1 + current_revision;

                                // Set the refactored code as the 'last
                                // compiling version'. We're assuming here that
                                // the refactor got applied in the editor
                                // successfully. If we don't do this elm-pair
                                // keeps comparing new changes to the old last
                                // compiling version, until the editor
                                // communicates the changes made by the refactor
                                // back to us _and_ the compilation thread
                                // compiles that version (which may be never).
                                self.last_compiling_code
                                    .insert(diff.new.buffer, diff.new);
                            }
                        }
                    }
                    Ok(_) => {
                        log::error!("refactor produced invalid code")
                    }
                    Err(err) => {
                        log::error!("failed to create refactor: {:?}", err)
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
            Msg::OpenedNewSourceFile { buffer, path } => {
                let AnalysisLoop {
                    refactor_engine, ..
                } = self;
                // TODO: We error here if elm-stuff/i.dat is missing. Figure out
                // something that won't bring the application down in this case.
                refactor_engine.init_buffer(buffer, path)?;
            }
            Msg::CompilationSucceeded(snapshot) => {
                // Replace 'last compiling version' with a newer revision only.
                // When we set the 'last compiling version' to the product of a
                // refactor then it might take a bit of time for the compilation
                // thread to catch up.
                if self.editor_driver.contains_key(&snapshot.buffer.editor_id) {
                    match self.last_compiling_code.entry(snapshot.buffer) {
                        hash_map::Entry::Vacant(vac) => {
                            vac.insert(snapshot);
                        }
                        hash_map::Entry::Occupied(mut occ) => {
                            let current = occ.get_mut();
                            if snapshot.revision >= current.revision {
                                *current = snapshot;
                            }
                        }
                    };
                }
            }
        }
        Ok(true)
    }
}

impl<'a> AnalysisLoop<'a> {
    fn source_file_diff(&self) -> Option<SourceFileDiff> {
        let new = self.latest_code.try_read()?;
        let old = self.last_compiling_code.get(&new.buffer)?.clone();
        if new.revision <= old.revision {
            return None;
        }
        let diff = SourceFileDiff { old, new };
        Some(diff)
    }
}

// An API for sending commands to an editor. This is defined as a trait to
// support different kinds of editors.
pub trait EditorDriver: 'static + Send {
    fn apply_edits(&self, edits: Vec<Edit>) -> bool;
}

pub struct SourceFileDiff {
    pub old: SourceFileSnapshot,
    pub new: SourceFileSnapshot,
}

pub struct TreeChanges<'a> {
    pub old_removed: Vec<Node<'a>>,
    pub old_parent: Node<'a>,
    pub new_added: Vec<Node<'a>>,
    pub new_parent: Node<'a>,
}

pub fn diff_trees(diff: &SourceFileDiff) -> TreeChanges<'_> {
    let old_code = &diff.old;
    let new_code = &diff.new;
    let mut old = diff.old.tree.walk();
    let mut new = diff.new.tree.walk();
    let mut old_parent = old.node();
    let mut new_parent = new.node();
    loop {
        match goto_first_changed_sibling(old_code, new_code, &mut old, &mut new)
        {
            FirstChangedSibling::NoneFound => {
                return TreeChanges {
                    old_parent,
                    new_parent,
                    old_removed: Vec::new(),
                    new_added: Vec::new(),
                }
            }
            FirstChangedSibling::OldAtFirstAdditional => {
                return TreeChanges {
                    old_parent,
                    new_parent,
                    old_removed: collect_remaining_siblings(old),
                    new_added: Vec::new(),
                }
            }
            FirstChangedSibling::NewAtFirstAdditional => {
                return TreeChanges {
                    old_parent,
                    new_parent,
                    old_removed: Vec::new(),
                    new_added: collect_remaining_siblings(new),
                }
            }
            FirstChangedSibling::OldAndNewAtFirstChanged => {}
        };
        let first_old_changed = old.node();
        let first_new_changed = new.node();
        let (old_removed_count, new_added_count) = count_changed_siblings(
            old_code,
            new_code,
            &mut old.clone(),
            &mut new.clone(),
        );

        // If only a single sibling changed and it's kind remained the same,
        // then we descend into that child.
        if old_removed_count == 1
            && new_added_count == 1
            && first_old_changed.kind_id() == first_new_changed.kind_id()
            && first_old_changed.child_count() > 0
            && first_new_changed.child_count() > 0
        {
            old_parent = old.node();
            new_parent = new.node();
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
            old_parent,
            new_parent,
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
    old: &'a mut TreeCursor,
    new: &'a mut TreeCursor,
) -> (usize, usize) {
    // We initialize the counts at 1, because we assume the node we're currenly
    // on is the first changed node.
    let mut old_siblings_removed = 1;
    let mut new_siblings_added = 1;

    // Walk forward, basically counting all remaining old and new siblings.
    loop {
        if old.goto_next_sibling() {
            old_siblings_removed += 1;
        } else {
            break;
        }
    }
    loop {
        if new.goto_next_sibling() {
            new_siblings_added += 1;
        } else {
            break;
        }
    }

    // Walk backwards again until we encounter a changed node.
    let mut old_sibling = old.node();
    let mut new_sibling = new.node();
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
    let old_bytes = old_code.slice(&old.byte_range());
    let new_bytes = new_code.slice(&new.byte_range());
    old_bytes != new_bytes
}
