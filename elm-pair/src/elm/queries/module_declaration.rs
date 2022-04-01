use crate::elm::module_name::ModuleName;
use crate::lib::log;
use crate::lib::log::Error;
use crate::lib::source_code::SourceFileSnapshot;
use tree_sitter::QueryCursor;

crate::elm::queries::query!("./module_declaration.query", name);

impl Query {
    pub fn run(
        &self,
        cursor: &mut QueryCursor,
        code: &SourceFileSnapshot,
    ) -> Result<ModuleName, Error> {
        let opt_match = cursor
            .matches(&self.query, code.tree.root_node(), code)
            .next();
        if let Some(match_) = opt_match {
            let opt_capture = match_.captures.iter().next();
            if let Some(capture) = opt_capture {
                let name = code.slice(&capture.node.byte_range());
                Ok(ModuleName(name.to_string()))
            } else {
                Err(log::mk_err!("Did not find name in module declaration"))
            }
        } else {
            Err(log::mk_err!(
                "Did not find module declaration in source file"
            ))
        }
    }
}
