use crate::editors;
use crate::elm;
use crate::elm::compiler::Compiler;
use crate::lib::log;
use crate::lib::source_code::{
    Buffer, Edit, RefactorAllowed, SourceFileSnapshot,
};
use crate::{Error, MsgLoop};
use std::collections::hash_map;
use std::collections::HashMap;
use std::iter::FromIterator;
use std::path::PathBuf;
use tree_sitter::{Node, TreeCursor};

pub enum Msg {
    SourceCodeModified {
        code: SourceFileSnapshot,
        refactor: RefactorAllowed,
    },
    ThreadFailed(Error),
    EditorConnected(editors::Id, Box<dyn editors::Driver>),
    EditorDisconnected(editors::Id),
    OpenedNewSourceFile {
        path: PathBuf,
        code: SourceFileSnapshot,
    },
    CompilationSucceeded(SourceFileSnapshot),
}

impl From<Error> for Msg {
    fn from(err: Error) -> Msg {
        Msg::ThreadFailed(err)
    }
}

pub fn create(compiler: Compiler) -> Result<AnalysisLoop, Error> {
    let analysis_loop = AnalysisLoop {
        buffers: HashMap::new(),
        buffers_by_path: HashMap::new(),
        last_change: None,
        last_compiling_code: HashMap::new(),
        editor_driver: HashMap::new(),
        refactor_engine: elm::RefactorEngine::new(compiler)?,
        previous_refactors: Vec::new(),
    };
    Ok(analysis_loop)
}

pub struct AnalysisLoop {
    buffers: HashMap<Buffer, SourceFileSnapshot>,
    buffers_by_path: HashMap<(editors::Id, PathBuf), Buffer>,
    last_change: Option<(Buffer, RefactorAllowed)>,
    last_compiling_code: HashMap<Buffer, SourceFileSnapshot>,
    editor_driver: HashMap<editors::Id, Box<dyn editors::Driver>>,
    refactor_engine: elm::RefactorEngine,
    previous_refactors: Vec<Vec<Edit>>,
}

impl MsgLoop for AnalysisLoop {
    type Msg = Msg;
    type Err = Error;

    fn on_idle(&mut self) -> Result<(), Error> {
        if let Some(diff) = self.source_file_diff() {
            log::info!(
                "diffing revision {:?} against {:?} for buffer {:?}",
                diff.new.revision,
                diff.old.revision,
                diff.old.buffer,
            );
            let tree_changes = diff_trees(&diff);
            if tree_changes.old_removed.is_empty()
                && tree_changes.new_added.is_empty()
            {
                // No changes were detected!
                return Ok(());
            }
            let editor_driver =
                match self.editor_driver.get(&diff.new.buffer.editor_id) {
                    Some(driver) => driver,
                    None => {
                        return Ok(());
                    }
                };
            let res_refactor = self.refactor_engine.respond_to_change(
                &diff,
                tree_changes,
                &self.buffers,
                &self.buffers_by_path,
            );
            let refactor = match res_refactor {
                Ok(refactor_) => refactor_,
                Err(err) => {
                    log::error!("failed to create refactor: {:?}", err);
                    return Ok(());
                }
            };
            let changed_buffers = refactor.changed_buffers();
            let mut refactored_code = HashMap::from_iter(
                self.buffers.iter().filter_map(|(buffer, code)| {
                    if changed_buffers.contains(buffer) {
                        Some((*buffer, code.clone()))
                    } else {
                        None
                    }
                }),
            );
            let refactor_description = refactor.description;
            let result = refactor.edits(&mut refactored_code);
            match result {
                Ok((edits, files_to_open)) => {
                    if !files_to_open.is_empty() {
                        log::info!(
                            "open {} files in preparation of refactor: {}",
                            changed_buffers.len(),
                            refactor_description
                        );
                        editor_driver.open_files(files_to_open);
                        return Ok(());
                    }

                    if edits.is_empty() {
                        return Ok(());
                    }

                    if refactored_code
                        .values()
                        .any(|code| code.tree.root_node().has_error())
                    {
                        log::error!("refactor produced invalid code");
                        return Ok(());
                    }

                    // If we recently performed the exact same refactor we might
                    // be in a loop. This can happen when the programmer undoes
                    // a refactor that introduced a single change with that undo
                    // triggering a new refactor.
                    if self.previous_refactors.contains(&edits) {
                        log::info!("redo of recent refactor aborted");
                        self.previous_refactors = Vec::new();
                        return Ok(());
                    }

                    log::info!(
                        "edit {} buffers to refactor: {}",
                        changed_buffers.len(),
                        refactor_description
                    );

                    if editor_driver.apply_edits(edits.clone()) {
                        for (buffer, mut code) in refactored_code.into_iter() {
                            // Increment the revision by one compared to the
                            // unrefactored code. Code revisions coming from
                            // the editor are all even numbers, so the
                            // revisions created by refactors will be odd.
                            // This is intended to help debugging.
                            // The next revision coming from the editor,
                            // being the next even number, will take
                            // precendence over this one.
                            code.revision = diff.new.revision + 1;

                            // Set the refactored code as the 'last
                            // compiling version'. We're assuming here that
                            // the refactor got applied in the editor
                            // successfully. If we don't do this elm-pair
                            // keeps comparing new changes to the old last
                            // compiling version, until the editor
                            // communicates the changes made by the refactor
                            // back to us _and_ the compilation thread
                            // compiles that version (which may be never).
                            self.last_compiling_code.insert(buffer, code);
                        }

                        // Keep the last two refactors, for detecting cycles.
                        self.previous_refactors =
                            match self.previous_refactors.pop() {
                                None => vec![edits],
                                Some(prev) => vec![prev, edits],
                            };
                    }
                }
                Err(err) => {
                    log::error!("failed to apply refactor: {:?}", err)
                }
            }
        };
        Ok(())
    }

    fn on_msg(&mut self, msg: Msg) -> Result<bool, Error> {
        match msg {
            Msg::SourceCodeModified { code, refactor } => {
                self.last_change = Some((code.buffer, refactor));
                self.buffers.insert(code.buffer, code);
            }
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
            Msg::OpenedNewSourceFile { path, code } => {
                self.refactor_engine.init_buffer(code.buffer, &path)?;
                self.buffers_by_path
                    .insert((code.buffer.editor_id, path.clone()), code.buffer);
                self.buffers.insert(code.buffer, code);
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

impl AnalysisLoop {
    fn source_file_diff(&self) -> Option<SourceFileDiff> {
        let (buffer, refactor_allowed) = self.last_change?;
        let new = self.buffers.get(&buffer)?.clone();
        let old = self.last_compiling_code.get(&buffer)?.clone();
        if let RefactorAllowed::No = refactor_allowed {
            return None;
        }
        if new.revision <= old.revision {
            return None;
        }
        let diff = SourceFileDiff { old, new };
        Some(diff)
    }
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
