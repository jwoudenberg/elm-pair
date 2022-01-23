use crate::elm::io::ExportedName;
use crate::elm::queries::imports::ExposedConstructors;
use crate::lib::log;
use crate::lib::log::Error;
use ropey::RopeSlice;

pub fn constructors_of_exports<'a, I>(
    exported_names: I,
    type_name: RopeSlice<'a>,
) -> Result<ExposedConstructors<'a>, Error>
where
    I: Iterator<Item = &'a ExportedName>,
{
    for export in exported_names {
        match export {
            ExportedName::Value { .. } => {}
            ExportedName::RecordTypeAlias { name } => {
                return Ok(ExposedConstructors::FromTypeAlias(name));
            }
            ExportedName::Type { name, constructors } => {
                if type_name.eq(name) {
                    return Ok(ExposedConstructors::FromCustomType(
                        constructors,
                    ));
                }
            }
        }
    }
    Err(log::mk_err!("did not find type in module"))
}
