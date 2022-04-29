use crate::lib::source_code::Edit;
use crate::lib::source_code::{RefactorAllowed, SourceFileSnapshot};
use crate::Error;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub mod neovim;
pub mod vscode;

#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Neovim,
    VsCode,
}

#[derive(
    Serialize,
    Deserialize,
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
)]
pub struct Id(u32);

impl Id {
    pub fn new(id: u32) -> Id {
        Id(id)
    }
}

// An API for communicatating with an editor.
pub trait Editor {
    type Driver: Driver;

    // Listen for changes to source files happening in the editor.
    fn listen<F>(self, on_event: F) -> Result<(), Error>
    where
        F: FnMut(Event) -> Result<(), Error>;

    // Obtain a driver for sending commands to the editor.
    fn driver(&self) -> Self::Driver;

    // Get the kind of editor this is.
    fn kind(&self) -> Kind;
}

pub enum Event {
    OpenedNewBuffer {
        code: SourceFileSnapshot,
        path: PathBuf,
    },
    ModifiedBuffer {
        code: SourceFileSnapshot,
        refactor_allowed: RefactorAllowed,
    },
    EnteredLicenseKey {
        key: String,
    },
}

// An API for sending commands to an editor.
pub trait Driver: 'static + Send {
    fn kind(&self) -> Kind;
    fn apply_edits(&self, edits: Vec<Edit>) -> bool;
    fn open_files(&self, files: Vec<PathBuf>) -> bool;
    fn show_file(&self, path: &Path) -> bool;
}
