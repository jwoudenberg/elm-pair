use crate::lib::intersperse::Intersperse;
use crate::lib::log;
use crate::lib::log::Error;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(
    Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord,
)]
pub struct ModuleName(pub String);

impl ModuleName {
    #[allow(dead_code)]
    pub fn from_str(name: &str) -> ModuleName {
        ModuleName(name.to_string())
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl std::fmt::Display for ModuleName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

pub fn from_path(source_dir: &Path, path: &Path) -> Result<ModuleName, Error> {
    let name = path
        .with_extension("")
        .strip_prefix(source_dir)
        .map_err(|err| {
            log::mk_err!(
        "error stripping source directory {:?} from elm module path {:?}: {:?}",
                path,
                source_dir,
                err
            )
        })?
        .components()
        .filter_map(|component| {
            if let std::path::Component::Normal(os_str) = component {
                Some(os_str.to_str().ok_or(os_str))
            } else {
                None
            }
        })
        .my_intersperse(Ok("."))
        .collect::<Result<String, &std::ffi::OsStr>>()
        .map_err(|os_str| {
            log::mk_err!(
"directory segment of Elm module used in module name is not valid UTF8: {:?}",
                os_str
            )
        })?;
    Ok(ModuleName(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_level_module() {
        assert_eq!(
            from_path(
                Path::new("/project/src"),
                Path::new("/project/src/TopLevel.elm"),
            ),
            Ok(ModuleName::from_str("TopLevel"))
        );
    }

    #[test]
    fn nested_module() {
        assert_eq!(
            from_path(
                Path::new("/project/src"),
                Path::new("/project/src/Some/Nested/Module.elm"),
            ),
            Ok(ModuleName::from_str("Some.Nested.Module"))
        );
    }

    #[test]
    fn path_not_in_src_module() {
        assert!(from_path(
            Path::new("/project/src"),
            Path::new("/elsewhere/Some/Nested/Module.elm"),
        )
        .is_err(),);
    }
}
