use crate::support::log;
use crate::support::log::Error;
use std::path::{Path, PathBuf};

// We look up the ELM_BINARY_PATH at compile time to register the elm binary as
// a dependency of elm-pair in a Nix build.
const NIX_ELM_BINARY_PATH: Option<&str> = option_env!("ELM_BINARY_PATH");

#[derive(Clone)]
pub struct Compiler {
    elm_binary_path: PathBuf,
}

impl Compiler {
    pub(crate) fn new() -> Result<Compiler, Error> {
        let elm_binary_path = match NIX_ELM_BINARY_PATH {
            Some(path) => PathBuf::from(path),
            None => PathBuf::from("elm"),
        };
        let compiler = Compiler { elm_binary_path };
        Ok(compiler)
    }

    pub(crate) fn make(
        &self,
        project_root: &Path,
        code: &ropey::Rope,
    ) -> Result<std::process::Output, Error> {
        // Write lates code to temporary file. We don't compile the original source
        // file, because the version stored on disk is likely ahead or behind the
        // version in the editor.
        let temp_path = crate::elm_pair_dir()?.join("Temp.elm");
        std::fs::write(&temp_path, &code.bytes().collect::<Vec<u8>>())
            .map_err(|err| {
                log::mk_err!(
                    "error while writing to file {:?}: {:?}",
                    temp_path,
                    err
                )
            })?;

        // Run Elm compiler against temporary file.
        std::process::Command::new(&self.elm_binary_path)
            .arg("make")
            .arg("--report=json")
            .arg(temp_path)
            .current_dir(project_root)
            .output()
            .map_err(|err| log::mk_err!("error running `elm make`: {:?}", err))
    }
}
