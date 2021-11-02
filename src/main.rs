extern crate notify;

use notify::{watcher, DebouncedEvent, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::mpsc::channel;
use std::time::Duration;

fn main() {
    let (tx, rx) = channel();

    let mut watcher = watcher(tx, Duration::from_millis(100)).unwrap();

    watcher
        .watch("./test-project", RecursiveMode::Recursive)
        .unwrap();

    loop {
        match rx.recv() {
            Ok(event) => handle_event(event),
            Err(e) => println!("watch error: {:?}", e),
        }
    }
}

fn handle_event(event: DebouncedEvent) {
    println!("{:?}", event_path(event));
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
