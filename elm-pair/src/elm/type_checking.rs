use crate::elm;
use crate::lib::dataflow;
use crate::lib::log;
use abomonation_derive::Abomonation;
use bimap::BiMap;
use differential_dataflow::operators::iterate::Iterate;
use differential_dataflow::operators::join::Join;
use differential_dataflow::operators::Reduce;
use differential_dataflow::operators::Threshold;
use std::rc::Rc;
use std::sync::Mutex;
use tree_sitter::{Node, Tree, TreeCursor};

#[derive(
    Abomonation, Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord,
)]
pub enum Loc {
    Name { name: Name, scope: Scope },
    Hypothetical { name: Name, scope: Scope },
    ArgTo(LocRef),
    ResultOf(LocRef),
    IfCond(LocRef),
    IfTrue(LocRef),
    IfFalse(LocRef),
    CaseExpr(LocRef),
    CaseBranch(LocRef, usize),
    FnExpr(LocRef),
}

#[derive(
    Abomonation, Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord,
)]
pub enum Scope {
    Module,        // Top-level names. TODO: add the module name here.
    Local(LocRef), // Local names, defined in arguments, let-bindings, etc.
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

    fn if_cond(&mut self, src_loc: Loc) -> Loc {
        let ref_ = self.get_ref(src_loc);
        Loc::IfCond(ref_)
    }

    fn if_true(&mut self, src_loc: Loc) -> Loc {
        let ref_ = self.get_ref(src_loc);
        Loc::IfTrue(ref_)
    }

