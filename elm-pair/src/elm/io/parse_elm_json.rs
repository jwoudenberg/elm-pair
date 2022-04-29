use crate::elm::project;
use crate::lib::log;
use crate::lib::log::Error;
use serde::Deserialize;
use std::io::BufReader;
use std::path::{Path, PathBuf};

pub fn parse_elm_json(path: &Path) -> Result<ElmJson, Error> {
    let file = std::fs::File::open(path).map_err(|err| {
        log::mk_err!("error while reading elm.json: {:?}", err)
    })?;
    let reader = BufReader::new(file);
    let mut elm_json: ElmJson =
        serde_json::from_reader(reader).map_err(|err| {
            log::mk_err!("error while parsing elm.json: {:?}", err)
        })?;
    let project_root = project::root_from_elm_json_path(path)?;
    for dir in elm_json.source_directories.as_mut_slice() {
        let abs_path = project_root.join(&dir);
        // If we cannot canonicalize the path, likely because it doesn't
        // exist, we still want to keep listing the directory in case it is
        // created in the future.
        *dir = abs_path.canonicalize().unwrap_or(abs_path);
    }
    Ok(elm_json)
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub struct ElmJson {
    #[serde(rename = "source-directories", default = "default_source_dirs")]
    pub source_directories: Vec<PathBuf>,
}

fn default_source_dirs() -> Vec<PathBuf> {
    vec![PathBuf::from("src")]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_elm_json() {
        let dir = crate::lib::tempdir::new();
        let elm_json = r#"
            {
                "type": "application",
                "source-directories": [
                    "src",
                    "lib/more-src"
                ],
                "elm-version": "0.19.1",
                "dependencies": {
                    "direct": {
                        "elm/core": "1.0.5",
                        "elm/json": "1.1.3",
                        "elm/parser": "1.1.0",
                        "elm/time": "1.0.0"
                    },
                    "indirect": {}
                },
                "test-dependencies": {
                    "direct": {},
                    "indirect": {}
                }
            }
        "#;
        let path = dir.join("elm.json");
        std::fs::write(&path, elm_json).unwrap();
        assert_eq!(
            parse_elm_json(&path),
            Ok(ElmJson {
                source_directories: vec![
                    dir.join("src"),
                    dir.join("lib/more-src"),
                ]
            }),
        );
    }

    #[test]
    fn package_elm_json() {
        let dir = crate::lib::tempdir::new();
        let elm_json = r#"
            {
                "type": "package",
                "name": "jwoudenberg/elm-pair-experiment",
                "summary": "If a package exists in a test suite where no-one can install it, does it make a sound?",
                "license": "MIT",
                "version": "1.2.3",
                "exposed-modules": [
                    "Experiment"
                ],
                "elm-version": "0.19.0 <= v < 0.20.0",
                "dependencies": {
                    "elm/core": "1.0.0 <= v < 2.0.0"
                },
                "test-dependencies": {}
            }
        "#;
        let path = dir.join("elm.json");
        std::fs::write(&path, elm_json).unwrap();
        assert_eq!(
            parse_elm_json(&path),
            Ok(ElmJson {
                source_directories: vec![dir.join("src"),]
            }),
        );
    }
}
