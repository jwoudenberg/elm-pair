use crate::lib::log;
use crate::lib::log::Error;
use std::path::{Path, PathBuf};

pub const VERSION: &str = "0.19.1";

// We look up the ELM_BINARY_PATH at compile time to register the elm binary as
// a dependency of elm-pair in a Nix build.
const NIX_ELM_BINARY_PATH: Option<&str> = option_env!("ELM_BINARY_PATH");

#[derive(Clone)]
pub struct Compiler {
    elm_binary_path: PathBuf,
}

impl Compiler {
    pub fn new() -> Result<Compiler, Error> {
        let elm_binary_path = NIX_ELM_BINARY_PATH
            .map(PathBuf::from)
            .and_then(valid_elm_binary)
            .or_else(|| plugin_provided_elm_binary().and_then(valid_elm_binary))
            .or_else(|| valid_elm_binary(PathBuf::from("elm")))
            .ok_or_else(|| {
                log::mk_err!(
                    "Could not find an Elm binary with the right version: {}",
                    VERSION
                )
            })?;
        log::info!("Found Elm compiler binary: {:?}", elm_binary_path);
        let compiler = Compiler { elm_binary_path };
        Ok(compiler)
    }

    pub fn make(
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

// On non-nix based installs the editor plugin will download the elm binary and
// put it next to the plugin executable. This function provides the path to this
// location.
fn plugin_provided_elm_binary() -> Option<PathBuf> {
    match std::env::current_exe() {
        Err(err) => {
            log::error!(
                "Could not read location of current executable: {:?}",
                err
            );
            None
        }
        Ok(path) => path.parent().map(|p| p.join("elm")),
    }
}

fn valid_elm_binary(path: PathBuf) -> Option<PathBuf> {
    let opt_output =
        std::process::Command::new(&path).arg("--version").output();
    match opt_output {
        Err(err) => {
            if err.kind() != std::io::ErrorKind::NotFound {
                log::error!(
                    "`elm --version` failed with for binary at {:?}: {:?}",
                    path,
                    err
                )
            }
            None
        }
        Ok(output) => {
            if output.stdout.starts_with(VERSION.as_bytes()) {
                Some(path)
            } else {
                None
            }
        }
    }
}
