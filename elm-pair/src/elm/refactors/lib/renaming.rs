use crate::elm::dependencies::DataflowComputation;
use crate::elm::queries::qualified_values::QualifiedName;
use crate::elm::queries::unqualified_values::IsDefinition;
use crate::elm::refactors::lib::qualify_value::qualify_value;
use crate::elm::{Name, Queries, Refactor};
use crate::lib::log;
use crate::lib::log::Error;
use crate::lib::source_code::SourceFileSnapshot;
use ropey::Rope;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use tree_sitter::{Node, QueryCursor};

// Free some names so we can use them for something else. Depending on the name
// this might happen in one of two ways:
// 1. If the name is defined locally in the module we rename by adding a digit
//    to the end. For example: `violin` might become `violon2`, or `violon3` if
//    `violon2` is already taken.
// 2. If the name is exposed in a module import then we drop it from the
//    exposing list and qualify all uses of the name in the module with the
//    module name or alias.
pub fn free_names(
    queries: &Queries,
    computation: &mut DataflowComputation,
    refactor: &mut Refactor,
    code: &SourceFileSnapshot,
    names: &HashSet<Name>,
    scope_must_include_one_of: &[&Range<usize>],
    // Often this function gets called after the programmer already introduced
    // a naming conflict. Elm-pair needs to rename the old usages of the name
    // from before the programmer introduced the conflict to resolve that
    // conflict. The conflicting name the programmer just introduced though
    // should be left alone. This parameter indicates the range in the code
    // that shouldn't be touched while renaming.
    skip_byteranges: &[&Range<usize>],
) -> Result<(), Error> {
    let mut cursor = QueryCursor::new();
    let unqualified_names: Vec<(Node, IsDefinition, Name)> = queries
        .query_for_unqualified_values
        .run_in(&mut cursor, code, code.tree.root_node())
        .collect::<Result<Vec<(Node, IsDefinition, Name)>, Error>>()?;

    let mut cursor2 = QueryCursor::new();
    let scopes: Vec<Range<usize>> =
        queries.query_for_scopes.run(&mut cursor2, code).collect();

    let definitions_with_scopes: Vec<(Name, Range<usize>)> = unqualified_names
        .iter()
        .filter_map(|(node, is_definition, name)| {
            if !matches!(is_definition, IsDefinition::Yes) {
                None
            } else {
                let definition_scope = scopes
                    .iter()
                    .filter(|scope| scope.contains(&node.start_byte()))
                    // If the variable definition is in multiple scopes, the innermost
                    // (i.e. shortes) scope will be the one the variable can be used in.
                    .min_by_key(|scope| scope.len())?;
                Some((name.clone(), definition_scope.clone()))
            }
        })
        .collect();

    let names_in_use: HashSet<Name> = unqualified_names
        .into_iter()
        .map(|(_, _, name)| name)
        .collect();

    let names_from_other_modules = imported_names(
        queries,
        &mut cursor,
        computation,
        code,
        skip_byteranges,
    )?;

    for name in names {
        if let Some(other_qualifier) = names_from_other_modules.get(name) {
            // If an import is exposing a variable by this name, un-expose it.
            qualify_value(
                queries,
                computation,
                refactor,
                code,
                skip_byteranges,
                other_qualifier,
                name,
                true,
            )?;
        } else {
            let scopes: Vec<&Range<usize>> = definitions_with_scopes
                .iter()
                .filter(|(name_, scope)| {
                    name_ == name
                        && (scope_must_include_one_of.is_empty()
                            || scope_must_include_one_of
                                .iter()
                                .any(|include| scope.contains(&include.start)))
                })
                .map(|(_, scope)| scope)
                .collect();

            let new_name = names_with_digit(name)
                .find(|name| !names_in_use.contains(name))
                .ok_or_else(|| {
                    log::mk_err!(
                        "names_with_digit unexpectedly ran out of names."
                    )
                })?;

            rename(
                queries,
                refactor,
                code,
                name,
                &new_name,
                &scopes,
                skip_byteranges,
            )?;
        }
    }
    Ok(())
}

pub fn imported_names(
    queries: &Queries,
    cursor: &mut QueryCursor,
    computation: &mut DataflowComputation,
    code: &SourceFileSnapshot,
    skip_byteranges: &[&Range<usize>],
) -> Result<HashMap<Name, Rope>, Error> {
    let mut names_from_other_modules: HashMap<Name, Rope> = HashMap::new();
    let imports = queries.query_for_imports.run(cursor, code);
    for import in imports {
        if skip_byteranges.iter().any(|skip_range| {
            skip_range.contains(&import.root_node.start_byte())
        }) {
            continue;
        } else {
            for res in import.exposing_list() {
                let (_, exposed) = res?;
                let mut cursor = computation
                    .exports_cursor(code.buffer, import.module_name());
                exposed.for_each_name(cursor.iter(), |name| {
                    names_from_other_modules
                        .insert(name, import.aliased_name().into());
                });
            }
        }
    }
    Ok(names_from_other_modules)
}

// Give an unqualied variable another name.
// This might introduce naming conflicts!
pub fn rename(
    queries: &Queries,
    refactor: &mut Refactor,
    code: &SourceFileSnapshot,
    from: &Name,
    to: &Name,
    // If this slice is non empty, only rename within the ranges specified.
    include_byteranges: &[&Range<usize>],
    // Skip remames in the slices specified. This argument takes precedence over
    // `incluce_byteranges`.
    skip_byteranges: &[&Range<usize>],
) -> Result<(), Error> {
    let mut cursor = QueryCursor::new();
    let unqualified_values = queries.query_for_unqualified_values.run_in(
        &mut cursor,
        code,
        code.tree.root_node(),
    );
    let should_include = |node: &Node| {
        if include_byteranges.is_empty() {
            true
        } else {
            include_byteranges
                .iter()
                .any(|include_range| include_range.contains(&node.start_byte()))
        }
    };
    let should_skip = |node: &Node| {
        skip_byteranges
            .iter()
            .any(|skip_range| skip_range.contains(&node.start_byte()))
    };
    for res in unqualified_values {
        let (node, _, reference) = res?;
        if &reference == from && should_include(&node) && !should_skip(&node) {
            refactor.add_change(
                code.buffer,
                node.byte_range(),
                to.name.to_string(),
            )
        }
    }
    Ok(())
}

// Give an unqualied variable another name.
// This might introduce naming conflicts!
pub fn rename_qualified(
    queries: &Queries,
    refactor: &mut Refactor,
    code: &SourceFileSnapshot,
    from: &QualifiedName,
    to: &QualifiedName,
) -> Result<(), Error> {
    let mut cursor = QueryCursor::new();
    let nodes_to_rename =
        queries.query_for_qualified_values.run(&mut cursor, code);
    for res in nodes_to_rename {
        let (node, name) = res?;
        if &name != from {
            continue;
        }
        refactor.add_change(
            code.buffer,
            node.byte_range(),
            format!("{}.{}", to.qualifier, to.unqualified_name.name),
        )
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
    use crate::elm::NameKind;

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
