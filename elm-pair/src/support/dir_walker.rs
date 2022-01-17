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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_files() {
        let dir = std::env::temp_dir().join("elm-pair-dir-walker-test");
        // Start the test with an empty directory.
        // This might fail if the directory doesn't exist, which is fine.
        std::fs::remove_dir_all(&dir).unwrap_or(());

        let mut files = vec![
            dir.join("top-level-path.txt"),
            dir.join("some/path.ext"),
            dir.join("some/nested/path"),
        ];

        for file in files.iter() {
            std::fs::create_dir_all(&file.parent().unwrap()).unwrap();
            std::fs::write(file, &[]).unwrap();
        }

        let mut actual_files = DirWalker::new(&dir).collect::<Vec<PathBuf>>();
        actual_files.sort();
        files.sort();
        assert_eq!(actual_files, files,);
    }
}
