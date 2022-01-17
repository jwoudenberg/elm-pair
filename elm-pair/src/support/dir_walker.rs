use crate::support::log;
use std::path::{Path, PathBuf};

// This iterator finds as many files as it can and so logs rather than fails
// when it encounters an error.
pub(crate) struct DirWalker {
    directories: Vec<std::fs::ReadDir>,
}

impl DirWalker {
    pub(crate) fn new(root: &Path) -> DirWalker {
        let directories = match std::fs::read_dir(root) {
            Ok(read_dir) => vec![read_dir],
            Err(err) => {
                log::error!(
                    "error while reading contents of source directory {:?}: {:?}",
                    root,
                    err
                );
                Vec::new()
            }
        };
        DirWalker { directories }
    }
}

impl Iterator for DirWalker {
    type Item = PathBuf;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(read_dir) = self.directories.last_mut() {
            match read_dir.next() {
                None => {
                    self.directories.pop();
                }
                Some(Err(err)) => {
                    log::error!(
                        "error while reading entry of source (sub)directory: {:?}",
                        err
                    );
                }
                Some(Ok(entry)) => match entry.file_type() {
                    Err(err) => {
                        log::error!(
                            "error while reading file type of path {:?}: {:?}",
                            entry.path(),
                            err
                        );
                    }
                    Ok(file_type) => {
                        let path = entry.path();
                        if file_type.is_dir() {
                            match std::fs::read_dir(&path) {
                                Ok(inner_read_dir) => {
                                    self.directories.push(inner_read_dir)
                                }
                                Err(err) => {
                                    log::error!(
                                                    "error while reading contents of source directory {:?}: {:?}",
                                                    path,
                                                    err
                                                );
                                }
                            }
                        } else {
                            return Some(path);
                        }
                    }
                },
            }
        }
        None
    }
}
