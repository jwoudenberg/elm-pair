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

#[derive(Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ElmJson {
    #[serde(rename = "source-directories")]
    pub source_directories: Vec<PathBuf>,
}
