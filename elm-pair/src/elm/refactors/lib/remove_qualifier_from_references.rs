use crate::elm::dependencies::DataflowComputation;
use crate::elm::refactors::lib::qualify_value::qualify_value;
use crate::elm::{Name, NameKind, Queries, Refactor};
use crate::lib::log;
use crate::lib::log::Error;
use crate::lib::source_code::SourceFileSnapshot;
use ropey::{Rope, RopeSlice};
use std::collections::HashMap;
use std::collections::HashSet;
use tree_sitter::{Node, QueryCursor};

pub fn remove_qualifier_from_references(
    engine: &Queries,
    computation: &mut DataflowComputation,
    refactor: &mut Refactor,
    code: &SourceFileSnapshot,
    qualifier: &RopeSlice,
    references: HashSet<Name>,
    // If we're removing qualifiers because the programmer started by removing
    // the qualifier from a single node, this is that node.
    // Our logic renaming a pre-existing variable of the same name should not
    // rename this node.
    node_stripped_of_qualifier: Option<Node>,
) -> Result<(), Error> {
    // Find existing unqualified references, so we can check whether removing
    // a qualifier from a qualified reference will introduce a naming conflict.
    let mut cursor = QueryCursor::new();
    let names_in_use: HashSet<Name> = engine
        .query_for_unqualified_values
        .run_in(&mut cursor, code, code.tree.root_node())
        .map(|r| r.map(|(_, reference)| reference))
        .collect::<Result<HashSet<Name>, Error>>()?;

    let mut names_from_other_modules: HashMap<Name, Rope> = HashMap::new();
    let imports = engine.query_for_imports.run(&mut cursor, code);
    for import in imports {
        if &import.aliased_name() == qualifier {
            continue;
        } else {
            for res in import.exposing_list() {
                let (_, exposed) = res?;
                let mut cursor = computation.exports_cursor(
                    code.buffer,
                    import.unaliased_name().to_string(),
                );
                exposed.for_each_name(cursor.iter(), |reference| {
                    names_from_other_modules
                        .insert(reference, import.aliased_name().into());
                });
            }
        }
    }

    for reference in names_in_use.intersection(&references) {
        // if another module is exposing a variable by this name, un-expose it
        if let Some(other_qualifier) = names_from_other_modules.get(reference) {
            qualify_value(
                engine,
                computation,
                refactor,
                code,
                node_stripped_of_qualifier,
                other_qualifier,
                reference,
                true,
            )?;
            continue;
        }

        // If an unqualified variable with this name already exists, rename it
        if names_in_use.contains(reference) {
            let new_name = names_with_digit(reference)
                .find(|name| !names_in_use.contains(name))
                .ok_or_else(|| {
                    log::mk_err!(
                        "names_with_digit unexpectedly ran out of names."
                    )
                })?;
            rename(
                engine,
                refactor,
                code,
                node_stripped_of_qualifier,
                reference,
                &new_name,
            )?;
        }
    }

    let qualified_references = engine.query_for_qualified_values.run_in(
        &mut cursor,
        code,
        code.tree.root_node(),
    );
    for reference_or_error in qualified_references {
        let (node, qualified) = reference_or_error?;
        if references.contains(&qualified.unqualified_name) {
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

fn rename(
    engine: &Queries,
    refactor: &mut Refactor,
    code: &SourceFileSnapshot,
    node_stripped_of_qualifier: Option<Node>,
    from: &Name,
    to: &Name,
) -> Result<(), Error> {
    let mut cursor = QueryCursor::new();
    let unqualified_values = engine.query_for_unqualified_values.run_in(
        &mut cursor,
        code,
        code.tree.root_node(),
    );
    for res in unqualified_values {
        let (node, reference) = res?;
        if &reference == from
            && Some(node.id()) != node_stripped_of_qualifier.map(|n| n.id())
        {
            refactor.add_change(node.byte_range(), to.name.to_string())
        }
    }
    Ok(())
}

struct NamesWithDigit<'a> {
    base_reference: &'a Name,
    next_digit: usize,
}

impl<'a> Iterator for NamesWithDigit<'a> {
    type Item = Name;

    fn next(&mut self) -> Option<Self::Item> {
        let mut new_name = self.base_reference.name.clone();
        new_name.append(Rope::from_str(self.next_digit.to_string().as_str()));
        let next_ref = Name {
            name: new_name,
            kind: self.base_reference.kind,
        };
        self.next_digit += 1;
        Some(next_ref)
    }
}

fn names_with_digit(reference: &Name) -> NamesWithDigit {
    NamesWithDigit {
        base_reference: reference,
        next_digit: 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iterator_returns_values_with_increasing_trailing_digit() {
        let base_reference = Name {
            name: Rope::from_str("hi"),
            kind: NameKind::Value,
        };
        let first_tree: Vec<Rope> = names_with_digit(&base_reference)
            .map(|reference| reference.name)
            .take(3)
            .collect();
        assert_eq!(first_tree, vec!["hi2", "hi3", "hi4"]);
    }
}
