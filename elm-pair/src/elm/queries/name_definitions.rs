use crate::elm::{Name, NameKind};
use crate::lib::source_code::SourceFileSnapshot;
use tree_sitter::{Node, QueryCursor};

crate::elm::queries::query!("./name_definitions.query");

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
        }
    }
}

pub struct NameDefinitions<'a, 'tree> {
    code: &'tree SourceFileSnapshot,
    matches: tree_sitter::QueryMatches<'a, 'tree, &'a SourceFileSnapshot>,
}

impl<'a, 'tree> Iterator for NameDefinitions<'a, 'tree> {
    type Item = Name;

    fn next(&mut self) -> Option<Self::Item> {
        let match_ = self.matches.next()?;
        let name_node = match_.captures[0].node;
        let name = Name {
            name: self.code.slice(&name_node.byte_range()).into(),
            kind: NameKind::Value,
        };
        Some(name)
    }
}
