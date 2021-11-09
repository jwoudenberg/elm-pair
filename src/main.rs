use core::ops::Range;
use std::collections::HashMap;
use std::io::BufRead;
use std::path::PathBuf;
use tree_sitter::{InputEdit, Tree};

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
        let changes = diff_trees(state, &new_tree);
        println!("CHANGES: {:?}", changes);
    }
}

#[derive(Debug)]
enum Change<'a> {
    NodeRemoved(tree_sitter::Node<'a>),
    VariableNameChange(String, String),
}

fn diff_trees<'a>(state: &'a SourceFileState, new_tree: &'a tree_sitter::Tree) -> Vec<Change<'a>> {
    let mut old_cursor = state.tree.walk();
    let mut changes = Vec::new();
    loop {
        let old_node = old_cursor.node();

        // Skip if old node hasn't been changed.
        if !old_node.has_changes() {
            if step_forward(&mut old_cursor) {
                continue;
            } else {
                break;
            }
        }

        // Fetch new node.
        let opt_new_node = new_tree
            .root_node()
            .descendant_for_byte_range(old_node.start_byte(), old_node.end_byte());
        let new_node = match opt_new_node {
            None => {
                changes.push(Change::NodeRemoved(old_node));
                if step_forward(&mut old_cursor) {
                    continue;
                } else {
                    break;
                }
            }
            Some(new_node) => new_node,
        };

        // Skip if new node has the same id. (unsure: can this happen, given
        // chec for `has_changes` above?)
        if new_node.id() == old_node.id() {
            if step_forward(&mut old_cursor) {
                continue;
            } else {
                break;
            }
        }

        let opt_old_bytes = state
            .node_ranges_at_last_checkpoint
            .get(&old_node.id())
            .map(|range| code_slice(state.code_at_last_checkpoint, range));

        if let Some(old_bytes) = opt_old_bytes {
            let new_bytes = code_slice(state.code_latest, &new_node.byte_range());

            // Skip if the new node and old node contain the exact same code.
            if old_bytes == new_bytes {
                if step_forward(&mut old_cursor) {
                    continue;
                } else {
                    break;
                }
            }

            if old_node.kind() == "lower_case_identifier"
                && new_node.kind() == "lower_case_identifier"
            {
                changes.push(Change::VariableNameChange(old_bytes, new_bytes));
            }
        }

        // Descend into child nodes.
        if step_down(&mut old_cursor) {
            continue;
        } else {
            break;
        }
    }
    changes
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
