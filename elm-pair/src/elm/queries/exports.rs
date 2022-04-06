use crate::elm::io::ExportedName;
use crate::lib::log;
use crate::lib::log::Error;
use crate::lib::source_code::SourceFileSnapshot;
use ropey::RopeSlice;
use std::collections::HashSet;
use tree_sitter::QueryCursor;

crate::elm::queries::query!(
    "./exports.query",
    exposed_all,
    exposed_value,
    exposed_type,
    value,
    type_,
    type_alias,
);

impl Query {
    pub fn run(
        &self,
        code: &SourceFileSnapshot,
    ) -> Result<Vec<ExportedName>, Error> {
        let mut cursor = QueryCursor::new();
        let matches = cursor
            .matches(&self.query, code.tree.root_node(), code)
            .filter_map(|match_| {
                if let [capture, rest @ ..] = match_.captures {
                    Some((capture, rest))
                } else {
                    None
                }
            });
        let mut exposed = ExposedList::Some(HashSet::new());
        let mut exports = Vec::new();
        for (capture, rest) in matches {
            if self.exposed_all == capture.index {
                exposed = ExposedList::All;
            } else if self.exposed_value == capture.index {
                let val =
                    Exposed::Value(code.slice(&capture.node.byte_range()));
                exposed = exposed.add(val);
            } else if self.exposed_type == capture.index {
                let name_node = capture.node.child(0).ok_or_else(|| {
                    log::mk_err!(
                        "could not find name node of type in exposing list"
                    )
                })?;
                let name = code.slice(&name_node.byte_range());
                let val = if capture.node.child(1).is_some() {
                    Exposed::TypeWithConstructors(name)
                } else {
                    Exposed::Type(name)
                };
                exposed = exposed.add(val);
            } else if self.value == capture.index {
                let name = code.slice(&capture.node.byte_range());
                if exposed.has(&Exposed::Value(name)) {
                    let export = ExportedName::Value {
                        name: name.to_string(),
                    };
                    exports.push(export);
                }
            } else if self.type_alias == capture.index {
                let name = code.slice(&capture.node.byte_range());
                if exposed.has(&Exposed::Type(name)) {
                    let aliased_type = capture
                        .node
                        .parent()
                        .and_then(|n| n.child_by_field_name("typeExpression"))
                        .and_then(|n| n.child_by_field_name("part"))
                        .map(|n| n.kind());
                    let export = if aliased_type == Some("record_type") {
                        ExportedName::RecordTypeAlias {
                            name: name.to_string(),
                        }
                    } else {
                        ExportedName::Type {
                            name: name.to_string(),
                            constructors: Vec::new(),
                        }
                    };
                    exports.push(export);
                }
            } else if self.type_ == capture.index {
                let name = code.slice(&capture.node.byte_range());
                if exposed.has(&Exposed::TypeWithConstructors(name)) {
                    let constructors = rest
                        .iter()
                        .map(|ctor_capture| {
                            code.slice(&ctor_capture.node.byte_range())
                                .to_string()
                        })
                        .collect::<Vec<String>>();
                    let export = ExportedName::Type {
                        name: name.to_string(),
                        constructors,
                    };
                    exports.push(export);
                } else if exposed.has(&Exposed::Type(name)) {
                    let export = ExportedName::Type {
                        name: name.to_string(),
                        constructors: Vec::new(),
                    };
                    exports.push(export);
                }
            }
        }
        Ok(exports)
    }
}

#[derive(Debug)]
enum ExposedList<'a> {
    All,
    Some(HashSet<Exposed<'a>>),
}

impl<'a> ExposedList<'a> {
    fn add(mut self, item: Exposed<'a>) -> Self {
        match &mut self {
            ExposedList::All => {}
            ExposedList::Some(items) => {
                items.insert(item);
            }
        }
        self
    }

    fn has(&self, item: &Exposed) -> bool {
        match self {
            ExposedList::All => true,
            ExposedList::Some(items) => items.contains(item),
        }
    }
}

#[derive(Debug, Hash, PartialEq)]
enum Exposed<'a> {
    Type(RopeSlice<'a>),
    TypeWithConstructors(RopeSlice<'a>),
    Value(RopeSlice<'a>),
}

impl Eq for Exposed<'_> {}
