use crate::Error;
use core::ops::Range;
use ropey::{Rope, RopeSlice};
use tree_sitter::{InputEdit, Node, Tree};

// A unique identifier for a buffer that elm-pair is tracking in any connected
// editor. First 32 bits uniquely identify the connected editor, while the last
// 32 bits identify one of the buffers openen in that particular editor.
#[derive(Copy, Clone, Debug, Hash, PartialEq)]
pub(crate) struct Buffer {
    pub(crate) editor_id: u32,
    pub(crate) buffer_id: u32,
}

impl Eq for Buffer {}

#[derive(Clone)]
pub(crate) struct SourceFileSnapshot {
    // A unique index identifying a source file open in an editor. We're not
    // using the file path for a couple of reasons:
    // - It's possible for the same file to be open in multiple editors with
    //   different unsaved changes each.
    // - A file path is stringy, so more expensive to copy.
    pub(crate) buffer: Buffer,
    // The full contents of the file, stored in a Rope datastructure. This
    // datastructure offers cheap modifications in random locations, and cheap
    // cloning (both of which we'll do a lot).
    pub(crate) bytes: Rope,
    // The tree-sitter concrete syntax tree representing the code in `bytes`.
    // This tree by itself is not enough to recover the source code, which is
    // why we also keep the original source code in `bytes`.
    pub(crate) tree: Tree,
    // A number that gets incremented for each change to this snapshot.
    pub(crate) revision: usize,
}

impl SourceFileSnapshot {
    pub(crate) fn new(
        buffer: Buffer,
        bytes: Rope,
    ) -> Result<SourceFileSnapshot, Error> {
        let snapshot = SourceFileSnapshot {
            tree: parse(None, &bytes)?,
            buffer,
            bytes,
            revision: 0,
        };
        Ok(snapshot)
    }

    pub(crate) fn apply_edit(&mut self, edit: InputEdit) -> Result<(), Error> {
        self.revision += 1;
        self.tree.edit(&edit);
        let new_tree = parse(Some(&self.tree), &self.bytes)?;
        self.tree = new_tree;
        Ok(())
    }

    pub(crate) fn slice(&self, range: &Range<usize>) -> RopeSlice {
        let start = self.bytes.byte_to_char(range.start);
        let end = self.bytes.byte_to_char(range.end);
        self.bytes.slice(start..end)
    }
}

// TODO: reuse parser.
fn parse(prev_tree: Option<&Tree>, code: &Rope) -> Result<Tree, Error> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(tree_sitter_elm::language())
        .map_err(Error::TreeSitterSettingLanguageFailed)?;
    match parser.parse(code.bytes().collect::<Vec<u8>>(), prev_tree) {
        None => Err(Error::TreeSitterParsingFailed),
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

pub(crate) struct Chunks<'a> {
    chunks: ropey::iter::Chunks<'a>,
}

impl<'a> Iterator for Chunks<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        self.chunks.next().map(str::as_bytes)
    }
}

// A change made by the user reported by the editor.
#[derive(Debug)]
pub(crate) struct Edit {
    // The buffer that was changed.
    pub(crate) buffer: Buffer,
    // A tree-sitter InputEdit value, describing what part of the file was changed.
    pub(crate) input_edit: InputEdit,
    // Bytes representing the new contents of the file at the location described
    // by `input_edit`.
    pub(crate) new_bytes: String,
}

impl Edit {
    pub(crate) fn new(
        buffer: Buffer,
        bytes: &mut Rope,
        range: &Range<usize>,
        new_bytes: String,
    ) -> Edit {
        let new_end_byte = range.start + new_bytes.len();
        let start_position = byte_to_point(bytes, range.start);
        let old_end_position = byte_to_point(bytes, range.end);
        apply_edit_helper(bytes, range.start, range.end, &new_bytes);
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

    pub(crate) fn apply(&self, bytes: &mut Rope) {
        apply_edit_helper(
            bytes,
            self.input_edit.start_byte,
            self.input_edit.old_end_byte,
            &self.new_bytes,
        );
    }
}

fn apply_edit_helper(
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

pub(crate) fn byte_to_point(code: &Rope, byte: usize) -> tree_sitter::Point {
    let row = code.byte_to_line(byte);
    tree_sitter::Point {
        row,
        column: code.byte_to_char(byte) - code.line_to_char(row),
    }
}
