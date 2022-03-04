use crate::elm::{Name, NameKind};
use crate::lib::log;
use crate::lib::log::Error;
use crate::lib::source_code::SourceFileSnapshot;
use tree_sitter::{Node, QueryCursor};

crate::elm::queries::query!(
    "./unqualified_values.query",
    value,
    value_definition,
    type_,
    constructor,
);

impl Query {
    pub fn run<'a, 'tree>(
        &'a self,
        cursor: &'a mut QueryCursor,
        code: &'tree SourceFileSnapshot,
    ) -> UnqualifiedValues<'a, 'tree> {
        self.run_in(cursor, code, code.tree.root_node())
    }

    pub fn run_in<'a, 'tree>(
        &'a self,
        cursor: &'a mut QueryCursor,
        code: &'tree SourceFileSnapshot,
        node: Node<'tree>,
    ) -> UnqualifiedValues<'a, 'tree> {
        let matches = cursor.matches(&self.query, node, code);
        UnqualifiedValues {
            matches,
            code,
            query: self,
        }
    }

    pub fn parse_single(
        &self,
        code: &SourceFileSnapshot,
        node: Node,
    ) -> Result<Name, Error> {
        let mut cursor = QueryCursor::new();
        let (_, _, new_name) = self
            .run_in(&mut cursor, code, node)
            .next()
            .ok_or_else(|| {
            log::mk_err!("parsing unqualified value node using query failed")
        })??;
        Ok(new_name)
    }
}

pub struct UnqualifiedValues<'a, 'tree> {
    matches: tree_sitter::QueryMatches<'a, 'tree, &'a SourceFileSnapshot>,
    code: &'a SourceFileSnapshot,
    query: &'a Query,
}

pub enum IsDefinition {
    Yes,
    No,
}

impl<'a, 'tree> Iterator for UnqualifiedValues<'a, 'tree> {
    type Item = Result<(Node<'a>, IsDefinition, Name), Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let match_ = self.matches.next()?;
        let capture = match_.captures.first()?;
        let (is_definition, kind) = match capture.index {
            index if index == self.query.value => (IsDefinition::No, NameKind::Value),
            index if index == self.query.value_definition => (IsDefinition::Yes, NameKind::Value),
            // TODO: Add IsDefinition::Yes branches for type and construtor names.
            index if index == self.query.type_ => (IsDefinition::No, NameKind::Type),
            index if index == self.query.constructor => (IsDefinition::No, NameKind::Constructor),
            index => {
                return Some(Err(log::mk_err!(
                    "query for unqualified values captured name with unexpected index {:?}",
                    index
                )))
            }
        };
        let node = capture.node;
        let name = self.code.slice(&node.byte_range());
        let reference = Name {
            name: name.into(),
            kind,
        };
        Some(Ok((node, is_definition, reference)))
    }
}
