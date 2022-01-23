use crate::elm::dependencies::DataflowComputation;
use crate::elm::{Refactor, BLOCK_COMMENT, MODULE_DECLARATION};
use crate::lib::log::Error;
use crate::lib::source_code::SourceFileSnapshot;
use std::collections::HashSet;

const IMPLICIT_ELM_IMPORTS: [&str; 10] = [
    "Basics", "Char", "Cmd", "List", "Maybe", "Platform", "Result", "String",
    "Sub", "Tuple",
];

pub fn refactor(
    computation: &mut DataflowComputation,
    refactor: &mut Refactor,
    code: &SourceFileSnapshot,
    new_import_names: HashSet<String>,
) -> Result<(), Error> {
    let mut tree_cursor = code.tree.root_node().walk();
    tree_cursor.goto_first_child();
    while (tree_cursor.node().kind_id() == MODULE_DECLARATION
        || tree_cursor.node().kind_id() == BLOCK_COMMENT)
        && tree_cursor.goto_next_sibling()
    {}
    let insert_at_byte = tree_cursor.node().start_byte();
    for new_import_name in new_import_names {
        if !IMPLICIT_ELM_IMPORTS.contains(&new_import_name.as_str())
            && computation
                .exports_cursor(code.buffer, new_import_name.clone())
                .iter()
                .next()
                .is_some()
        {
            refactor.add_change(
                insert_at_byte..insert_at_byte,
                format!("import {}\n", new_import_name),
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::elm::refactors::simulations::simulation_test;

    simulation_test!(use_qualifier_of_unimported_module_in_new_code);
    simulation_test!(use_qualifier_of_non_existing_module_in_new_code);
    simulation_test!(use_qualifier_of_implicitly_imported_module_in_new_code);
    simulation_test!(use_qualifier_of_unimported_module_while_in_the_middle_of_writing_identifier);
}
