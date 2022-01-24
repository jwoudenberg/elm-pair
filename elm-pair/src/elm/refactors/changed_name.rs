use crate::elm::{Name, Queries, Refactor};
use crate::lib::log::Error;
use crate::lib::source_code::SourceFileSnapshot;

pub fn refactor(
    queries: &Queries,
    refactor: &mut Refactor,
    code: &SourceFileSnapshot,
    old_name: Name,
    new_name: Name,
) -> Result<(), Error> {
    crate::elm::refactors::lib::remove_qualifier_from_references::rename(
        queries, refactor, code, None, &old_name, &new_name,
    )
}

#[cfg(test)]
mod tests {
    use crate::elm::refactors::lib::simulations::simulation_test;

    simulation_test!(change_variable_name_in_let_binding);
}
