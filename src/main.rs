extern crate notify;

use notify::{watcher, DebouncedEvent, RecursiveMode, Watcher};
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::channel;
use std::time::Duration;
use tree_sitter::Tree;

fn main() {
    let (tx, rx) = channel();

    let mut watcher = watcher(tx, Duration::from_millis(100)).unwrap();

    let mut prev_tree = None;

    watcher
        .watch("./test-project", RecursiveMode::Recursive)
        .unwrap();

    loop {
        match rx.recv() {
            Ok(event) => handle_event(&mut prev_tree, event),
            Err(e) => println!("watch error: {:?}", e),
        }
    }
}

fn handle_event(prev_tree: &mut Option<Tree>, event: DebouncedEvent) {
    let parse_result = event_path(event).and_then(|path| parse(prev_tree, path));
    if let Some(tree) = parse_result {
        print_tree(0, &mut tree.walk());
        *prev_tree = Some(tree);
    }
}

fn event_path(event: DebouncedEvent) -> Option<PathBuf> {
    match event {
        DebouncedEvent::NoticeWrite(_) => None,
        DebouncedEvent::NoticeRemove(_) => None,
        DebouncedEvent::Create(path) => Some(path),
        DebouncedEvent::Write(path) => Some(path),
        DebouncedEvent::Chmod(path) => Some(path),
        DebouncedEvent::Remove(_) => None,
        DebouncedEvent::Rename(_, path) => Some(path),
        DebouncedEvent::Rescan => None,
        DebouncedEvent::Error(e, _) => {
            println!("error event: {:?}", e);
            None
        }
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
    println!("{}{:?}", "  ".repeat(indent), cursor.node());
    if cursor.goto_first_child() {
        print_tree(indent + 1, cursor);
        cursor.goto_parent();
    }
    if cursor.goto_next_sibling() {
        print_tree(indent, cursor);
    }
}
