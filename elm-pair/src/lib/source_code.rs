use crate::editors;
use crate::lib::log;
use crate::lib::log::Error;
use core::ops::Range;
use ropey::{Rope, RopeSlice};
use serde::{Deserialize, Serialize};
use tree_sitter::{InputEdit, Node, Tree};

// A unique identifier for a buffer that elm-pair is tracking in any connected
// editor. First 32 bits uniquely identify the connected editor, while the last
// 32 bits identify one of the buffers openen in that particular editor.
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
pub struct Buffer {
    pub editor_id: editors::Id,
    pub buffer_id: u32,
}

#[derive(Clone)]
pub struct SourceFileSnapshot {
    // A unique index identifying a source file open in an editor. We're not
    // using the file path for a couple of reasons:
    // - It's possible for the same file to be open in multiple editors with
    //   different unsaved changes each.
    // - A file path is stringy, so more expensive to copy.
    pub buffer: Buffer,
    // The full contents of the file, stored in a Rope datastructure. This
    // datastructure offers cheap modifications in random locations, and cheap
    // cloning (both of which we'll do a lot).
    pub bytes: Rope,
    // The tree-sitter concrete syntax tree representing the code in `bytes`.
    // This tree by itself is not enough to recover the source code, which is
    // why we also keep the original source code in `bytes`.
    pub tree: Tree,
    // A number that gets incremented for each change to this snapshot.
    pub revision: usize,
}

impl SourceFileSnapshot {
    pub fn new(
        buffer: Buffer,
        bytes: Rope,
    ) -> Result<SourceFileSnapshot, Error> {
        let snapshot = SourceFileSnapshot {
            tree: parse_rope(None, &bytes)?,
            buffer,
            bytes,
            revision: 0,
        };
        Ok(snapshot)
    }

    pub fn apply_edit(&mut self, edit: InputEdit) -> Result<(), Error> {
        // Increment the revision by 2. Given a first revision of 0, this will
        // ensure we only get even revision numbers by default. Refactor code
        // will manually set odd revisions, to help keep revisions from the
        // editor and elm-pair introduced ones apart.
        self.revision += 2;
        self.tree.edit(&edit);
        let new_tree = parse_rope(Some(&self.tree), &self.bytes)?;
        self.tree = new_tree;
        Ok(())
    }

    pub fn slice(&self, range: &Range<usize>) -> RopeSlice {
        let start = self
            .bytes
            .try_byte_to_char(range.start)
            .unwrap_or_else(|_| self.bytes.len_bytes());
        let end = self
            .bytes
            .try_byte_to_char(range.end)
            .unwrap_or_else(|_| self.bytes.len_bytes());
        self.bytes.slice(start..end)
    }
}

// TODO: reuse parser.
fn parse_rope(prev_tree: Option<&Tree>, code: &Rope) -> Result<Tree, Error> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(tree_sitter_elm::language())
        .map_err(|err| {
            log::mk_err!(
                "failed setting tree-sitter parser language: {:?}",
                err
            )
        })?;
    let mut callback = |offset, _| {
        let (chunk, chunk_byte_index, _, _) = code.chunk_at_byte(offset);
        &chunk[(offset - chunk_byte_index)..]
    };
    match parser.parse_with(&mut callback, prev_tree) {
        None => Err(log::mk_err!("tree-sitter failed to parse code")),
        Some(tree) => Ok(tree),
    }
}

// TODO: reuse parser.
pub fn parse_bytes(bytes: impl AsRef<[u8]>) -> Result<Tree, Error> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(tree_sitter_elm::language())
        .map_err(|err| {
            log::mk_err!(
                "failed setting tree-sitter parser language: {:?}",
                err
            )
        })?;
    match parser.parse(bytes, None) {
        None => Err(log::mk_err!("tree-sitter failed to parse code")),
        Some(tree) => Ok(tree),
    }
}

impl<'a> tree_sitter::TextProvider<'a> for &'a SourceFileSnapshot {
    type I = Chunks<'a>;

    fn text(&mut self, node: Node<'_>) -> Chunks<'a> {
        let chunks = self.bytes.slice(node.byte_range()).chunks();
        Chunks { chunks }
    }
}

pub struct Chunks<'a> {
    chunks: ropey::iter::Chunks<'a>,
}

impl<'a> Iterator for Chunks<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        self.chunks.next().map(str::as_bytes)
    }
}

// A change made by the user reported by the editor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    // The buffer that was changed.
    pub buffer: Buffer,
    // A tree-sitter InputEdit value, describing what part of the file was changed.
    pub input_edit: InputEdit,
    // Bytes representing the new contents of the file at the location described
    // by `input_edit`.
    pub new_bytes: String,
}

impl Edit {
    pub fn new(
        buffer: Buffer,
        bytes: &mut Rope,
        range: &Range<usize>,
        new_bytes: String,
    ) -> Edit {
        let new_end_byte = range.start + new_bytes.len();
        let start_position = byte_to_point(bytes, range.start);
        let old_end_position = byte_to_point(bytes, range.end);
        update_bytes(bytes, range.start, range.end, &new_bytes);
        let new_end_position = byte_to_point(bytes, new_end_byte);
        Edit {
            buffer,
            new_bytes,
            input_edit: tree_sitter::InputEdit {
                start_byte: range.start,
                old_end_byte: range.end,
                new_end_byte,
                start_position,
                old_end_position,
                new_end_position,
            },
        }
    }
}

pub fn update_bytes(
    bytes: &mut Rope,
    start_byte: usize,
    old_end_byte: usize,
    new_bytes: &str,
) {
    let start_char = bytes.byte_to_char(start_byte);
    let old_end_char = bytes.byte_to_char(old_end_byte);
    bytes.remove(start_char..old_end_char);
    bytes.insert(start_char, new_bytes);
}

pub fn byte_to_point(code: &Rope, byte: usize) -> tree_sitter::Point {
    let row = code.byte_to_line(byte);
    tree_sitter::Point {
        row,
        column: code.byte_to_char(byte) - code.line_to_char(row),
    }
}

#[derive(Clone, Copy)]
pub enum RefactorAllowed {
    Yes,
    No,
}
