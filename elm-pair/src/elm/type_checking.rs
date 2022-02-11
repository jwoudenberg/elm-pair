use crate::elm;
use crate::lib::dataflow;
use crate::lib::log;
use abomonation_derive::Abomonation;
use bimap::BiMap;
use differential_dataflow::operators::iterate::Iterate;
use differential_dataflow::operators::join::Join;
use differential_dataflow::operators::Threshold;
use tree_sitter::{Node, Tree, TreeCursor};

#[derive(
    Abomonation, Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord,
)]
pub enum Loc {
    Name(Name),
    ArgTo(LocRef),
    ResultOf(LocRef),
}

#[derive(
    Abomonation, Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord,
)]
pub struct LocRef(usize);

pub struct LocRefs(BiMap<Loc, LocRef>);

impl LocRefs {
    pub fn new() -> LocRefs {
        LocRefs(BiMap::new())
    }

    fn arg_to(&mut self, src_loc: Loc) -> Loc {
        let ref_ = self.get_ref(src_loc);
        Loc::ArgTo(ref_)
    }

    fn result_of(&mut self, src_loc: Loc) -> Loc {
        let ref_ = self.get_ref(src_loc);
        Loc::ResultOf(ref_)
    }

    fn get_ref(&mut self, src_loc: Loc) -> LocRef {
        if let Some(ref_) = self.0.get_by_left(&src_loc) {
            return *ref_;
        }
        let ref_ = LocRef(self.0.len());
        self.0.insert(src_loc, ref_);
        ref_
    }

    fn get_loc(&self, ref_: LocRef) -> Loc {
        *self.0.get_by_right(&ref_).unwrap()
    }
}

#[derive(
    Abomonation, Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord,
)]
pub struct Name(usize);

pub struct Names(BiMap<String, Name>);

impl Names {
    pub fn new() -> Names {
        Names(BiMap::new())
    }

    pub fn from_str(&mut self, str: &str) -> Name {
        if let Some(name) = self.0.get_by_left(str) {
            return *name;
        }
        let name = Name(self.0.len());
        self.0.insert(str.to_string(), name);
        name
    }
}

pub fn node_name(names: &mut Names, bytes: &[u8], node: &Node) -> Name {
    let slice = &bytes[node.byte_range()];
    let str = std::str::from_utf8(slice).unwrap();
    names.from_str(str)
}

#[derive(Debug)]
pub enum TypeRelation {
    SameAs,
    ArgTo,
    ResultOf,
}

type Type = String;

pub fn dataflow_graph<'a>(
    starter_types: dataflow::Collection<'a, (Loc, Type)>,
    same_as: dataflow::Collection<'a, (Loc, Loc)>,
    arg_to: dataflow::Collection<'a, (Loc, Loc)>,
    result_of: dataflow::Collection<'a, (Loc, Loc)>,
) -> dataflow::Collection<'a, (Name, Type)> {
    let same_as_bidirectional =
        same_as.concat(&same_as.map(|(x, y)| (y, x))).distinct();

    let types = starter_types.iterate(|transative| {
        let same_as_local = same_as_bidirectional.enter(&transative.scope());
        let arg_to_local = arg_to.enter(&transative.scope());
        let result_of_local = result_of.enter(&transative.scope());

        let arg_types = arg_to_local
            .join_map(transative, |_, k, type_| (*k, type_.clone()));

        let res_types = result_of_local
            .join_map(transative, |_, k, type_| (*k, type_.clone()));

        let new_fn_types = arg_types.join_map(&res_types, |k, arg, res| {
            (*k, format!("{arg} -> {res}"))
        });

        let new_arg_types = arg_to_local.map(|(arg, f)| (f, arg)).join_map(
            transative,
            |_, arg, f_type| {
                (
                    *arg,
                    f_type.split_once("->").unwrap().0.trim_end().to_string(),
                )
            },
        );

        let new_result_types = result_of_local
            .map(|(result, f)| (f, result))
            .join_map(transative, |_, result, f_type| {
                (
                    *result,
                    f_type.split_once("->").unwrap().1.trim_start().to_string(),
                )
            });

        let new_from_same_as = transative
            .join_map(&same_as_local, |_, type_, same_as| {
                (*same_as, type_.clone())
            });

        transative
            .concat(&new_from_same_as)
            .concat(&new_fn_types)
            .concat(&new_arg_types)
            .concat(&new_result_types)
            .distinct()
    });

    types.flat_map(|(loc, type_)| match loc {
        Loc::Name(name) => Some((name, type_)),
        Loc::ArgTo(_) => None,
        Loc::ResultOf(_) => None,
    })
}

