use crate::elm::module_name::ModuleName;
use crate::elm::{ExportedName, Name, NameKind};
use crate::elm::{DOUBLE_DOT, EXPOSED_OPERATOR, EXPOSED_TYPE, EXPOSED_VALUE};
use crate::lib::log;
use crate::lib::log::Error;
use crate::lib::source_code::SourceFileSnapshot;
use ropey::{Rope, RopeSlice};
use tree_sitter::{Node, QueryCursor, TreeCursor};

crate::elm::queries::query!(
    "./imports.query",
    root,
    name,
    as_clause,
    exposing_list
);

impl Query {
    pub fn run<'a, 'tree>(
        &'a self,
        cursor: &'a mut QueryCursor,
        code: &'tree SourceFileSnapshot,
    ) -> Imports<'a, 'tree> {
        self.run_in(cursor, code, code.tree.root_node())
    }

    pub fn run_in<'a, 'tree>(
        &'a self,
        cursor: &'a mut QueryCursor,
        code: &'tree SourceFileSnapshot,
        node: Node<'tree>,
    ) -> Imports<'a, 'tree> {
        let matches = cursor.matches(&self.query, node, code);
        Imports {
            code,
            matches,
            query: self,
        }
    }

    pub fn by_aliased_name<'a>(
        &'a self,
        code: &'a SourceFileSnapshot,
        qualifier: &RopeSlice,
    ) -> Result<Import<'a>, Error> {
        let mut cursor = QueryCursor::new();
        self.run(&mut cursor, code)
            .find(|import| import.aliased_name() == *qualifier)
            .ok_or_else(|| {
                log::mk_err!(
                    "could not find an import with the requested aliased name"
                )
            })
    }
}

pub struct Imports<'a, 'tree> {
    code: &'tree SourceFileSnapshot,
    matches: tree_sitter::QueryMatches<'a, 'tree, &'a SourceFileSnapshot>,
    query: &'a Query,
}

impl<'a, 'tree> Iterator for Imports<'a, 'tree> {
    type Item = Import<'tree>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut nodes: [Option<Node>; 4] = [None; 4];
        self.matches.next()?.captures.iter().for_each(|capture| {
            nodes[capture.index as usize] = Some(capture.node)
        });
        Some(Import {
            code: self.code,
            root_node: nodes[self.query.root as usize]?,
            name_node: nodes[self.query.name as usize]?,
            as_clause_node: nodes[self.query.as_clause as usize],
            exposing_list_node: nodes[self.query.exposing_list as usize],
        })
    }
}

pub struct Import<'a> {
    code: &'a SourceFileSnapshot,
    pub root_node: Node<'a>,
    pub name_node: Node<'a>,
    pub as_clause_node: Option<Node<'a>>,
    pub exposing_list_node: Option<Node<'a>>,
}

impl Import<'_> {
    pub fn unaliased_name(&self) -> RopeSlice {
        self.code.slice(&self.name_node.byte_range())
    }

    pub fn module_name(&self) -> ModuleName {
        ModuleName(self.unaliased_name().to_string())
    }

    pub fn aliased_name(&self) -> RopeSlice {
        let name_node = self.as_clause_node.unwrap_or(self.name_node);
        self.code.slice(&name_node.byte_range())
    }

    pub fn exposing_list(&self) -> ExposedList<'_> {
        let cursor = self.exposing_list_node.and_then(|node| {
            let mut cursor = node.walk();
            if cursor.goto_first_child() {
                Some(cursor)
            } else {
                None
            }
        });
        ExposedList {
            code: self.code,
            cursor,
        }
    }
}

pub struct ExposedList<'a> {
    code: &'a SourceFileSnapshot,
    cursor: Option<TreeCursor<'a>>,
}