    fn if_false(&mut self, src_loc: Loc) -> Loc {
        let ref_ = self.get_ref(src_loc);
        Loc::IfFalse(ref_)
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

    #[cfg(test)]
    fn print(&self, names: &Names, loc: Loc) -> String {
        match loc {
            Loc::Hypothetical { name, scope } => {
                format!("{}?", self.print(names, Loc::Name { name, scope }),)
            }
            Loc::Name { name, scope } => match scope {
                Scope::Module => {
                    names.0.get_by_right(&name).unwrap().to_string()
                }
                Scope::Local(ref_) => format!(
                    "{}.{}",
                    self.print(names, self.get_loc(ref_)),
                    names.0.get_by_right(&name).unwrap(),
                ),
            },
            Loc::ArgTo(ref_) => {
                format!("{}.arg", self.print(names, self.get_loc(ref_)))
            }
            Loc::ResultOf(ref_) => {
                format!("{}.result", self.print(names, self.get_loc(ref_)))
            }
            Loc::IfCond(ref_) => {
                format!("{}.if_cond", self.print(names, self.get_loc(ref_)))
            }
            Loc::IfTrue(ref_) => {
                format!("{}.if_true", self.print(names, self.get_loc(ref_)))
            }
            Loc::IfFalse(ref_) => {
                format!("{}.if_false", self.print(names, self.get_loc(ref_)))
            }
            Loc::CaseExpr(ref_) => {
                format!("{}case_expr", self.print(names, self.get_loc(ref_)))
            }
            Loc::CaseBranch(ref_, i) => {
                format!(
                    "{}.case_branch_{i}",
                    self.print(names, self.get_loc(ref_))
                )
            }
            Loc::FnExpr(ref_) => {
                format!("{}.fn_expr", self.print(names, self.get_loc(ref_)))
            }
        }
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

type Type = String;

pub fn equivalences<'a>(
    loc_refs: Rc<Mutex<LocRefs>>,
    relations: &dataflow::Collection<'a, (Loc, Loc)>,
    starter_types: &dataflow::Collection<'a, (Loc, Type)>,
) -> dataflow::Collection<'a, (Loc, Loc)> {
    let names = relations
        .flat_map(|(x, y)| [x, y])
        .concat(&starter_types.map(|(loc, _)| loc))
        .iterate(|locs| {
            locs.flat_map(move |loc| match &loc {
                Loc::Name { .. } => None,
                Loc::Hypothetical { .. } => None,
                Loc::ArgTo(ref_) => {
                    Some(loc_refs.lock().unwrap().get_loc(*ref_))
                }
                Loc::ResultOf(ref_) => {
                    Some(loc_refs.lock().unwrap().get_loc(*ref_))
                }
                Loc::IfCond(ref_) => {
                    Some(loc_refs.lock().unwrap().get_loc(*ref_))
                }
                Loc::IfTrue(ref_) => {
                    Some(loc_refs.lock().unwrap().get_loc(*ref_))
                }
                Loc::IfFalse(ref_) => {
                    Some(loc_refs.lock().unwrap().get_loc(*ref_))
                }
                Loc::CaseExpr(ref_) => {
                    Some(loc_refs.lock().unwrap().get_loc(*ref_))
                }
                Loc::CaseBranch(ref_, _) => {
                    Some(loc_refs.lock().unwrap().get_loc(*ref_))
                }
                Loc::FnExpr(ref_) => {
                    Some(loc_refs.lock().unwrap().get_loc(*ref_))
                }
            })
            .concat(locs)
            .distinct()
        })
        .filter(|x| matches!(x, Loc::Name { .. }));

    let same_as_from_hypotheticals = relations
        .flat_map(|(x, y)| {
            if let Loc::Hypothetical { name, scope } = x {
                Some((Loc::Name { name, scope }, y))
            } else if let Loc::Hypothetical { name, scope } = y {
                Some((Loc::Name { name, scope }, x))
            } else {
                None
            }
        })
        .semijoin(&names);

    let same_as_without_hypotheticals = relations.flat_map(|(x, y)| {
        if matches!(x, Loc::Hypothetical { .. })
            || matches!(y, Loc::Hypothetical { .. })
        {
            None
        } else {
            Some((x, y))
        }
    });

    let same_as_all =
        same_as_from_hypotheticals.concat(&same_as_without_hypotheticals);

    same_as_all
        .concat(&same_as_all.map(|(x, y)| (y, x)))
        .distinct()
}

pub fn propagate_types<'a>(
    loc_refs: Rc<Mutex<LocRefs>>,
    starter_types: &dataflow::Collection<'a, (Loc, Type)>,
    same_as: &dataflow::Collection<'a, (Loc, Loc)>,
) -> dataflow::Collection<'a, (Loc, Type)> {
    starter_types.iterate(|types| {
        let same_as_local = same_as.enter(&types.scope());

        let new_from_same_as = types
            .join_map(&same_as_local, |_, type_, same_as| {
                (*same_as, type_.clone())
            });

        let loc_refs2 = loc_refs.clone();
        let arg_types = types.flat_map(move |(loc, type_)| {
            if let Loc::ArgTo(loc_ref) = loc {
                Some((loc_refs2.lock().unwrap().get_loc(loc_ref), type_))
            } else {
                None
            }
        });

        let loc_refs3 = loc_refs.clone();
        let result_types = types.flat_map(move |(loc, type_)| {
            if let Loc::ResultOf(loc_ref) = loc {
                Some((loc_refs3.lock().unwrap().get_loc(loc_ref), type_))
            } else {
                None
            }
        });
        let new_function_types = arg_types
            .join_map(&result_types, |loc, arg, result| {
                (*loc, format!("{arg} -> {result}"))
            });

        let loc_refs4 = loc_refs.clone();
        let new_arg_and_res_types = types.flat_map(move |(loc, type_)| {
            if let Some((arg, res)) = type_.split_once(" -> ") {
                let mut loc_refs_locked = loc_refs4.lock().unwrap();
                vec![
                    (loc_refs_locked.arg_to(loc), arg.to_string()),
                    (loc_refs_locked.result_of(loc), res.to_string()),
                ]
            } else {
                Vec::new()
            }
        });

        types
            .concat(&new_from_same_as)
            .concat(&new_function_types)
            .concat(&new_arg_and_res_types)
            .distinct()
    })
}

pub fn scan_tree(
    tree: Tree,
    bytes: &[u8],
    names: &mut Names,
    loc_refs: &mut LocRefs,
    relations: &mut Vec<(Loc, Loc)>,
) {
    let mut cursor = tree.walk();
    let scopes = vec![Scope::Module];
    scan_root(&mut cursor, bytes, names, loc_refs, &scopes, relations)
}

