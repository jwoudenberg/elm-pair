use crate::elm::queries::imports::Import;
use crate::elm::{Name, Queries, Refactor};
use crate::lib::log::Error;
use crate::lib::source_code::SourceFileSnapshot;
use std::collections::HashSet;
use std::ops::Range;
use tree_sitter::{Node, QueryCursor};

pub fn add_qualifier_to_references(
    engine: &Queries,
    refactor: &mut Refactor,
    cursor: &mut QueryCursor,
    code: &SourceFileSnapshot,
    skip_byteranges: &[&Range<usize>],
    import: &Import,
    references: HashSet<Name>,
) -> Result<(), Error> {
    let results = engine.query_for_unqualified_values.run(cursor, code);
    let should_skip = |node: Node| {
        skip_byteranges
            .iter()
            .any(|skip_range| skip_range.contains(&node.start_byte()))
    };
    for result in results {
        let (node, _, reference) = result?;
        if references.contains(&reference) && !should_skip(node) {
            refactor.add_change(
                node.start_byte()..node.start_byte(),
                format!("{}.", import.aliased_name()),
            );
        }
    }
    Ok(())
}
