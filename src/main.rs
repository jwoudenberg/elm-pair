extern crate notify;

use notify::{watcher, RecursiveMode, Watcher};
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
            Ok(event) => println!("{:?}", event),
            Err(e) => println!("watch error: {:?}", e),
        }
    }
}
