use std::fs;
use std::io::BufRead;
use std::path::PathBuf;
use tree_sitter::{InputEdit, Tree};

fn main() {
    let mut prev_tree: Option<Tree> = None;

    let fifo_path = "/tmp/elm-pair";
    nix::unistd::mkfifo(fifo_path, nix::sys::stat::Mode::S_IRWXU).unwrap();
    let fifo = std::fs::File::open(fifo_path).unwrap();
    for line in std::io::BufReader::new(fifo).lines() {
        let serialized_event = &line.unwrap();
        let (path, input_edit) = parse_event(serialized_event);
        handle_event(&mut prev_tree, path, input_edit);
    }
}

fn parse_event(serialized_event: &str) -> (PathBuf, InputEdit) {
    let (
        path,
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
    (path, input_edit)
}

fn handle_event(prev_tree: &mut Option<Tree>, path: PathBuf, edit: InputEdit) {
    if let Some(prev_tree_exists) = prev_tree {
        prev_tree_exists.edit(&edit);
    }
    // TODO: either wait for save event before reading, or get code from editor
    // directly.
    let parse_result = parse(prev_tree, path);
    if let Some(tree) = parse_result {
        print_tree(0, &mut tree.walk());
        println!();
        *prev_tree = Some(tree);
    }
}

fn parse(prev_tree: &mut Option<Tree>, path: PathBuf) -> Option<Tree> {
    let code = fs::read(path).ok()?;
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(tree_sitter_elm::language())
        .expect("Error loading elm grammer");
    parser.parse(code, prev_tree.as_ref())
}

fn print_tree(indent: usize, cursor: &mut tree_sitter::TreeCursor) {
    let node = cursor.node();
    if node.has_changes() {
        println!("{}CHANGED: {:?}", "  ".repeat(indent), cursor.node());
    } else {
        println!("{}{:?}", "  ".repeat(indent), cursor.node());
    }
    if cursor.goto_first_child() {
        print_tree(indent + 1, cursor);
        cursor.goto_parent();
    }
    if cursor.goto_next_sibling() {
        print_tree(indent, cursor);
    }
}