pub fn scan_tree(
    tree: Tree,
    bytes: &[u8],
    names: &mut Names,
    loc_refs: &mut LocRefs,
    relations: &mut Vec<(Loc, TypeRelation, Loc)>,
) {
    let mut cursor = tree.walk();
    scan_root(&mut cursor, bytes, names, loc_refs, relations)
}

pub fn scan_root(
    cursor: &mut TreeCursor,
    bytes: &[u8],
    names: &mut Names,
    loc_refs: &mut LocRefs,
    relations: &mut Vec<(Loc, TypeRelation, Loc)>,
) {
    // TODO: Remove asserts in favor of logging errors.
    let node = cursor.node();
    assert_eq!(node.kind_id(), elm::FILE);
    if cursor.goto_first_child() {
        loop {
            scan_node(cursor, bytes, names, loc_refs, relations);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

pub fn scan_node(
    cursor: &mut TreeCursor,
    bytes: &[u8],
    names: &mut Names,
    loc_refs: &mut LocRefs,
    relations: &mut Vec<(Loc, TypeRelation, Loc)>,
) {
    let node = cursor.node();
    match node.kind_id() {
        elm::MODULE_DECLARATION => {}
        elm::IMPORT_CLAUSE => {}
        elm::VALUE_DECLARATION => scan_value_declaration(
            &cursor.node(),
            bytes,
            names,
            loc_refs,
            relations,
        ),
        elm::TYPE_ALIAS_DECLARATION => todo!(),
        elm::TYPE_DECLARATION => todo!(),
        elm::TYPE_ANNOTATION => scan_type_annotation(
            &cursor.node(),
            bytes,
            names,
            loc_refs,
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
    names: &mut Names,
    loc_refs: &mut LocRefs,
    relations: &mut Vec<(Loc, TypeRelation, Loc)>,
) {
    let func_node =
        node.child_by_field_name("functionDeclarationLeft").unwrap();
    let mut cursor = func_node.walk();

    if !cursor.goto_first_child() {
        log::error!("found empty function declaration");
        return;
    }

    let name = node_name(names, bytes, &cursor.node());
    let loc = Loc::Name(name);
    let mut parent_loc = loc;

    // Scan the function argument list.
    while cursor.goto_next_sibling() {
        let arg_node = cursor.node();
        if arg_node.kind_id() != elm::LOWER_PATTERN {
            log::error!("unexpected kind {} in argument list", node.kind_id());
            return;
        }
        let arg_name = node_name(names, bytes, &arg_node);
        let arg_loc = Loc::Name(arg_name);
        relations.push((arg_loc, TypeRelation::ArgTo, parent_loc));
        parent_loc = loc_refs.result_of(parent_loc);
    }

    scan_expression(
        parent_loc,
        &node.child_by_field_name("body").unwrap(),
        bytes,
        names,
        loc_refs,
        relations,
    );
}

fn scan_expression(
    parent_loc: Loc,
    node: &Node,
    bytes: &[u8],
    names: &mut Names,
    loc_refs: &mut LocRefs,
    relations: &mut Vec<(Loc, TypeRelation, Loc)>,
) {
    match node.kind_id() {
        elm::FUNCTION_CALL_EXPR => {
            let mut cursor = node.walk();
            if !cursor.goto_first_child() {
                log::error!("found empty type expression");
                return;
            }

            if cursor.field_name() != Some("target") {
                log::error!(
                    "unexpected kind {} in function call expression",
                    cursor.node().kind_id()
                );
                return;
            }

            // TODO: Don't create Name loc here, because we don't know for sure
            // this is a name. It could be an expression, like: `(add 4)`.
            let fn_node = cursor.node();
            let fn_name = node_name(names, bytes, &fn_node);
            let mut fn_loc = Loc::Name(fn_name);

            // Scan expression representing function to be called.
            scan_expression(
                fn_loc,
                &cursor.node(),
                bytes,
                names,
                loc_refs,
                relations,
            );

            fn to_next_arg_node(c: &mut TreeCursor) -> bool {
                proceed_to_sibling(c, |c_| c_.field_name() == Some("arg"))
            }

            while to_next_arg_node(&mut cursor) {
                let fn_arg_loc = loc_refs.arg_to(fn_loc);
                relations.push((fn_arg_loc, TypeRelation::ArgTo, fn_loc));

                scan_expression(
                    fn_arg_loc,
                    &cursor.node(),
                    bytes,
                    names,
                    loc_refs,
                    relations,
                );

                fn_loc = loc_refs.result_of(fn_loc);
            }

            relations.push((parent_loc, TypeRelation::SameAs, fn_loc));
        }
        elm::BIN_OP_EXPR => {
            let mut cursor = node.walk();

            fn to_next_part_node(c: &mut TreeCursor) -> bool {
                proceed_to_sibling(c, |c_| c_.field_name() == Some("part"))
            }

            // First argument
            if !(cursor.goto_first_child()) {
                log::error!("found empty binop expression");
                return;
            }
            let arg1_node = cursor.node();

            // Operator
            if !to_next_part_node(&mut cursor) {
                log::error!("missing operator in binop expression");
                return;
            }
            let op_node = cursor.node();
            if op_node.kind_id() != elm::OPERATOR {
                log::error!(
                    "unexpected kind {} in operator position",
                    op_node.kind_id()
                );
                return;
            }

            // Second argument
            if !to_next_part_node(&mut cursor) {
                log::error!("missing second arg in binop expression");
                return;
            }
            let arg2_node = cursor.node();

            // Add relations between nodes.
            let op_slice = &bytes[op_node.byte_range()];
            let op_str =
                format!("({})", std::str::from_utf8(op_slice).unwrap());
            let op_name = names.from_str(&op_str);
            let op_loc = Loc::Name(op_name);
            let arg1_loc = loc_refs.arg_to(op_loc);
            let partial_res_loc = loc_refs.result_of(op_loc);
            let arg2_loc = loc_refs.arg_to(partial_res_loc);
            let res_loc = loc_refs.result_of(partial_res_loc);
            relations.push((arg1_loc, TypeRelation::ArgTo, op_loc));
            relations.push((partial_res_loc, TypeRelation::ResultOf, op_loc));
            relations.push((arg2_loc, TypeRelation::ArgTo, partial_res_loc));
            relations.push((res_loc, TypeRelation::ResultOf, partial_res_loc));
            relations.push((res_loc, TypeRelation::SameAs, parent_loc));

            scan_expression(
                arg1_loc, &arg1_node, bytes, names, loc_refs, relations,
            );

            scan_expression(
                arg2_loc, &arg2_node, bytes, names, loc_refs, relations,
            );
        }
        elm::VALUE_EXPR => {
            let name = node_name(names, bytes, node);
            let loc = Loc::Name(name);
            relations.push((parent_loc, TypeRelation::SameAs, loc));
        }
        elm::PARENTHESIZED_EXPR => {
            scan_expression(
                parent_loc,
                &node.child_by_field_name("expression").unwrap(),
                bytes,
                names,
                loc_refs,
                relations,
            );
        }
        _ => todo!(),
    }
}

fn scan_type_annotation(
    node: &Node,
    bytes: &[u8],
    names: &mut Names,
    loc_refs: &mut LocRefs,
    relations: &mut Vec<(Loc, TypeRelation, Loc)>,
) {
    let name_node = node.child_by_field_name("name").unwrap();
    let name = node_name(names, bytes, &name_node);
    let loc = Loc::Name(name);
    let type_node = node.child_by_field_name("typeExpression").unwrap();
    let mut cursor = type_node.walk();

    fn to_next_arg_node(c: &mut TreeCursor) -> bool {
        proceed_to_sibling(c, |c_| c_.field_name() == Some("part"))
    }

    if !cursor.goto_first_child() {
        log::error!("found empty type expression");
        return;
    }

    let mut type_segment_node = cursor.node();
    let mut parent_loc = loc;

    // Keep the cursor one argument ahead of `type_segment_node` to detect when
    // a node segment is the final one, i.e. not an argument but a return type.
    while to_next_arg_node(&mut cursor) {
        if type_segment_node.kind_id() != elm::TYPE_REF {
            log::error!(
                "unexpected kind {} in type expression",
                type_segment_node.kind_id()
            );
            return;
        }
        let arg_name = node_name(names, bytes, &type_segment_node);
        let arg_loc = loc_refs.arg_to(parent_loc);
        relations.push((arg_loc, TypeRelation::SameAs, Loc::Name(arg_name)));
        relations.push((arg_loc, TypeRelation::ArgTo, parent_loc));
        let res_loc = loc_refs.result_of(parent_loc);
        relations.push((res_loc, TypeRelation::ResultOf, parent_loc));
        parent_loc = res_loc;
        type_segment_node = cursor.node();
    }

    if parent_loc == loc {
        // No arguments. The single type segment is the type of the definition.
        let type_name = node_name(names, bytes, &type_segment_node);
        let type_loc = Loc::Name(type_name);
        relations.push((loc, TypeRelation::SameAs, type_loc));
    } else {
        // We've seen arguments. This final type segment is the return type.
        let res_name = node_name(names, bytes, &type_segment_node);
        relations.push((parent_loc, TypeRelation::SameAs, Loc::Name(res_name)));
    }
}

fn proceed_to_sibling<F>(cursor: &mut TreeCursor, predicate: F) -> bool
where
    F: Fn(&TreeCursor) -> bool,
{
    while cursor.goto_next_sibling() {
        if predicate(cursor) {
            return true;
        }
    }
    false
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
        let mut names = Names::new();
        let mut loc_refs = LocRefs::new();
        let mut relations = Vec::new();
        scan_tree(tree, &bytes, &mut names, &mut loc_refs, &mut relations);
        let mut relation_strs: Vec<String> = relations
            .into_iter()
            .map(|(from, rel, to)| {
                format!(
                    "{} `{:?}` {}\n",
                    print_loc(&names, &loc_refs, from),
                    rel,
                    print_loc(&names, &loc_refs, to),
                )
            })
            .collect();
        relation_strs.sort();
        let output: String = relation_strs.into_iter().collect();
        assert_eq_answer_in(&output, &path);
    }

    #[test]
    fn dataflow_test() {
        let path = PathBuf::from("./tests/type-checking/Test2.elm");
        let bytes = std::fs::read(&path).unwrap();
        let tree = parse_bytes(bytes.clone()).unwrap();
        let mut names = Names::new();
        let mut loc_refs = LocRefs::new();
        let mut relations = Vec::new();
        scan_tree(tree, &bytes, &mut names, &mut loc_refs, &mut relations);

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

        let int_loc = Loc::Name(names.from_str("Int"));
        let string_loc = Loc::Name(names.from_str("String"));
        let append_loc = Loc::Name(names.from_str("(++)"));
        let length_loc = Loc::Name(names.from_str("List.length"));
        starter_types_input.insert((int_loc, "Int".to_string()));
        starter_types_input.insert((string_loc, "String".to_string()));
        starter_types_input.insert((length_loc, "String -> Int".to_string()));
        // starter_types_input
        //     .insert((append_loc, "String -> String -> String".to_string()));
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

        let mut types: Vec<(Name, Type)> = cursor
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
        for (name, type_) in types.into_iter() {
            let name_str = names.0.get_by_right(&name).unwrap();
            output.push_str(&format!("{name_str} : {type_}\n"));
        }
        assert_eq_answer_in(&output, &path);
    }

    fn print_loc(names: &Names, loc_refs: &LocRefs, loc: Loc) -> String {
        match loc {
            Loc::Name(name) => names.0.get_by_right(&name).unwrap().to_string(),
            Loc::ArgTo(ref_) => {
                format!(
                    "ArgTo({})",
                    print_loc(names, loc_refs, loc_refs.get_loc(ref_))
                )
            }
            Loc::ResultOf(ref_) => {
                format!(
                    "ResultOf({})",
                    print_loc(names, loc_refs, loc_refs.get_loc(ref_))
                )
            }
        }
    }
}
