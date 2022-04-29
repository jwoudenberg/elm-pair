use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time;

static NEXT_TEMP_DIR_INDEX: AtomicUsize = AtomicUsize::new(0);

pub fn new() -> PathBuf {
    let base = std::env::temp_dir();
    let time = time::SystemTime::now()
        .duration_since(time::SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let index = NEXT_TEMP_DIR_INDEX.fetch_add(1, Ordering::Relaxed);
    let path = base.join(format!("elm-pair-tests-{}-{}", time, index));
    std::fs::create_dir_all(&path).unwrap();
    path
}
