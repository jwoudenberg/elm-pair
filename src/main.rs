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
