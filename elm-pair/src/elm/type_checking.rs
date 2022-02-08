use crate::elm;
use std::collections::HashMap;
use tree_sitter::{Node, Tree, TreeCursor};

#[derive(Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum SrcLocCrumb {
    ModuleRoot,
    Name(String),
    LambdaArg,
    LambdaRes,
}

#[derive(Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct SrcLoc(usize);

#[derive(Debug)]
pub enum TypeRelation {
    SameAs,
    ArgTo,
    ResultOf,
}

pub struct SrcLocs(HashMap<(Option<SrcLoc>, SrcLocCrumb), SrcLoc>);

impl SrcLocs {
    fn new() -> SrcLocs {
        SrcLocs(HashMap::new())
    }

    fn from(&mut self, parent: Option<SrcLoc>, child: SrcLocCrumb) -> SrcLoc {
        let len = self.0.len();
        *self.0.entry((parent, child)).or_insert(SrcLoc(len))
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
    let loc = src_locs.from(None, SrcLocCrumb::ModuleRoot);
    let node = cursor.node();
    assert_eq!(node.kind_id(), elm::FILE);
    if cursor.goto_first_child() {
        loop {
            scan_node(cursor, bytes, loc, src_locs, relations);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

pub fn scan_node(
    cursor: &mut TreeCursor,
    bytes: &[u8],
    loc: SrcLoc,
    src_locs: &mut SrcLocs,
    relations: &mut Vec<(SrcLoc, TypeRelation, SrcLoc)>,
) {
    let node = cursor.node();
    match node.kind_id() {
        elm::MODULE_DECLARATION => {}
        elm::IMPORT_CLAUSE => {}
        elm::VALUE_DECLARATION => scan_value_declaration(
            &cursor.node(),
            bytes,
            loc,
            src_locs,
            relations,
        ),
        elm::TYPE_ALIAS_DECLARATION => todo!(),
        elm::TYPE_DECLARATION => todo!(),
        elm::TYPE_ANNOTATION => scan_type_annotation(
            &cursor.node(),
            bytes,
            loc,
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

fn scan_value_declaration(
    node: &Node,
    bytes: &[u8],
    parent: SrcLoc,
    src_locs: &mut SrcLocs,
    relations: &mut Vec<(SrcLoc, TypeRelation, SrcLoc)>,
) {
    let func_node =
        node.child_by_field_name("functionDeclarationLeft").unwrap();
    let name_node = func_node.child(0).unwrap();
    let name = String::from_utf8(bytes[name_node.byte_range()].into()).unwrap();
    let loc = src_locs.from(Some(parent), SrcLocCrumb::Name(name));

    let arg_node = func_node.child(1).unwrap();
    let arg_name =
        String::from_utf8(bytes[arg_node.byte_range()].into()).unwrap();
    let arg_loc = src_locs.from(Some(loc), SrcLocCrumb::Name(arg_name));
    relations.push((arg_loc, TypeRelation::ArgTo, loc));

    let res_loc = src_locs.from(Some(loc), SrcLocCrumb::LambdaRes);

    let body_node = node.child_by_field_name("body").unwrap();
    match body_node.kind_id() {
        elm::FUNCTION_CALL_EXPR => {
            let fn_name_node = body_node.child_by_field_name("target").unwrap();
            let fn_name =
                String::from_utf8(bytes[fn_name_node.byte_range()].into())
                    .unwrap();
            let fn_name_loc =
                src_locs.from(Some(parent), SrcLocCrumb::Name(fn_name));

            let fn_arg_node = body_node.child_by_field_name("arg").unwrap();
            let fn_arg_name =
                String::from_utf8(bytes[fn_arg_node.byte_range()].into())
                    .unwrap();
            let fn_arg_loc =
                src_locs.from(Some(parent), SrcLocCrumb::Name(fn_arg_name));

            let fn_res_loc =
                src_locs.from(Some(fn_name_loc), SrcLocCrumb::LambdaRes);

            relations.push((fn_arg_loc, TypeRelation::ArgTo, fn_name_loc));
            relations.push((res_loc, TypeRelation::SameAs, fn_res_loc));
            relations.push((res_loc, TypeRelation::ResultOf, fn_name_loc));
        }
        _ => todo!(),
    }
}

fn scan_type_annotation(
    node: &Node,
    bytes: &[u8],
    parent: SrcLoc,
    src_locs: &mut SrcLocs,
    relations: &mut Vec<(SrcLoc, TypeRelation, SrcLoc)>,
) {
    let name_node = node.child_by_field_name("name").unwrap();
    let name = String::from_utf8(bytes[name_node.byte_range()].into()).unwrap();
    let loc = src_locs.from(Some(parent), SrcLocCrumb::Name(name));
    let type_node = node.child_by_field_name("typeExpression").unwrap();
    let mut cursor = type_node.walk();
    let children: Vec<Node> = type_node.children(&mut cursor).collect();
    match children
        .iter()
        .map(|n| n.kind_id())
        .collect::<Vec<u16>>()
        .as_slice()
    {
        [elm::TYPE_REF] => {
            let type_name =
                String::from_utf8(bytes[type_node.byte_range()].into())
                    .unwrap();
            let type_loc =
                src_locs.from(Some(loc), SrcLocCrumb::Name(type_name));
            relations.push((loc, TypeRelation::SameAs, type_loc));
        }
        [elm::TYPE_REF, elm::ARROW, elm::TYPE_REF] => {
            let arg_name =
                String::from_utf8(bytes[children[0].byte_range()].into())
                    .unwrap();
            let arg_loc = src_locs.from(Some(loc), SrcLocCrumb::LambdaArg);
            relations.push((
                arg_loc,
                TypeRelation::SameAs,
                src_locs.from(Some(parent), SrcLocCrumb::Name(arg_name)),
            ));
            relations.push((arg_loc, TypeRelation::ArgTo, loc));

            let res_name =
                String::from_utf8(bytes[children[2].byte_range()].into())
                    .unwrap();
            let res_loc = src_locs.from(Some(loc), SrcLocCrumb::LambdaRes);
            relations.push((
                res_loc,
                TypeRelation::SameAs,
                src_locs.from(Some(parent), SrcLocCrumb::Name(res_name)),
            ));
            relations.push((res_loc, TypeRelation::ResultOf, loc));
        }
        child_kinds => {
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

    fn mk_loc_lookup(
        locs: SrcLocs,
    ) -> HashMap<SrcLoc, (Option<SrcLoc>, SrcLocCrumb)> {
        locs.0.into_iter().map(|(k, v)| (v, k)).collect()
    }

    fn loc_as_str(
        loc: &SrcLoc,
        loc_lookup: &HashMap<SrcLoc, (Option<SrcLoc>, SrcLocCrumb)>,
    ) -> String {
        match loc_lookup.get(loc) {
            Some((None, crumb)) => format!("/{crumb:?}"),
            Some((Some(parent), crumb)) => {
                format!("{}/{crumb:?}", loc_as_str(parent, loc_lookup))
            }
            None => panic!(),
        }
    }

    #[test]
    fn first_test() {
        let path = PathBuf::from("./tests/type-checking/Test.elm");
        let bytes = std::fs::read(&path).unwrap();
        let tree = parse_bytes(bytes.clone()).unwrap();
        let mut relations = Vec::new();
        let mut src_locs = SrcLocs::new();
        scan_tree(tree, &bytes, &mut src_locs, &mut relations);
        let loc_lookup = mk_loc_lookup(src_locs);
        let mut output = "Relations\n".to_string();
        for (from, rel, to) in relations.into_iter() {
            output.push_str(&format!(
                "{} `{:?}` {}\n",
                loc_as_str(&from, &loc_lookup),
                rel,
                loc_as_str(&to, &loc_lookup)
            ));
        }
        assert_eq_answer_in(&output, &path);
    }
}
