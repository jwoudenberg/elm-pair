use crate::elm::{Name, NameKind};
use crate::lib::log;
use crate::lib::log::Error;
use crate::lib::source_code::SourceFileSnapshot;
use std::ops::Range;
use tree_sitter::{Node, QueryCursor, QueryMatch};

crate::elm::queries::query!("./names_with_scopes.query", name, scope);

impl Query {
    pub fn run<'a, 'tree>(
        &'a self,
        cursor: &'a mut QueryCursor,
        code: &'tree SourceFileSnapshot,
    ) -> NamesWithScopes<'a, 'tree> {
        self.run_in(cursor, code, code.tree.root_node())
    }

    pub fn run_in<'a, 'tree>(
        &'a self,
        cursor: &'a mut QueryCursor,
        code: &'tree SourceFileSnapshot,
        node: Node<'tree>,
    ) -> NamesWithScopes<'a, 'tree> {
        NamesWithScopes {
            code,
            query: self,
            matches: cursor.matches(&self.query, node, code),
        }
    }
}

pub struct NamesWithScopes<'a, 'tree> {
    query: &'a Query,
    code: &'tree SourceFileSnapshot,
    matches: tree_sitter::QueryMatches<'a, 'tree, &'a SourceFileSnapshot>,
}

impl<'a, 'tree> Iterator for NamesWithScopes<'a, 'tree> {
    type Item = Result<(Name, Node<'a>, Range<usize>), Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let match_ = self.matches.next()?;
        Some(self.parse_match(match_))
    }
}

impl<'a, 'tree> NamesWithScopes<'a, 'tree> {
    fn parse_match(
        &self,
        match_: QueryMatch<'a, 'tree>,
    ) -> Result<(Name, Node<'a>, Range<usize>), Error> {
        let mut scope_range = None;
        let mut name = None;
        for capture in match_.captures.iter() {
            if capture.index == self.query.scope {
                scope_range = Some(capture.node.byte_range());
            }
            if capture.index == self.query.name {
                name = Some((
                    Name {
                        name: self
                            .code
                            .slice(&capture.node.byte_range())
                            .into(),
                        kind: NameKind::Value,
                    },
                    capture.node,
                ));
            }
        }
        let (name_, name_node) = name.ok_or_else(|| {
            log::mk_err!("match of name with scope did not include name node")
        })?;
        Ok((
            name_,
            name_node,
            scope_range.ok_or_else(|| {
                log::mk_err!(
                    "match of name with scope did not include scope node"
                )
            })?,
        ))
    }
}
