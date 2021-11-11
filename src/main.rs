use core::ops::Range;
use std::io::BufRead;
use std::path::PathBuf;
use tree_sitter::{InputEdit, Node, Tree, TreeCursor};

// TODO: remove current assumption that line breaks are a single byte (`\n`)
// TODO: check what happens if we send inclusive end-ranges to tree-sitter, i.e. 3..3 instead of 3..4 for a single-byte change.

fn main() {
    let fifo_path = "/tmp/elm-pair";
    nix::unistd::mkfifo(fifo_path, nix::sys::stat::Mode::S_IRWXU).unwrap();
    let fifo = std::fs::File::open(fifo_path).unwrap();
    handle_events(&mut std::io::BufReader::new(fifo).lines());
}

struct SourceFileState<'a> {
    // The code at the time of the last 'checkpoint' (when the code compiled).
    // When we get to a new checkpoint we should create a new SourceFileState
    // struct, hence this field is not mutable.
    checkpointed_code: &'a [u8],
    // The tree at the time of the last 'checkpoint'.
    checkpointed_tree: Tree,
    // Vec offers a '.splice()' operation we need to replace bits of the vector
    // in response to updates made in the editor. This is probably not super
    // efficient though, so we should look for a better datastructure here.
    //
    // This is the latest version of the code. It's mutable because we'll be
    // updating it frequently, in response to edit events from the editor.
    // We're curently storing this in a Vec because it offers a '.splice()'
    // function that replaces part of the vector with different contents.
    //
    // TODO: look into better data structures for splice-heavy workloads.
    latest_code: &'a mut Vec<u8>,
    // A tree-sitter concrete syntax tree of the latest code.
    latest_tree: Tree,
}

fn parse_event(serialized_event: &str) -> (PathBuf, String, InputEdit) {
    let (
        path,
        changed_code,
        start_byte,
        old_end_byte,
        new_end_byte,
        start_row,
        start_col,
        old_end_row,
        old_end_col,
        new_end_row,
        new_end_col,
    ) = serde_json::from_str(serialized_event).unwrap();
    let start_position = tree_sitter::Point {
        row: start_row,
        column: start_col,
    };
    let old_end_position = tree_sitter::Point {
        row: old_end_row,
        column: old_end_col,
    };
    let new_end_position = tree_sitter::Point {
        row: new_end_row,
        column: new_end_col,
    };
    let input_edit = tree_sitter::InputEdit {
        start_byte,
        old_end_byte,
        new_end_byte,
        start_position,
        old_end_position,
        new_end_position,
    };
    (path, changed_code, input_edit)
}

fn handle_events<I>(lines: &mut I)
where
    I: Iterator<Item = Result<String, std::io::Error>>,
{
    // First event returns the initial state.
    let first_line = match lines.next() {
        None => return, // We receive no events at all :(. Exit early.
        Some(line) => line,
    };
    let (_, initial_lines, _) = parse_event(&first_line.unwrap());
    let tree = parse(None, initial_lines.as_bytes()).unwrap();
    let mut state = SourceFileState {
        latest_tree: tree.clone(),
        checkpointed_code: initial_lines.as_bytes(),
        checkpointed_tree: tree,
        latest_code: &mut initial_lines.clone().into_bytes(),
    };
    print_latest_tree(&state);

    // Subsequent parses of a file.
    for line in lines {
        let (_, changed_lines, edit) = parse_event(&line.unwrap());
        handle_event(&mut state, changed_lines.into_bytes(), edit)
    }
}

fn handle_event(state: &mut SourceFileState, changed_bytes: Vec<u8>, edit: InputEdit) {
    println!("edit: {:?}", edit);
    state.latest_tree.edit(&edit);
    print_latest_tree(state);

    let range = edit.start_byte..edit.old_end_byte;
    state.latest_code.splice(range, changed_bytes);
    let parse_result = parse(Some(&state.latest_tree), state.latest_code);
    if let Some(new_tree) = parse_result {
        state.latest_tree = new_tree;
        print_latest_tree(state);
        let mut old_cursor = state.checkpointed_tree.walk();
        let mut new_cursor = state.latest_tree.walk();
        let changes = diff_trees(state, &mut old_cursor, &mut new_cursor);
        let elm_change = interpret_change(state, &changes);
        println!("CHANGE: {:?}", elm_change);
    }
}

