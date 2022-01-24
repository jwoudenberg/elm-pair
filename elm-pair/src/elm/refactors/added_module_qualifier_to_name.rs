use crate::elm::dependencies::DataflowComputation;
use crate::elm::refactors::lib::qualify_value::qualify_value;
use crate::elm::{QualifiedName, Queries, Refactor};
use crate::lib::log::Error;
use crate::lib::source_code::SourceFileSnapshot;

pub fn refactor(
    queries: &Queries,
    computation: &mut DataflowComputation,
    refactor: &mut Refactor,
    code: &SourceFileSnapshot,
    name: QualifiedName,
) -> Result<(), Error> {
    qualify_value(
        queries,
        computation,
        refactor,
        code,
        &[],
        &name.qualifier,
        &name.unqualified_name,
        false,
    )
}

#[cfg(test)]
mod tests {
    use crate::elm::refactors::lib::simulations::simulation_test;

    simulation_test!(add_module_qualifier_to_constructor);
    simulation_test!(
        add_module_qualifier_to_constructor_from_expose_all_import
    );
    simulation_test!(add_module_qualifier_to_type);
    simulation_test!(add_module_qualifier_to_type_with_same_name);
    simulation_test!(add_module_qualifier_to_value_from_exposing_all_import);
    simulation_test!(add_module_qualifier_to_variable);
    simulation_test!(
        add_module_qualifier_to_record_type_alias_in_type_declaration
    );
    simulation_test!(
        add_module_qualifier_to_record_type_alias_used_as_constructor
    );
    simulation_test!(add_module_alias_as_qualifier_to_variable);
}
