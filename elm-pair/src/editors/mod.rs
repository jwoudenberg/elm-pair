use crate::lib::source_code::Edit;
use crate::lib::source_code::{RefactorAllowed, SourceFileSnapshot};
use crate::Error;
use abomonation_derive::Abomonation;
use std::path::PathBuf;

pub mod neovim;
pub mod vscode;

#[derive(Debug)]
pub enum Kind {
    Neovim,
    VsCode,
}

#[derive(
    Abomonation, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord,
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

    fn name(&self) -> &'static str;
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
}

// An API for sending commands to an editor.
pub trait Driver: 'static + Send {
    fn apply_edits(&self, edits: Vec<Edit>) -> bool;
    fn open_files(&self, files: Vec<PathBuf>) -> bool;
}