fn interpret_change(state: &SourceFileState, changes: &TreeChanges) -> Option<ElmChange> {
    match (
        attach_kinds(&changes.old_removed).as_slice(),
        attach_kinds(&changes.new_added).as_slice(),
    ) {
        ([("lower_case_identifier", before)], [("lower_case_identifier", after)]) => {
            Some(ElmChange::RenamedVar(
                code_slice(state.checkpointed_code, &before.byte_range()),
                code_slice(state.latest_code, &after.byte_range()),
            ))
        }
        (before, after) => {
            println!("NOT-MATCH BEFORE: {:?}", before);
            println!("NOT-MATCH AFTER: {:?}", after);
            None
        }
    }
}

fn attach_kinds<'a>(nodes: &'a [Node<'a>]) -> Vec<(&'a str, &'a Node<'a>)> {
    nodes.iter().map(|node| (node.kind(), node)).collect()
}

#[derive(Debug)]
enum ElmChange {
    RenamedVar(String, String),
}

#[derive(Debug)]
struct TreeChanges<'a> {
    old_removed: Vec<Node<'a>>,
    new_added: Vec<Node<'a>>,
}

fn diff_trees<'a>(
    state: &'a SourceFileState,
    old: &'a mut TreeCursor,
    new: &'a mut TreeCursor,
) -> TreeChanges<'a> {
    loop {
        if !goto_first_changed_sibling(state, old, new) {
            return TreeChanges {
                old_removed: Vec::new(),
                new_added: Vec::new(),
            };
        }
        let first_old_changed = old.node();
        let first_new_changed = new.node();
        let (old_removed_count, new_added_count) = count_changed_siblings(state, old, new);

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
            old_removed,
            new_added,
        };
    }
}

// Move both cursors forward through sibbling nodes in lock step, stopping when
// we encounter a difference between the old and new node.
fn goto_first_changed_sibling(
    state: &SourceFileState,
    old: &mut TreeCursor,
    new: &mut TreeCursor,
) -> bool {
    loop {
        if has_node_changed(state, &old.node(), &new.node()) {
            return true;
        } else {
            match (old.goto_next_sibling(), new.goto_next_sibling()) {
                (true, true) => continue,
                (false, false) => return false,
                (_, _) => return true,
            }
        }
    }
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
    state: &'a SourceFileState,
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
        if has_node_changed(state, &old_sibling, &new_sibling) {
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
fn has_node_changed(state: &SourceFileState, old: &Node, new: &Node) -> bool {
    old.id() != new.id()
        && (old.kind_id() != new.kind_id() || have_node_contents_changed(state, old, new))
}

// Compare two nodes by comparing snippets of code covered by them. This is
// supposed to be a 100% accurate, albeit potentially slower equivalency check.
//
// TODO: code formatters can change code in ways that don't matter but would
// fail this check. Consider alternative approaches.
// TODO: compare u8 array slices here instead of parsing to string.
fn have_node_contents_changed(state: &SourceFileState, old: &Node, new: &Node) -> bool {
    let old_bytes = code_slice(state.checkpointed_code, &old.byte_range());
    let new_bytes = code_slice(state.latest_code, &new.byte_range());
    old_bytes != new_bytes
}

fn code_slice(code: &[u8], range: &Range<usize>) -> String {
    std::string::String::from_utf8(code[range.start..range.end].to_vec()).unwrap()
}

fn parse(prev_tree: Option<&Tree>, code: &[u8]) -> Option<Tree> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(tree_sitter_elm::language())
        .expect("Error loading elm grammer");
    parser.parse(code, prev_tree)
}

fn print_latest_tree(state: &SourceFileState) {
    let mut cursor = state.latest_tree.walk();
    print_tree_helper(state.latest_code, 0, &mut cursor);
    println!();
}

fn print_tree_helper(code: &[u8], indent: usize, cursor: &mut tree_sitter::TreeCursor) {
    let node = cursor.node();
    println!(
        "{}[{} {:?}] {:?}{}",
        "  ".repeat(indent),
        node.kind(),
        node.start_position().row + 1,
        code_slice(code, &node.byte_range()),
        if node.has_changes() { " (changed)" } else { "" },
    );
    if cursor.goto_first_child() {
        print_tree_helper(code, indent + 1, cursor);
        cursor.goto_parent();
    }
    if cursor.goto_next_sibling() {
        print_tree_helper(code, indent, cursor);
    }
}