pub fn scan_root(
    cursor: &mut TreeCursor,
    bytes: &[u8],
    names: &mut Names,
    loc_refs: &mut LocRefs,
    scopes: &[Scope],
    relations: &mut Vec<(Loc, Loc)>,
) {
    // TODO: Remove asserts in favor of logging errors.
    let node = cursor.node();
    assert_eq!(node.kind_id(), elm::FILE);
    if cursor.goto_first_child() {
        loop {
            scan_node(cursor, bytes, names, loc_refs, scopes, relations);
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
    scopes: &[Scope],
    relations: &mut Vec<(Loc, Loc)>,
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
            scopes,
            relations,
        ),
        elm::TYPE_ALIAS_DECLARATION => todo!(),
        elm::TYPE_DECLARATION => todo!(),
        elm::TYPE_ANNOTATION => scan_type_annotation(
            &cursor.node(),
            bytes,
            names,
            loc_refs,
            scopes,
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
    scopes: &[Scope],
    relations: &mut Vec<(Loc, Loc)>,
) {
    let func_node =
        node.child_by_field_name("functionDeclarationLeft").unwrap();
    let mut cursor = func_node.walk();

    if !cursor.goto_first_child() {
        log::error!("found empty function declaration");
        return;
    }

    let name = node_name(names, bytes, &cursor.node());
    let loc = Loc::Name {
        name,
        scope: scopes[0],
    };
    let function_scope = Scope::Local(loc_refs.get_ref(loc));
    let mut parent_loc = loc;

    // Scan the function argument list.
    while cursor.goto_next_sibling() {
        let arg_node = cursor.node();
        if arg_node.kind_id() != elm::LOWER_PATTERN {
            log::error!("unexpected kind {} in argument list", node.kind_id());
            return;
        }
        let arg_name = node_name(names, bytes, &arg_node);
        let arg_loc = Loc::Name {
            name: arg_name,
            scope: function_scope,
        };
        relations.push((arg_loc, loc_refs.arg_to(parent_loc)));
        parent_loc = loc_refs.result_of(parent_loc);
    }

    let child_scopes: Vec<Scope> = std::iter::once(function_scope)
        .chain(scopes.iter().copied())
        .collect();

    scan_expression(
        parent_loc,
        &node.child_by_field_name("body").unwrap(),
        bytes,
        names,
        loc_refs,
        &child_scopes,
        relations,
    );
}

fn scan_expression(
    parent_loc: Loc,
    node: &Node,
    bytes: &[u8],
    names: &mut Names,
    loc_refs: &mut LocRefs,
    scopes: &[Scope],
    relations: &mut Vec<(Loc, Loc)>,
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

            let mut fn_loc = Loc::FnExpr(loc_refs.get_ref(parent_loc));

            // Scan expression representing function to be called.
            scan_expression(
                fn_loc,
                &cursor.node(),
                bytes,
                names,
                loc_refs,
                scopes,
                relations,
            );

            while to_next_field(&mut cursor, "arg") {
                let fn_arg_loc = loc_refs.arg_to(fn_loc);
                relations.push((fn_arg_loc, loc_refs.arg_to(fn_loc)));

                scan_expression(
                    fn_arg_loc,
                    &cursor.node(),
                    bytes,
                    names,
                    loc_refs,
                    scopes,
                    relations,
                );

                fn_loc = loc_refs.result_of(fn_loc);
            }

            relations.push((parent_loc, fn_loc));
        }
        elm::BIN_OP_EXPR => {
            let mut cursor = node.walk();

            // First argument
            if !(cursor.goto_first_child() && to_field(&mut cursor, "part")) {
                log::error!("found empty binop expression");
                return;
            }
            let arg1_node = cursor.node();

            // Operator
            if !to_next_field(&mut cursor, "part") {
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
            if !to_next_field(&mut cursor, "part") {
                log::error!("missing second arg in binop expression");
                return;
            }
            let arg2_node = cursor.node();

            // Add relations between nodes.
            let op_slice = &bytes[op_node.byte_range()];
            let op_str =
                format!("({})", std::str::from_utf8(op_slice).unwrap());
            let op_name = names.from_str(&op_str);
            let op_loc = Loc::Name {
                name: op_name,
                scope: Scope::Module,
            };
            let arg1_loc = loc_refs.arg_to(op_loc);
            let partial_res_loc = loc_refs.result_of(op_loc);
            let arg2_loc = loc_refs.arg_to(partial_res_loc);
            let res_loc = loc_refs.result_of(partial_res_loc);
            relations.push((arg1_loc, loc_refs.arg_to(op_loc)));
            relations.push((partial_res_loc, loc_refs.result_of(op_loc)));
            relations.push((arg2_loc, loc_refs.arg_to(partial_res_loc)));
            relations.push((res_loc, loc_refs.result_of(partial_res_loc)));
            relations.push((res_loc, parent_loc));

            scan_expression(
                arg1_loc, &arg1_node, bytes, names, loc_refs, scopes, relations,
            );

            scan_expression(
                arg2_loc, &arg2_node, bytes, names, loc_refs, scopes, relations,
            );
        }
        elm::VALUE_EXPR => {
            // We encountered a variable name usage. The name has the same type
            // as the variable definition so we should add an equality relation.
            // We don't know which scope the variable is defined in though,
            // without which we cannot construct the `Loc` of the definition.
            //
            // We know the stack of scopes this expression is in, so we can
            // create equality relations with all the hypothetical variable
            // definitions with the right name at all these scopes. Because Elm
            // does not allow variable shadowing we know that in compiling code
            // only one of those relations is going to match. The other
            // equality relations will 'dangle' harmlessly.
            let name = node_name(names, bytes, node);
            let new_relations = scopes
                .iter()
                .copied()
                .map(|scope| (parent_loc, Loc::Hypothetical { name, scope }));
            relations.extend(new_relations);
        }
        elm::STRING_CONSTANT_EXPR => {
            let string_name = names.from_str("String");
            let string_loc = Loc::Name {
                name: string_name,
                scope: Scope::Module,
            };
            relations.push((parent_loc, string_loc));
        }
        elm::NUMBER_CONSTANT_EXPR => {
            // TODO: Use knowledge that this literal is a `num`.
        }
        elm::PARENTHESIZED_EXPR => {
            scan_expression(
                parent_loc,
                &node.child_by_field_name("expression").unwrap(),
                bytes,
                names,
                loc_refs,
                scopes,
                relations,
            );
        }
        elm::IF_ELSE_EXPR => {
            let mut cursor = node.walk();

            if !(cursor.goto_first_child() && to_field(&mut cursor, "exprList"))
            {
                log::error!("found empty if expression");
                return;
            }
            let cond_node = cursor.node();

            if !to_next_field(&mut cursor, "exprList") {
                log::error!("if expression without true or false branch");
                return;
            }
            let true_node = cursor.node();

            if !to_next_field(&mut cursor, "exprList") {
                log::error!("if expression without false branch");
                return;
            }
            let false_node = cursor.node();

            let cond_loc = loc_refs.if_cond(parent_loc);
            let true_loc = loc_refs.if_true(parent_loc);
            let false_loc = loc_refs.if_false(parent_loc);
            let bool_loc = Loc::Name {
                name: names.from_str("Bool"),
                scope: Scope::Module,
            };
            relations.push((cond_loc, bool_loc));
            relations.push((true_loc, parent_loc));
            relations.push((false_loc, parent_loc));

            scan_expression(
                cond_loc, &cond_node, bytes, names, loc_refs, scopes, relations,
            );
            scan_expression(
                true_loc, &true_node, bytes, names, loc_refs, scopes, relations,
            );
            scan_expression(
                false_loc,
                &false_node,
                bytes,
                names,
                loc_refs,
                scopes,
                relations,
            );
        }
        elm::LET_IN_EXPR => {
            let let_scope = Scope::Local(loc_refs.get_ref(parent_loc));
            let child_scopes: Vec<Scope> = std::iter::once(let_scope)
                .chain(scopes.iter().copied())
                .collect();
            let mut cursor = node.walk();
            if !cursor.goto_first_child() {
                log::error!("found empty let expression");
                return;
            }

            // Scan let bindings.
            while cursor.field_name() != Some("body") {
                if cursor.field_name() == Some("valueDeclaration") {
                    scan_value_declaration(
                        &cursor.node(),
                        bytes,
                        names,
                        loc_refs,
                        &child_scopes,
                        relations,
                    );
                } else if cursor.node().kind_id() == elm::TYPE_ANNOTATION {
                    scan_type_annotation(
                        &cursor.node(),
                        bytes,
                        names,
                        loc_refs,
                        &child_scopes,
                        relations,
                    );
                }
                cursor.goto_next_sibling();
            }

            // Scan 'in' expression.
            if !to_field(&mut cursor, "body") {
                log::error!("let statement misses in expression");
                return;
            }
            scan_expression(
                parent_loc,
                &cursor.node(),
                bytes,
                names,
                loc_refs,
                &child_scopes,
                relations,
            );
        }
        elm::CASE_OF_EXPR => {
            let mut cursor = node.walk();
            if !cursor.goto_first_child() {
                log::error!("found empty case statement node");
                return;
            }

            if !to_field(&mut cursor, "expr") {
                log::error!("case statement without case expression");
                return;
            }
            let case_expr_loc = Loc::CaseExpr(loc_refs.get_ref(parent_loc));
            scan_expression(
                case_expr_loc,
                &cursor.node(),
                bytes,
                names,
                loc_refs,
                scopes,
                relations,
            );

            let mut branch_no = 0;
            while to_next_field(&mut cursor, "branch") {
                let parent_loc_ref = loc_refs.get_ref(parent_loc);
                let branch_loc = Loc::CaseBranch(parent_loc_ref, branch_no);
                let branch_scope = Scope::Local(loc_refs.get_ref(branch_loc));
                let child_scopes: Vec<Scope> = std::iter::once(branch_scope)
                    .chain(scopes.iter().copied())
                    .collect();
                let branch_node = cursor.node();
                let pattern_node =
                    branch_node.child_by_field_name("pattern").unwrap();
                scan_pattern(
                    case_expr_loc,
                    &pattern_node,
                    bytes,
                    names,
                    loc_refs,
                    &child_scopes,
                    relations,
                );
                let expr_node =
                    branch_node.child_by_field_name("expr").unwrap();
                scan_expression(
                    branch_loc,
                    &expr_node,
                    bytes,
                    names,
                    loc_refs,
                    &child_scopes,
                    relations,
                );
                relations.push((branch_loc, parent_loc));
                branch_no += 1;
            }
        }
        kind_id => {
            let language = tree_sitter_elm::language();
            let kind = language.node_kind_for_id(kind_id).unwrap();
            todo!("unimplemented expression kind {kind}")
        }
    }
}

fn scan_pattern(
    parent_loc: Loc,
    node: &Node,
    bytes: &[u8],
    names: &mut Names,
    loc_refs: &mut LocRefs,
    scopes: &[Scope],
    relations: &mut Vec<(Loc, Loc)>,
) {
    match node.kind_id() {
        elm::PATTERN => scan_pattern(
            parent_loc,
            &node.child_by_field_name("child").unwrap(),
            bytes,
            names,
            loc_refs,
            scopes,
            relations,
        ),
        elm::NUMBER_CONSTANT_EXPR => {
            // TODO: Use knowledge that this literal is a `num`.
        }
        elm::STRING_CONSTANT_EXPR => {
            let string_name = names.from_str("String");
            let string_loc = Loc::Name {
                name: string_name,
                scope: Scope::Module,
            };
            relations.push((parent_loc, string_loc));
        }
        elm::LOWER_PATTERN => {
            let name = node_name(names, bytes, node);
            let loc = Loc::Name {
                name,
                scope: scopes[0],
            };
            relations.push((loc, parent_loc));
        }
        kind_id => {
            let language = tree_sitter_elm::language();
            let kind = language.node_kind_for_id(kind_id).unwrap();
            todo!("unimplemented pattern kind {kind}")
        }
    }
}

fn scan_type_annotation(
    node: &Node,
    bytes: &[u8],
    names: &mut Names,
    loc_refs: &mut LocRefs,
    scopes: &[Scope],
    relations: &mut Vec<(Loc, Loc)>,
) {
    let name_node = node.child_by_field_name("name").unwrap();
    let name = node_name(names, bytes, &name_node);
    let loc = Loc::Name {
        name,
        scope: scopes[0],
    };
    let type_node = node.child_by_field_name("typeExpression").unwrap();
    let mut cursor = type_node.walk();

    if !cursor.goto_first_child() {
        log::error!("found empty type expression");
        return;
    }

    let mut type_segment_node = cursor.node();
    let mut parent_loc = loc;

    // Keep the cursor one argument ahead of `type_segment_node` to detect when
    // a node segment is the final one, i.e. not an argument but a return type.
    while to_next_field(&mut cursor, "part") {
        if type_segment_node.kind_id() != elm::TYPE_REF {
            log::error!(
                "unexpected kind {} in type expression",
                type_segment_node.kind_id()
            );
            return;
        }
        let arg_name = node_name(names, bytes, &type_segment_node);
        let arg_loc = loc_refs.arg_to(parent_loc);
        relations.push((
            arg_loc,
            Loc::Name {
                name: arg_name,
                scope: scopes[0],
            },
        ));
        relations.push((arg_loc, loc_refs.arg_to(parent_loc)));
        let res_loc = loc_refs.result_of(parent_loc);
        relations.push((res_loc, loc_refs.result_of(parent_loc)));
        parent_loc = res_loc;
        type_segment_node = cursor.node();
    }

    if parent_loc == loc {
        // No arguments. The single type segment is the type of the definition.
        let type_name = node_name(names, bytes, &type_segment_node);
        let type_loc = Loc::Name {
            name: type_name,
            scope: scopes[0],
        };
        relations.push((loc, type_loc));
    } else {
        // We've seen arguments. This final type segment is the return type.
        let res_name = node_name(names, bytes, &type_segment_node);
        relations.push((
            parent_loc,
            Loc::Name {
                name: res_name,
                scope: scopes[0],
            },
        ));
    }
}

fn to_field(cursor: &mut TreeCursor, field: &str) -> bool {
    loop {
        if cursor.field_name() == Some(field) {
            return true;
        }
        if !cursor.goto_next_sibling() {
            return false;
        }
    }
}

fn to_next_field(cursor: &mut TreeCursor, field: &str) -> bool {
    loop {
        if !cursor.goto_next_sibling() {
            return false;
        }
        if cursor.field_name() == Some(field) {
            return true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lib::included_answer_test::assert_eq_answer_in;
    use crate::lib::source_code::parse_bytes;
    use differential_dataflow::operators::arrange::ArrangeBySelf;
    use differential_dataflow::trace::cursor::CursorDebug;
    use differential_dataflow::trace::TraceReader;
    use std::path::{Path, PathBuf};
    use timely::dataflow::operators::Probe;

    macro_rules! type_test {
        ($name:ident) => {
            #[test]
            fn $name() {
                let mut path = std::path::PathBuf::new();
                path.push("./tests/type-checking");
                let module_name = stringify!($name);
                path.push(module_name.to_owned() + ".elm");
                println!("Run type-checking test {:?}", &path);
                typing_test(&path);
            }
        };
    }

    type_test!(nested_function_calls);
    type_test!(if_statement);
    type_test!(same_name_in_different_scopes);
    type_test!(let_in_statement);
    type_test!(case_statement);

    fn typing_test(path: &Path) {
        let path = PathBuf::from(path);
        let bytes = std::fs::read(&path).unwrap();
        let tree = parse_bytes(bytes.clone()).unwrap();
        let mut names = Names::new();
        let mut loc_refs = LocRefs::new();
        let mut relations = Vec::new();
        scan_tree(tree, &bytes, &mut names, &mut loc_refs, &mut relations);

        let mut starter_types_input =
            differential_dataflow::input::InputSession::new();
        let bool_loc = Loc::Name {
            name: names.from_str("Bool"),
            scope: Scope::Module,
        };
        let int_loc = Loc::Name {
            name: names.from_str("Int"),
            scope: Scope::Module,
        };
        let string_loc = Loc::Name {
            name: names.from_str("String"),
            scope: Scope::Module,
        };
        let length_loc = Loc::Name {
            name: names.from_str("String.length"),
            scope: Scope::Module,
        };
        starter_types_input.insert((bool_loc, "Bool".to_string()));
        starter_types_input.insert((int_loc, "Int".to_string()));
        starter_types_input.insert((string_loc, "String".to_string()));
        starter_types_input.insert((length_loc, "String -> Int".to_string()));

        let mut same_as_input =
            differential_dataflow::input::InputSession::new();
        for (from, to) in relations.into_iter() {
            same_as_input.insert((from, to));
        }

        let alloc = timely::communication::allocator::thread::Thread::new();
        let mut worker =
            timely::worker::Worker::new(timely::WorkerConfig::default(), alloc);

        let loc_refs_rc = Rc::new(Mutex::new(loc_refs));
        let names_rc = Rc::new(names);
        let (mut graph_trace, mut probe) = worker.dataflow(|scope| {
            let starter_types = starter_types_input.to_collection(scope);
            let same_as = equivalences(
                loc_refs_rc.clone(),
                &same_as_input.to_collection(scope),
                &starter_types,
            );
            let types =
                propagate_types(loc_refs_rc.clone(), &starter_types, &same_as);
            let types_graph = debug_dataflow_typechecking(
                loc_refs_rc,
                names_rc.clone(),
                &types,
                &same_as,
            );
            let graph_agg = types_graph.arrange_by_self();
            (graph_agg.trace, graph_agg.stream.probe())
        });

        dataflow::Advancable::advance(
            &mut (
                &mut graph_trace,
                &mut probe,
                &mut starter_types_input,
                &mut same_as_input,
            ),
            &mut worker,
        );

        let (mut cursor, storage) = graph_trace.cursor();

        let graph: String = cursor
            .to_vec(&storage)
            .into_iter()
            .find_map(|(i, counts)| {
                let total: isize =
                    counts.into_iter().map(|(_, count)| count).sum();
                if total > 0 {
                    Some(i)
                } else {
                    None
                }
            })
            .unwrap()
            .0;
        assert_eq_answer_in(&graph, &path);
    }

    // A dataflow computation that returns graphviz dot graphs of type-checking
    // progress.
    #[allow(dead_code)]
    pub fn debug_dataflow_typechecking<'a>(
        loc_refs: Rc<Mutex<LocRefs>>,
        names: Rc<Names>,
        types: &dataflow::Collection<'a, (Loc, Type)>,
        same_as_bidirectional: &dataflow::Collection<'a, (Loc, Loc)>,
    ) -> dataflow::Collection<'a, String> {
        let names2 = names.clone();
        let loc_refs2 = loc_refs.clone();

        let same_as = same_as_bidirectional
            .map(|(x, y)| if x < y { (x, y) } else { (y, x) })
            .distinct();

        let relation_lines = same_as.flat_map(move |(from, to)| {
            if from == to
                || matches!(from, Loc::Hypothetical { .. })
                || matches!(to, Loc::Hypothetical { .. })
            {
                None
            } else {
                let loc_refs_locked = loc_refs.lock().unwrap();
                Some((
                    (),
                    format!(
                        "\"{}\" -> \"{}\" [dir=none]\n",
                        loc_refs_locked.print(&names, from),
                        loc_refs_locked.print(&names, to)
                    ),
                ))
            }
        });

        let typed_lines = types.map(move |(loc, type_)| {
            (
                (),
                format!(
                    "\"{}\" -> \"{type_}\" [color = red]\n",
                    loc_refs2.lock().unwrap().print(&names2, loc),
                ),
            )
        });

        relation_lines
            .concat(&typed_lines)
            .reduce(|_, input, output| {
                let mut relation_strings: Vec<String> =
                    input.iter().map(|(line, _)| (*line).to_string()).collect();
                relation_strings.sort();
                let relations_string: String =
                    relation_strings.into_iter().collect();
                output.push((format!("digraph {{\n{relations_string}}}"), 1))
            })
            .map(|((), graph)| graph)
    }
}
