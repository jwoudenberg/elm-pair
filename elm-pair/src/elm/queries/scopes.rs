use crate::lib::source_code::SourceFileSnapshot;
use tree_sitter::{Node, QueryCursor};

crate::elm::queries::query!("./scopes.query");

impl Query {
    pub fn run<'a, 'tree>(
        &'a self,
        cursor: &'a mut QueryCursor,
        code: &'tree SourceFileSnapshot,
    ) -> Scopes<'a, 'tree> {
        self.run_in(cursor, code, code.tree.root_node())
    }

    pub fn run_in<'a, 'tree>(
        &'a self,
        cursor: &'a mut QueryCursor,
        code: &'tree SourceFileSnapshot,
        node: Node<'tree>,
    ) -> Scopes<'a, 'tree> {
        Scopes {
            matches: cursor.matches(&self.query, node, code),
        }
    }
}

pub struct Scopes<'a, 'tree> {
    matches: tree_sitter::QueryMatches<'a, 'tree, &'a SourceFileSnapshot>,
}

impl<'a, 'tree> Iterator for Scopes<'a, 'tree> {
    type Item = Node<'tree>;

    fn next(&mut self) -> Option<Self::Item> {
        let match_ = self.matches.next()?;
        let scope_node = match_.captures[0].node;
        Some(scope_node)
    }
}
