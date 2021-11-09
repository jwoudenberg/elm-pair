use std::io::BufRead;
use std::path::PathBuf;
use tree_sitter::{InputEdit, Tree};

fn main() {
    let mut prev_tree: Option<Tree> = None;
    let mut code: Option<Vec<String>> = None;

    let fifo_path = "/tmp/elm-pair";
    nix::unistd::mkfifo(fifo_path, nix::sys::stat::Mode::S_IRWXU).unwrap();
    let fifo = std::fs::File::open(fifo_path).unwrap();
    for line in std::io::BufReader::new(fifo).lines() {
        let (_, changed_lines, input_edit) = parse_event(&line.unwrap());
        let new_code = match code {
            None => changed_lines,
            Some(mut old) => {
                let range = input_edit.start_position.row..(input_edit.new_end_position.row + 1);
                old.splice(range, changed_lines);
                old
            }
        };
        handle_event(&mut prev_tree, new_code.as_ref(), input_edit);
        code = Some(new_code);
    }
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

fn handle_event(prev_tree: &mut Option<Tree>, code: &[String], edit: InputEdit) {
    if let Some(prev_tree_exists) = prev_tree {
        println!("edit: {:?}", edit);
        prev_tree_exists.edit(&edit);
        print_tree(0, &mut prev_tree_exists.walk());
    }
    let parse_result = parse(prev_tree, code);
    if let Some(tree) = parse_result {
        print_tree(0, &mut tree.walk());
        println!();
        if let Some(prev_tree_exists) = prev_tree {
            let changes = diff_trees(code, prev_tree_exists, &tree);
            println!("CHANGES: {:?}", changes);
        }

        // Temporarily only save the first generated tree, to allow tests of
        // multiple edits after a 'checkpoint'.
        match prev_tree {
            None => {
                *prev_tree = Some(tree);
            }
            Some(_) => {}
        }
    }
}

#[derive(Debug)]
enum Change<'a> {
    NodeRemoved(tree_sitter::Node<'a>),
    VariableNameChange(String, String),
}

fn diff_trees<'a>(
    code: &[String],
    old_tree: &'a tree_sitter::Tree,
    new_tree: &'a tree_sitter::Tree,
) -> Vec<Change<'a>> {
    let mut old_cursor = old_tree.walk();
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

        // TODO: skip if code covered by this node hasn't changed.

        if old_node.kind() == "lower_case_identifier"
            && new_node.kind() == "lower_case_identifier"
            && old_node.byte_range() == new_node.byte_range()
        {
            // TODO: this is twice wrong. I need to consider the old code here, and the old node before we edited it (and possibly changed the byterange)
            changes.push(Change::VariableNameChange(
                code_slice(code, old_node.byte_range()),
                code_slice(code, new_node.byte_range()),
            ));
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

fn code_slice(code: &[String], range: core::ops::Range<usize>) -> String {
    std::string::String::from_utf8(code.join("\n").as_bytes()[range].to_vec()).unwrap()
}

fn step_forward(tree: &mut tree_sitter::TreeCursor) -> bool {
    tree.goto_next_sibling() || (tree.goto_parent() && tree.goto_next_sibling())
}

fn step_down(tree: &mut tree_sitter::TreeCursor) -> bool {
    tree.goto_first_child() || step_forward(tree)
}

fn parse(prev_tree: &mut Option<Tree>, code: &[String]) -> Option<Tree> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(tree_sitter_elm::language())
        .expect("Error loading elm grammer");
    parser.parse(code.join("\n"), prev_tree.as_ref())
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
