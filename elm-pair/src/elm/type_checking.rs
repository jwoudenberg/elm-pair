use crate::elm;
use crate::lib::dataflow;
use abomonation_derive::Abomonation;
use differential_dataflow::operators::iterate::Iterate;
use differential_dataflow::operators::join::Join;
use differential_dataflow::operators::Threshold;
use tree_sitter::{Node, Tree, TreeCursor};

#[derive(Abomonation, Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum SrcLoc {
    Name(String),
    ArgTo(String),
    ResultOf(String),
}

#[derive(Debug)]
pub enum TypeRelation {
    SameAs,
    ArgTo,
    ResultOf,
}

type Type = String;

pub fn dataflow_graph<'a>(
    starter_types: dataflow::Collection<'a, (SrcLoc, Type)>,
    same_as: dataflow::Collection<'a, (SrcLoc, SrcLoc)>,
    arg_to: dataflow::Collection<'a, (SrcLoc, SrcLoc)>,
    result_of: dataflow::Collection<'a, (SrcLoc, SrcLoc)>,
) -> dataflow::Collection<'a, (String, Type)> {
    let same_as_bidirectional =
        same_as.concat(&same_as.map(|(x, y)| (y, x))).distinct();

    let types = starter_types.iterate(|transative| {
        let same_as_local = same_as_bidirectional.enter(&transative.scope());
        let arg_to_local = arg_to.enter(&transative.scope());
        let result_of_local = result_of.enter(&transative.scope());

        let arg_types = arg_to_local
            .join_map(transative, |_, k, type_| (k.clone(), type_.clone()));

        let res_types = result_of_local
            .join_map(transative, |_, k, type_| (k.clone(), type_.clone()));

        let new_fn_types = arg_types.join_map(&res_types, |k, arg, res| {
            (k.clone(), format!("{arg} -> {res}"))
        });

        let new_arg_types = arg_to_local.map(|(arg, f)| (f, arg)).join_map(
            transative,
            |_, arg, f_type| {
                (
                    arg.clone(),
                    f_type.split_once("->").unwrap().0.trim_end().to_string(),
                )
            },
        );

        let new_result_types = result_of_local
            .map(|(result, f)| (f, result))
            .join_map(transative, |_, result, f_type| {
                (
                    result.clone(),
                    f_type.split_once("->").unwrap().1.trim_start().to_string(),
                )
            });

        let new_from_same_as = transative
            .join_map(&same_as_local, |_, type_, same_as| {
                (same_as.clone(), type_.clone())
            });

        transative
            .concat(&new_from_same_as)
            .concat(&new_fn_types)
            .concat(&new_arg_types)
            .concat(&new_result_types)
            .distinct()
    });

    types.flat_map(|(loc, type_)| match loc {
        SrcLoc::Name(name) => Some((name, type_)),
        SrcLoc::ArgTo(_) => None,
        SrcLoc::ResultOf(_) => None,
    })
}

pub fn scan_tree(
    tree: Tree,
    bytes: &[u8],
    relations: &mut Vec<(SrcLoc, TypeRelation, SrcLoc)>,
) {
    let mut cursor = tree.walk();
    scan_root(&mut cursor, bytes, relations)
}

