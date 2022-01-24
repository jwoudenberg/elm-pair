use crate::elm::dependencies::DataflowComputation;
use crate::elm::refactors::lib::renaming::free_names;
use crate::elm::{Name, Queries, Refactor};
use crate::lib::log::Error;
use crate::lib::source_code::SourceFileSnapshot;
use ropey::RopeSlice;
use std::collections::HashSet;
use std::ops::Range;
use tree_sitter::QueryCursor;

pub fn remove_qualifier_from_references(
    queries: &Queries,
    computation: &mut DataflowComputation,
    refactor: &mut Refactor,
    code: &SourceFileSnapshot,
    qualifier: RopeSlice,
    names: HashSet<Name>,
    skip_byteranges: &[&Range<usize>],
) -> Result<(), Error> {
    // Find existing unqualified names, so we can check whether removing
    // a qualifier from a qualified reference will introduce a naming conflict.
    free_names(
        queries,
        computation,
        refactor,
        code,
        &names,
        skip_byteranges,
    )?;
    let mut cursor = QueryCursor::new();
    let qualified_references = queries.query_for_qualified_values.run_in(
        &mut cursor,
        code,
        code.tree.root_node(),
    );
    for reference_or_error in qualified_references {
        let (node, qualified) = reference_or_error?;
        if names.contains(&qualified.unqualified_name) {
            refactor.add_change(
                // The +1 makes it include the trailing dot between qualifier
                // and qualified value.
                node.start_byte()
                    ..(node.start_byte() + qualifier.len_bytes() + 1),
                String::new(),
            );
        }
    }
    Ok(())
}
