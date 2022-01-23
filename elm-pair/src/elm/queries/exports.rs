use crate::elm::io::ExportedName;
use crate::lib::log;
use crate::lib::log::Error;
use std::collections::HashSet;
use tree_sitter::{Node, QueryCursor, Tree};

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
        tree: &Tree,
        code: &[u8],
    ) -> Result<Vec<ExportedName>, Error> {
        let mut cursor = QueryCursor::new();
        let matches = cursor
            .matches(&self.query, tree.root_node(), code)
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
                let val = Exposed::Value(code_slice(code, &capture.node)?);
                exposed = exposed.add(val);
            } else if self.exposed_type == capture.index {
                let name_node = capture.node.child(0).ok_or_else(|| {
                    log::mk_err!(
                        "could not find name node of type in exposing list"
                    )
                })?;
                let name = code_slice(code, &name_node)?;
                let val = if capture.node.child(1).is_some() {
                    Exposed::TypeWithConstructors(name)
                } else {
                    Exposed::Type(name)
                };
                exposed = exposed.add(val);
            } else if self.value == capture.index {
                let name = code_slice(code, &capture.node)?;
                if exposed.has(&Exposed::Value(name)) {
                    let export = ExportedName::Value {
                        name: name.to_owned(),
                    };
                    exports.push(export);
                }
            } else if self.type_alias == capture.index {
                let name = code_slice(code, &capture.node)?;
                if exposed.has(&Exposed::Type(name)) {
                    let aliased_type = capture
                        .node
                        .parent()
                        .and_then(|n| n.child_by_field_name("typeExpression"))
                        .and_then(|n| n.child_by_field_name("part"))
                        .map(|n| n.kind());
                    let export = if aliased_type == Some("record_type") {
                        ExportedName::RecordTypeAlias {
                            name: name.to_owned(),
                        }
                    } else {
                        ExportedName::Type {
                            name: name.to_owned(),
                            constructors: Vec::new(),
                        }
                    };
                    exports.push(export);
                }
            } else if self.type_ == capture.index {
                let name = code_slice(code, &capture.node)?;
                if exposed.has(&Exposed::TypeWithConstructors(name)) {
                    let constructors = rest
                        .iter()
                        .map(|ctor_capture| {
                            code_slice(code, &ctor_capture.node)
                                .map(std::borrow::ToOwned::to_owned)
                        })
                        .collect::<Result<Vec<String>, Error>>()?;
                    let export = ExportedName::Type {
                        name: name.to_owned(),
                        constructors,
                    };
                    exports.push(export);
                } else if exposed.has(&Exposed::Type(name)) {
                    let export = ExportedName::Type {
                        name: name.to_owned(),
                        constructors: Vec::new(),
                    };
                    exports.push(export);
                }
            }
        }
        Ok(exports)
    }
}

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

#[derive(Hash, PartialEq)]
enum Exposed<'a> {
    Type(&'a str),
    TypeWithConstructors(&'a str),
    Value(&'a str),
}

impl Eq for Exposed<'_> {}

fn code_slice<'a>(code: &'a [u8], node: &Node) -> Result<&'a str, Error> {
    std::str::from_utf8(&code[node.byte_range()]).map_err(|err| {
        log::mk_err!(
            "Failed to decode code slice for node {} as UTF8: {:?}",
            node.kind(),
            err
        )
    })
}
