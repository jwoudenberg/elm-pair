use core::ops::Range;
use std::collections::HashMap;
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
    code_at_last_checkpoint: &'a [u8],
    // A map of tree-sitter node ids to byte ranges. This will allow us to
    // look up code snippets in the checkpointed code at later time.
    node_ranges_at_last_checkpoint: HashMap<usize, Range<usize>>,
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
    code_latest: &'a mut Vec<u8>,
    // A tree-sitter concrete syntax tree of the latest code.
    tree: Tree,
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
    println!("changed code: {:?}", changed_code);
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
    let node_ranges_at_last_checkpoint = byte_ranges_by_node_id(&tree, HashMap::new());
    print_tree(&tree);
    let mut state = SourceFileState {
        tree,
        node_ranges_at_last_checkpoint,
        code_at_last_checkpoint: initial_lines.as_bytes(),
        code_latest: &mut initial_lines.clone().into_bytes(),
    };

    // Subsequent parses of a file.
    for line in lines {
        let (_, changed_lines, edit) = parse_event(&line.unwrap());
        handle_event(&mut state, changed_lines.into_bytes(), edit)
    }

    // TODO: save a new state if compilation passes.
}

fn byte_ranges_by_node_id(
    tree: &Tree,
    mut acc: HashMap<usize, Range<usize>>,
) -> HashMap<usize, Range<usize>> {
    let mut cursor = tree.walk();
    loop {
        let node = cursor.node();
        acc.insert(node.id(), node.byte_range());
        if step_down(&mut cursor) {
            continue;
        } else {
            break;
        };
    }
    acc
}

fn handle_event(state: &mut SourceFileState, changed_bytes: Vec<u8>, edit: InputEdit) {
    println!("edit: {:?}", edit);
    state.tree.edit(&edit);
    print_tree(&state.tree);

    let range = edit.start_byte..edit.old_end_byte;
    state.code_latest.splice(range, changed_bytes);
    println!("{:?}", String::from_utf8(state.code_latest.to_vec()));
    let parse_result = parse(Some(&state.tree), state.code_latest);
    if let Some(new_tree) = parse_result {
        print_tree(&new_tree);
        let mut old_cursor = state.tree.walk();
        let mut new_cursor = new_tree.walk();
        let changes = diff_trees(state, &mut old_cursor, &mut new_cursor);
        println!("CHANGES: {:?}", changes);
    }
}

#[derive(Debug)]
enum TreeChanges<'a> {
    None,
    Single(Vec<Node<'a>>, Vec<Node<'a>>),
    AtLeastTwoUnrelated(Range<usize>, Range<usize>),
}

fn diff_trees<'a>(
    state: &'a SourceFileState,
    old: &'a mut TreeCursor,
    new: &'a mut TreeCursor,
) -> TreeChanges<'a> {
    loop {
        if !goto_first_changed_sibling(state, old, new) {
            return TreeChanges::None;
        }
        let first_old_changed = old.node();
        let first_new_changed = new.node();
        let (old_removed_count, new_added_count) = count_changed_siblings(state, old, new);

        // If only a single sibling changed and it's kind remained the same,
        // then we descend into that child.
        old.reset(first_old_changed);
        new.reset(first_new_changed);
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
        while old_removed.len() <= old_removed_count {
            old_removed.push(old.node());
            old.goto_next_sibling();
        }

        let mut new_added = Vec::with_capacity(new_added_count);
        while new_added.len() <= new_added_count {
            new_added.push(old.node());
            old.goto_next_sibling();
        }

        // TODO: confirm there are no changes elsewhere in the tree.

        return TreeChanges::Single(old_removed, new_added);
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
    old: &'a mut TreeCursor,
    new: &'a mut TreeCursor,
) -> (usize, usize) {
    // We initialize the counts at 1, because we assume the node we're currenly
    // on is the first changed node.
    let mut old_siblings_removed = 1;
    let mut new_siblings_added = 1;

    // Walk forward, basically counting all remaining old and new siblings.
    while old.goto_next_sibling() {
        old_siblings_removed += 1;
    }
    while new.goto_next_sibling() {
        new_siblings_added += 1;
    }

    // Walk backwards again until we encounter a changed node.
    let mut old_sibling = old.node();
    let mut new_sibling = new.node();
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
    old.has_changes()
        && old.id() != new.id()
        && (old.kind_id() != new.kind_id() || have_node_contents_changed(state, old, new))
}

// Compare two nodes by comparing snippets of code covered by them. This is
// supposed to be a 100% accurate, albeit potentially slower equivalency check.
//
// TODO: code formatters can change code in ways that don't matter but would
// fail this check. Consider alternative approaches.
fn have_node_contents_changed(state: &SourceFileState, old: &Node, new: &Node) -> bool {
    // TODO: compare u8 array slices here instead of parsing to string.
    let opt_old_bytes = state
        .node_ranges_at_last_checkpoint
        .get(&old.id())
        .map(|range| code_slice(state.code_at_last_checkpoint, range));
    match opt_old_bytes {
        None => true,
        Some(old_bytes) => {
            let new_bytes = code_slice(state.code_latest, &new.byte_range());
            old_bytes != new_bytes
        }
    }
}

fn code_slice(code: &[u8], range: &Range<usize>) -> String {
    std::string::String::from_utf8(code[range.start..range.end].to_vec()).unwrap()
}

fn step_forward(tree: &mut tree_sitter::TreeCursor) -> bool {
    tree.goto_next_sibling() || (tree.goto_parent() && step_forward(tree))
}

fn step_down(tree: &mut tree_sitter::TreeCursor) -> bool {
    tree.goto_first_child() || step_forward(tree)
}

fn parse(prev_tree: Option<&Tree>, code: &[u8]) -> Option<Tree> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(tree_sitter_elm::language())
        .expect("Error loading elm grammer");
    parser.parse(code, prev_tree)
}

fn print_tree(tree: &Tree) {
    let mut cursor = tree.walk();
    print_tree_helper(0, &mut cursor);
    println!();
}

fn print_tree_helper(indent: usize, cursor: &mut tree_sitter::TreeCursor) {
    let node = cursor.node();
    if node.has_changes() {
        println!("{}CHANGED: {:?}", "  ".repeat(indent), cursor.node());
    } else {
        println!(
            "{}{:?} {:?} ({:?})",
            "  ".repeat(indent),
            node.id(),
            node.kind(),
            node.byte_range()
        );
    }
    if cursor.goto_first_child() {
        print_tree_helper(indent + 1, cursor);
        cursor.goto_parent();
    }
    if cursor.goto_next_sibling() {
        print_tree_helper(indent, cursor);
    }
}
