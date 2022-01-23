use crate::elm::{Name, NameKind};
use crate::lib::log;
use crate::lib::log::Error;
use crate::lib::source_code::SourceFileSnapshot;
use ropey::Rope;
use tree_sitter::{Node, QueryCursor, QueryMatch};

crate::elm::queries::query!(
    "./qualified_values.query",
    root,
    qualifier,
    value,
    type_,
    constructor,
);

impl Query {
    pub fn run<'a, 'tree>(
        &'a self,
        cursor: &'a mut QueryCursor,
        code: &'tree SourceFileSnapshot,
    ) -> QualifiedNames<'a, 'tree> {
        self.run_in(cursor, code, code.tree.root_node())
    }

    pub fn run_in<'a, 'tree>(
        &'a self,
        cursor: &'a mut QueryCursor,
        code: &'tree SourceFileSnapshot,
        node: Node<'tree>,
    ) -> QualifiedNames<'a, 'tree> {
        QualifiedNames {
            code,
            query: self,
            matches: cursor.matches(&self.query, node, code),
        }
    }
}

pub struct QualifiedNames<'a, 'tree> {
    query: &'a Query,
    code: &'tree SourceFileSnapshot,
    matches: tree_sitter::QueryMatches<'a, 'tree, &'a SourceFileSnapshot>,
}

impl<'a, 'tree> Iterator for QualifiedNames<'a, 'tree> {
    type Item = Result<(Node<'a>, QualifiedName), Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let match_ = self.matches.next()?;
        Some(self.parse_match(match_))
    }
}

impl<'a, 'tree> QualifiedNames<'a, 'tree> {
    fn parse_match(
        &self,
        match_: QueryMatch<'a, 'tree>,
    ) -> Result<(Node<'a>, QualifiedName), Error> {
        let mut qualifier_range = None;
        let mut root_node = None;
        let mut opt_name_capture = None;
        match_.captures.iter().for_each(|capture| {
            if capture.index == self.query.root {
                root_node = Some(capture.node);
            }
            if capture.index == self.query.qualifier {
                match &qualifier_range {
                    None => qualifier_range = Some(capture.node.byte_range()),
                    Some(existing_range) => {
                        qualifier_range =
                            Some(existing_range.start..capture.node.end_byte())
                    }
                }
            } else {
                opt_name_capture = Some(capture)
            }
        });
        let name_capture = opt_name_capture.ok_or_else(|| {
            log::mk_err!("match of qualified reference did not include name")
        })?;
        let qualifier_range = qualifier_range.ok_or_else(|| {
            log::mk_err!(
                "match of qualified reference did not include qualifier"
            )
        })?;
        let qualifier = self.code.slice(&qualifier_range);
        let kind = match name_capture.index {
            index if index == self.query.value => NameKind::Value,
            index if index == self.query.type_ => NameKind::Type,
            index if index == self.query.constructor => NameKind::Constructor,
            index => {
                return Err(log::mk_err!(
                    "name in match of qualified reference has unexpected index {:?}",
                    index,
                ))
            }
        };
        let unqualified_name = Name {
            name: self.code.slice(&name_capture.node.byte_range()).into(),
            kind,
        };
        let qualified = QualifiedName {
            qualifier: qualifier.into(),
            unqualified_name,
        };
        Ok((
            root_node.ok_or_else(|| {
                log::mk_err!(
                    "match of qualified reference did not include root node"
                )
            })?,
            qualified,
        ))
    }
}

#[derive(PartialEq)]
pub struct QualifiedName {
    pub qualifier: Rope,
    pub unqualified_name: Name,
}
