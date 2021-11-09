use core::ops::Range;
use std::collections::HashMap;
use std::io::BufRead;
use std::path::PathBuf;
use tree_sitter::{InputEdit, Tree};

fn main() {
    let fifo_path = "/tmp/elm-pair";
    nix::unistd::mkfifo(fifo_path, nix::sys::stat::Mode::S_IRWXU).unwrap();
    let fifo = std::fs::File::open(fifo_path).unwrap();
    handle_events(&mut std::io::BufReader::new(fifo).lines());
}

struct SourceFileState {
    code_at_last_checkpoint: Vec<String>,
    node_ranges_at_last_checkpoint: HashMap<usize, Range<usize>>,
    code_latest: Vec<String>,
    tree: Tree,
}

fn parse_event(serialized_event: &str) -> (PathBuf, Vec<String>, InputEdit) {
    let (
        path,
        changed_lines,
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
    (path, changed_lines, input_edit)
}

fn handle_events<I>(lines: &mut I)
where
    I: Iterator<Item = Result<String, std::io::Error>>,
{
    // First event returns the initial state.
    let first_line = match lines.next() {
        None => return, // We receive no lines at all :(. Exit early.
        Some(line) => line,
    };
    let (_, initial_lines, _) = parse_event(&first_line.unwrap());
    let mut state = handle_event_first(initial_lines);

    // Subsequent parses of a file.
    for line in lines {
        let (_, changed_lines, edit) = parse_event(&line.unwrap());
        handle_event_but_first(&mut state, changed_lines, edit)
    }

    // TODO: save a new state if compilation passes.
}

fn handle_event_first(changed_lines: Vec<String>) -> SourceFileState {
    let tree = parse(None, &changed_lines).unwrap();
    print_tree(0, &mut tree.walk());
    println!();
    let mut node_ranges_at_last_checkpoint = HashMap::new();
    {
        let mut cursor = tree.walk();
        loop {
            let node = cursor.node();
            node_ranges_at_last_checkpoint.insert(node.id(), node.byte_range());
            if step_down(&mut cursor) {
                continue;
            } else {
                break;
            };
        }
    }
    SourceFileState {
        tree,
        node_ranges_at_last_checkpoint,
        code_at_last_checkpoint: changed_lines.clone(),
        code_latest: changed_lines,
    }
}
fn handle_event_but_first(
    state: &mut SourceFileState,
    changed_lines: Vec<String>,
    edit: InputEdit,
) {
    println!("edit: {:?}", edit);
    state.tree.edit(&edit);
    print_tree(0, &mut state.tree.walk());

    let range = edit.start_position.row..(edit.new_end_position.row + 1);
    state.code_latest.splice(range, changed_lines);
    let parse_result = parse(Some(&state.tree), &state.code_latest);
    if let Some(new_tree) = parse_result {
        print_tree(0, &mut new_tree.walk());
        println!();
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
            .map(|range| code_slice(&state.code_at_last_checkpoint, range));

        if let Some(old_bytes) = opt_old_bytes {
            let new_bytes = code_slice(&state.code_latest, &new_node.byte_range());

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

fn code_slice(code: &[String], range: &Range<usize>) -> String {
    std::string::String::from_utf8(code.join("\n").as_bytes()[range.start..range.end].to_vec())
        .unwrap()
}

fn step_forward(tree: &mut tree_sitter::TreeCursor) -> bool {
    tree.goto_next_sibling() || (tree.goto_parent() && step_forward(tree))
}

fn step_down(tree: &mut tree_sitter::TreeCursor) -> bool {
    tree.goto_first_child() || step_forward(tree)
}

fn parse(prev_tree: Option<&Tree>, code: &[String]) -> Option<Tree> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(tree_sitter_elm::language())
        .expect("Error loading elm grammer");
    parser.parse(code.join("\n"), prev_tree)
}

fn print_tree(indent: usize, cursor: &mut tree_sitter::TreeCursor) {
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
        print_tree(indent + 1, cursor);
        cursor.goto_parent();
    }
    if cursor.goto_next_sibling() {
        print_tree(indent, cursor);
    }
}
