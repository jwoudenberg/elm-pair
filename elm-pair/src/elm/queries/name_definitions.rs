use crate::elm::{Name, NameKind};
use crate::lib::log;
use crate::lib::source_code::SourceFileSnapshot;
use tree_sitter::{Node, QueryCursor};

crate::elm::queries::query!(
    "./name_definitions.query",
    value,
    type_,
    constructor
);

impl Query {
    pub fn run<'a, 'tree>(
        &'a self,
        cursor: &'a mut QueryCursor,
        code: &'tree SourceFileSnapshot,
    ) -> NameDefinitions<'a, 'tree> {
        self.run_in(cursor, code, code.tree.root_node())
    }

    pub fn run_in<'a, 'tree>(
        &'a self,
        cursor: &'a mut QueryCursor,
        code: &'tree SourceFileSnapshot,
        node: Node<'tree>,
    ) -> NameDefinitions<'a, 'tree> {
        NameDefinitions {
            code,
            matches: cursor.matches(&self.query, node, code),
            query: self,
        }
    }
}

pub struct NameDefinitions<'a, 'tree> {
    code: &'tree SourceFileSnapshot,
    matches: tree_sitter::QueryMatches<'a, 'tree, &'a SourceFileSnapshot>,
    query: &'a Query,
}

impl<'a, 'tree> Iterator for NameDefinitions<'a, 'tree> {
    type Item = (Name, Node<'a>);

    fn next(&mut self) -> Option<Self::Item> {
        let match_ = self.matches.next()?;
        let capture = match_.captures[0];
        let kind = if capture.index == self.query.value {
            NameKind::Value
        } else if capture.index == self.query.type_ {
            NameKind::Type
        } else if capture.index == self.query.constructor {
            NameKind::Constructor
        } else {
            log::error!("unexpected name definition index {}", capture.index);
            return None;
        };
        let name = Name {
            name: self.code.slice(&capture.node.byte_range()).into(),
            kind,
        };
        Some((name, capture.node))
    }
}
