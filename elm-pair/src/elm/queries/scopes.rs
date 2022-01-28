use crate::lib::source_code::SourceFileSnapshot;
use std::ops::Range;
use tree_sitter::{Node, QueryCursor};

crate::elm::queries::query!("./scopes.query", scope, function_name);

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
            query: self,
        }
    }
}

pub struct Scopes<'a, 'tree> {
    matches: tree_sitter::QueryMatches<'a, 'tree, &'a SourceFileSnapshot>,
    query: &'a Query,
}

impl<'a, 'tree> Iterator for Scopes<'a, 'tree> {
    type Item = Range<usize>;

    fn next(&mut self) -> Option<Self::Item> {
        let match_ = self.matches.next()?;
        let scope_node = match_.captures[self.query.scope as usize].node;
        let mut scope = scope_node.byte_range();
        // If we're looking at the scope created by a function that name itself
        // belongs to the scope above.
        if let Some(function_name_capture) =
            match_.captures.get(self.query.function_name as usize)
        {
            scope.start = function_name_capture.node.end_byte();
        }
        Some(scope)
    }
}
