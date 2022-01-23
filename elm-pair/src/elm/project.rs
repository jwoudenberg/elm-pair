use crate::lib::log;
use crate::lib::log::Error;
use std::path::{Path, PathBuf};

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

pub fn is_elm_file(path: &Path) -> bool {
    path.extension() == Some(std::ffi::OsStr::new("elm"))
}

pub fn elm_json_path(project_root: &Path) -> PathBuf {
    project_root.join("elm.json")
}

pub fn root_from_elm_json_path(elm_json: &Path) -> Result<&Path, Error> {
    elm_json.parent().ok_or_else(|| {
        log::mk_err!(
            "couldn't navigate from elm.json file to project root directory"
        )
    })
}

pub fn idat_path(project_root: &Path) -> PathBuf {
    project_root
        .join(format!("elm-stuff/{}/i.dat", crate::elm::compiler::VERSION))
}

pub fn root_from_idat_path(idat: &Path) -> Result<&Path, Error> {
    idat.parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .ok_or_else(|| {
            log::mk_err!(
                "couldn't navigate from i.dat file to project root directory"
            )
        })
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
