use crate::elm;
use std::collections::HashMap;
use tree_sitter::{Node, Tree, TreeCursor};

#[derive(Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum SrcLocCrumb {
    ModuleRoot,
    Name(String),
}

#[derive(Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct SrcLoc(usize);

#[derive(Debug)]
pub enum TypeRelation {
    SameAs,
}

pub struct SrcLocs(HashMap<Vec<SrcLocCrumb>, SrcLoc>);

impl SrcLocs {
    fn new() -> SrcLocs {
        SrcLocs(HashMap::new())
    }

    fn from(&mut self, path: Vec<SrcLocCrumb>) -> SrcLoc {
        let len = self.0.len();
        *self.0.entry(path).or_insert(SrcLoc(len))
    }
}

pub fn scan_tree(
    tree: Tree,
    bytes: &[u8],
    src_locs: &mut SrcLocs,
    relations: &mut Vec<(SrcLoc, TypeRelation, SrcLoc)>,
) {
    let mut cursor = tree.walk();
    scan_root(&mut cursor, bytes, src_locs, relations)
}

pub fn scan_root(
    cursor: &mut TreeCursor,
    bytes: &[u8],
    src_locs: &mut SrcLocs,
    relations: &mut Vec<(SrcLoc, TypeRelation, SrcLoc)>,
) {
    // TODO: Remove asserts in favor of logging errors.
    let path = vec![SrcLocCrumb::ModuleRoot];
    let node = cursor.node();
    assert_eq!(node.kind_id(), elm::FILE);
    if cursor.goto_first_child() {
        loop {
            scan_node(cursor, bytes, &path, src_locs, relations);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

pub fn scan_node(
    cursor: &mut TreeCursor,
    bytes: &[u8],
    path: &[SrcLocCrumb],
    src_locs: &mut SrcLocs,
    relations: &mut Vec<(SrcLoc, TypeRelation, SrcLoc)>,
) {
    let node = cursor.node();
    match node.kind_id() {
        elm::MODULE_DECLARATION => {}
        elm::IMPORT_CLAUSE => {}
        elm::VALUE_DECLARATION => {}
        elm::TYPE_ALIAS_DECLARATION => todo!(),
        elm::TYPE_DECLARATION => todo!(),
        elm::TYPE_ANNOTATION => scan_type_annotation(
            &cursor.node(),
            bytes,
            path,
            src_locs,
            relations,
        ),
        elm::PORT_ANNOTATION => todo!(),
        elm::INFIX_DECLARATION => todo!(),
        elm::LINE_COMMENT => {}
        elm::BLOCK_COMMENT => {}
        _ => todo!(),
    }
}

fn scan_type_annotation(
    node: &Node,
    bytes: &[u8],
    path: &[SrcLocCrumb],
    src_locs: &mut SrcLocs,
    relations: &mut Vec<(SrcLoc, TypeRelation, SrcLoc)>,
) {
    let name_node = node.child_by_field_name("name").unwrap();
    let name = String::from_utf8(bytes[name_node.byte_range()].into()).unwrap();
    let loc = src_locs.from(vec![path[0].clone(), SrcLocCrumb::Name(name)]);
    let type_node = node.child_by_field_name("typeExpression").unwrap();
    let mut cursor = type_node.walk();
    let child_kinds: Vec<u16> = type_node
        .children(&mut cursor)
        .map(|n| n.kind_id())
        .collect();
    match child_kinds.as_slice() {
        [elm::TYPE_REF] => {
            let type_name =
                String::from_utf8(bytes[type_node.byte_range()].into())
                    .unwrap();
            let type_loc = src_locs
                .from(vec![path[0].clone(), SrcLocCrumb::Name(type_name)]);
            relations.push((loc, TypeRelation::SameAs, type_loc));
        }
        _ => {
            dbg!(child_kinds);
            todo!();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lib::included_answer_test::assert_eq_answer_in;
    use crate::lib::source_code::parse_bytes;
    use std::path::PathBuf;

    #[test]
    fn first_test() {
        let path = PathBuf::from("./tests/type-checking/Test.elm");
        let bytes = std::fs::read(&path).unwrap();
        let tree = parse_bytes(bytes.clone()).unwrap();
        let mut relations = Vec::new();
        let mut src_locs = SrcLocs::new();
        scan_tree(tree, &bytes, &mut src_locs, &mut relations);
        let mut output = "Source Locations\n".to_string();
        for (crumbs, SrcLoc(id)) in src_locs.0.into_iter() {
            output.push_str(&format!("{}: {:?}\n", id, crumbs));
        }
        output.push_str("\nRelations\n");
        for (SrcLoc(from), rel, SrcLoc(to)) in relations.into_iter() {
            output.push_str(&format!("{} `{:?}` {}\n", from, rel, to));
        }
        assert_eq_answer_in(&output, &path);
    }
}
