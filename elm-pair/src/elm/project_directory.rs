use crate::support::log;
use crate::support::log::Error;
use std::path::Path;

pub fn root(path: &Path) -> Result<&Path, Error> {
    let mut maybe_root = path;
    loop {
        if maybe_root.join("elm.json").exists() {
            return Ok(maybe_root);
        } else {
            match maybe_root.parent() {
                None => {
                    return Err(log::mk_err!(
                        "Did not find elm.json file in any ancestor directory of module path"
                    ));
                }
                Some(parent) => {
                    maybe_root = parent;
                }
            }
        }
    }
}

#[cfg(test)]
mod root_tests {
    use super::*;

    #[test]
    fn finds_root_when_ancestor_directory_contains_elm_json() {
        let dir = tempdir::TempDir::new("elm-pair-tests").unwrap().into_path();

        let path = dir.join("project/src/long/winding/path");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(&dir.join("project/elm.json"), &[]).unwrap();

        assert_eq!(root(&path), Ok(dir.join("project").as_path()));
    }

    #[test]
    fn returns_error_when_no_ancestor_directory_contains_elm_json() {
        let dir = tempdir::TempDir::new("elm-pair-tests").unwrap().into_path();

        let path = dir.join("project/src/long/winding/path");
        std::fs::create_dir_all(&path).unwrap();

        assert!(root(&path).is_err());
    }
}