pub fn scan_root(
    cursor: &mut TreeCursor,
    bytes: &[u8],
    relations: &mut Vec<(SrcLoc, TypeRelation, SrcLoc)>,
) {
    // TODO: Remove asserts in favor of logging errors.
    let node = cursor.node();
    assert_eq!(node.kind_id(), elm::FILE);
    if cursor.goto_first_child() {
        loop {
            scan_node(cursor, bytes, relations);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

pub fn scan_node(
    cursor: &mut TreeCursor,
    bytes: &[u8],
    relations: &mut Vec<(SrcLoc, TypeRelation, SrcLoc)>,
) {
    let node = cursor.node();
    match node.kind_id() {
        elm::MODULE_DECLARATION => {}
        elm::IMPORT_CLAUSE => {}
        elm::VALUE_DECLARATION => {
            scan_value_declaration(&cursor.node(), bytes, relations)
        }
        elm::TYPE_ALIAS_DECLARATION => todo!(),
        elm::TYPE_DECLARATION => todo!(),
        elm::TYPE_ANNOTATION => {
            scan_type_annotation(&cursor.node(), bytes, relations)
        }
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
    relations: &mut Vec<(SrcLoc, TypeRelation, SrcLoc)>,
) {
    let func_node =
        node.child_by_field_name("functionDeclarationLeft").unwrap();
    let name_node = func_node.child(0).unwrap();
    let name = String::from_utf8(bytes[name_node.byte_range()].into()).unwrap();
    let loc = SrcLoc::Name(name.clone());

    let arg_node = func_node.child(1).unwrap();
    let arg_name =
        String::from_utf8(bytes[arg_node.byte_range()].into()).unwrap();
    let arg_loc = SrcLoc::Name(arg_name);
    relations.push((arg_loc, TypeRelation::ArgTo, loc));

    let res_loc = SrcLoc::ResultOf(name);

    let body_node = node.child_by_field_name("body").unwrap();
    match body_node.kind_id() {
        elm::FUNCTION_CALL_EXPR => {
            let fn_name_node = body_node.child_by_field_name("target").unwrap();
            let fn_name =
                String::from_utf8(bytes[fn_name_node.byte_range()].into())
                    .unwrap();
            let fn_name_loc = SrcLoc::Name(fn_name.clone());

            let fn_arg_node = body_node.child_by_field_name("arg").unwrap();
            let fn_arg_name =
                String::from_utf8(bytes[fn_arg_node.byte_range()].into())
                    .unwrap();
            let fn_arg_loc = SrcLoc::Name(fn_arg_name);

            let fn_res_loc = SrcLoc::ResultOf(fn_name);

            relations.push((
                fn_arg_loc,
                TypeRelation::ArgTo,
                fn_name_loc.clone(),
            ));
            relations.push((res_loc.clone(), TypeRelation::SameAs, fn_res_loc));
            relations.push((res_loc, TypeRelation::ResultOf, fn_name_loc));
        }
        _ => todo!(),
    }
}

fn scan_type_annotation(
    node: &Node,
    bytes: &[u8],
    relations: &mut Vec<(SrcLoc, TypeRelation, SrcLoc)>,
) {
    let name_node = node.child_by_field_name("name").unwrap();
    let name = String::from_utf8(bytes[name_node.byte_range()].into()).unwrap();
    let loc = SrcLoc::Name(name.clone());
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
            let type_loc = SrcLoc::Name(type_name);
            relations.push((loc, TypeRelation::SameAs, type_loc));
        }
        [elm::TYPE_REF, elm::ARROW, elm::TYPE_REF] => {
            let arg_name =
                String::from_utf8(bytes[children[0].byte_range()].into())
                    .unwrap();
            let arg_loc = SrcLoc::ArgTo(name.clone());
            relations.push((
                arg_loc.clone(),
                TypeRelation::SameAs,
                SrcLoc::Name(arg_name),
            ));
            relations.push((arg_loc, TypeRelation::ArgTo, loc.clone()));

            let res_name =
                String::from_utf8(bytes[children[2].byte_range()].into())
                    .unwrap();
            let res_loc = SrcLoc::ResultOf(name);
            relations.push((
                res_loc.clone(),
                TypeRelation::SameAs,
                SrcLoc::Name(res_name),
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
    use differential_dataflow::operators::arrange::ArrangeByKey;
    use differential_dataflow::trace::cursor::CursorDebug;
    use differential_dataflow::trace::TraceReader;
    use std::path::PathBuf;
    use timely::dataflow::operators::Probe;

    #[test]
    fn first_test() {
        let path = PathBuf::from("./tests/type-checking/Test.elm");
        let bytes = std::fs::read(&path).unwrap();
        let tree = parse_bytes(bytes.clone()).unwrap();
        let mut relations = Vec::new();
        scan_tree(tree, &bytes, &mut relations);
        let mut output = String::new();
        for (from, rel, to) in relations.into_iter() {
            output.push_str(&format!("{:?} `{:?}` {:?}\n", &from, rel, &to,));
        }
        assert_eq_answer_in(&output, &path);
    }

    #[test]
    fn dataflow_test() {
        let path = PathBuf::from("./tests/type-checking/Test2.elm");
        let bytes = std::fs::read(&path).unwrap();
        let tree = parse_bytes(bytes.clone()).unwrap();
        let mut relations = Vec::new();
        scan_tree(tree, &bytes, &mut relations);

        let alloc = timely::communication::allocator::thread::Thread::new();
        let mut worker =
            timely::worker::Worker::new(timely::WorkerConfig::default(), alloc);
        let mut starter_types_input =
            differential_dataflow::input::InputSession::new();
        let mut same_as_input =
            differential_dataflow::input::InputSession::new();
        let mut arg_to_input =
            differential_dataflow::input::InputSession::new();
        let mut result_of_input =
            differential_dataflow::input::InputSession::new();
        let (mut types_trace, mut probe) = worker.dataflow(|scope| {
            let starter_types = starter_types_input.to_collection(scope);
            let same_as = same_as_input.to_collection(scope);
            let arg_to = arg_to_input.to_collection(scope);
            let result_of = result_of_input.to_collection(scope);
            let types =
                dataflow_graph(starter_types, same_as, arg_to, result_of);
            let types_agg = types.arrange_by_key();
            (types_agg.trace, types_agg.stream.probe())
        });

        let int_loc = SrcLoc::Name("Int".to_string());
        let string_loc = SrcLoc::Name("String".to_string());
        starter_types_input.insert((int_loc, "Int".to_string()));
        starter_types_input.insert((string_loc, "String".to_string()));
        for (from, rel, to) in relations.into_iter() {
            match rel {
                TypeRelation::SameAs => same_as_input.insert((from, to)),
                TypeRelation::ArgTo => arg_to_input.insert((from, to)),
                TypeRelation::ResultOf => result_of_input.insert((from, to)),
            }
        }

        dataflow::Advancable::advance(
            &mut (
                &mut types_trace,
                &mut probe,
                &mut starter_types_input,
                &mut same_as_input,
                &mut arg_to_input,
                &mut result_of_input,
            ),
            &mut worker,
        );

        let (mut cursor, storage) = types_trace.cursor();

        let mut types: Vec<(String, Type)> = cursor
            .to_vec(&storage)
            .into_iter()
            .filter_map(|(i, counts)| {
                let total: isize =
                    counts.into_iter().map(|(_, count)| count).sum();
                if total > 0 {
                    Some(i)
                } else {
                    None
                }
            })
            .collect();
        types.sort();
        let mut output = String::new();
        for (loc, type_) in types.into_iter() {
            output.push_str(&format!("{loc} : {type_}\n"));
        }
        assert_eq_answer_in(&output, &path);
    }
}