impl<'a> Iterator for ExposedList<'a> {
    type Item = Result<(Node<'a>, ExposedName<'a>), Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let cursor = self.cursor.as_mut()?;
        while cursor.goto_next_sibling() {
            let node = cursor.node();
            // When the programmer emptied out an exposing list entirely, so
            // only `exposing ()` remains, then the tree-sitter-elm parse result
            // will contain a single, empty `exposed_val` node, containing
            // another node marked 'missing'. This is not-unreasonable, given
            // an empty exposed list isn't valid Elm.
            //
            // For our purposes here we'd like to treat that exposed-list as
            // empty, so we can easily check for emptiness and then remove it.
            // Because the 'missing' node is wrapped inside a regular node, we
            // cannot use `is_missing()` on the outer nodes we see here, so we
            // check for length instead.
            //
            // We might consider tweaking the grammer to either put the
            // 'missing' state on the outside node, or maybe even remove the
            // wrapping entirely. Then this check likely wouldn't need this huge
            // comment explaining it.
            if node.is_named() && !node.byte_range().is_empty() {
                let exposed = match node.kind_id() {
                    EXPOSED_VALUE => ExposedName::Value(ExposedValue {
                        name: self.code.slice(&node.byte_range()),
                    }),
                    EXPOSED_OPERATOR => ExposedName::Operator(ExposedOperator {
                        name: self.code.slice(&node.byte_range()),
                    }),
                    EXPOSED_TYPE => {
                        let type_name_node = match node.child(0) {
                            Some(node) => node,
                            None => {
                                return Some(Err(log::mk_err!(
                                    "did not find name node for type in exposing list"
                                )));
                            }
                        };
                        ExposedName::Type(ExposedType {
                            name: self.code.slice(&type_name_node.byte_range()),
                            exposing_constructors: node.child(1).is_some(),
                        })
                    }
                    DOUBLE_DOT => ExposedName::All,
                    _ => {
                        return Some(Err(log::mk_err!(
                            "capture in query for exposing list has unexpected kind {:?}",
                            node.kind()
                        )))
                    }
                };
                return Some(Ok((node, exposed)));
            }
        }
        None
    }
}

#[derive(Debug, PartialEq)]
pub enum ExposedName<'a> {
    Operator(ExposedOperator<'a>),
    Value(ExposedValue<'a>),
    Type(ExposedType<'a>),
    All,
}

#[derive(Debug, PartialEq)]
pub struct ExposedOperator<'a> {
    pub name: RopeSlice<'a>,
}

#[derive(Debug, PartialEq)]
pub struct ExposedValue<'a> {
    pub name: RopeSlice<'a>,
}

#[derive(Debug, PartialEq)]
pub struct ExposedType<'a> {
    pub name: RopeSlice<'a>,
    pub exposing_constructors: bool,
}

#[derive(Clone)]
pub enum ExposedConstructors<'a> {
    FromTypeAlias(&'a String),
    FromCustomType(&'a Vec<String>),
}

impl<'a> ExposedName<'a> {
    pub fn for_each_name<I, F>(&self, exports: I, mut f: F)
    where
        I: Iterator<Item = &'a ExportedName>,
        F: FnMut(Name),
    {
        match self {
            ExposedName::Value(val) => f(Name {
                kind: NameKind::Value,
                name: val.name.into(),
            }),
            ExposedName::Type(type_) => {
                f(Name {
                    kind: NameKind::Type,
                    name: type_.name.into(),
                });
                exports.for_each(|export| match export {
                    ExportedName::Value { .. } => {}
                    ExportedName::RecordTypeAlias { name } => {
                        if name == &type_.name {
                            f(Name {
                                kind: NameKind::Constructor,
                                name: Rope::from_str(name),
                            });
                        }
                    }
                    ExportedName::Type { name, constructors } => {
                        if type_.exposing_constructors && name == &type_.name {
                            for ctor in constructors.iter() {
                                f(Name {
                                    kind: NameKind::Constructor,
                                    name: Rope::from_str(ctor),
                                })
                            }
                        }
                    }
                });
            }
            ExposedName::All => {
                exports.for_each(|export| match export {
                    ExportedName::Value { name } => f(Name {
                        kind: NameKind::Value,
                        name: Rope::from_str(name),
                    }),
                    ExportedName::RecordTypeAlias { name } => {
                        f(Name {
                            kind: NameKind::Value,
                            name: Rope::from_str(name),
                        });
                        f(Name {
                            kind: NameKind::Type,
                            name: Rope::from_str(name),
                        });
                    }
                    ExportedName::Type { name, constructors } => {
                        f(Name {
                            kind: NameKind::Type,
                            name: Rope::from_str(name),
                        });
                        for ctor in constructors.iter() {
                            f(Name {
                                kind: NameKind::Constructor,
                                name: Rope::from_str(ctor),
                            });
                        }
                    }
                });
            }
            ExposedName::Operator(op) => f(Name {
                kind: NameKind::Operator,
                name: op.name.into(),
            }),
        }
    }
}
