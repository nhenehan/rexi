//! The messages of the MQTT todo example, and how they turn into EXI and back.
//!
//! Both message types are plain serde structs. rexi-serde maps them onto the
//! grammars that rexi-schema builds from `todo.xsd` at start-up, so the schema
//! alone decides what is an attribute, what is a child element, and which
//! namespace each name is in.
//!
//! Every stream is strict, schema-informed and bit-packed, with no options in the
//! header and no `$EXI` cookie. Both sides know the options in advance ("out of
//! band", [EXI 5.4](https://www.w3.org/TR/exi/#options)), so the header is a
//! single byte, `0x80` ([EXI 5](https://www.w3.org/TR/exi/#header)).

use rexi_core::{ExiOptions, GrammarSetBuf, HeaderConfig};
use serde::{Deserialize, Serialize};

/// The topic clients publish commands to.
pub const CMD_TOPIC: &str = "todo/cmd";
/// The topic the server publishes the whole list to (retained).
pub const LIST_TOPIC: &str = "todo/list";

/// The schema every side builds its grammars from.
pub const XSD: &str = include_str!("../../todo.xsd");

/// One change to the list, sent by a client on [`CMD_TOPIC`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename = "command")]
pub struct Command {
    /// Who sent the command (an attribute in the schema).
    pub from: String,
    /// Which change: the schema's `xs:choice` of `add`, `done` and `remove`.
    #[serde(rename = "$value")]
    pub action: Action,
}

/// The branches of the command's `xs:choice`. Each variant is written as an
/// element with the variant's name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Action {
    /// Add a new item with this title.
    #[serde(rename = "add")]
    Add {
        /// The new item's title.
        title: String,
    },
    /// Mark an item as done.
    #[serde(rename = "done")]
    Done {
        /// The item to mark.
        id: u32,
    },
    /// Take an item off the list.
    #[serde(rename = "remove")]
    Remove {
        /// The item to remove.
        id: u32,
    },
}

/// The whole list, sent by the server on [`LIST_TOPIC`] after every change.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename = "todoList")]
pub struct TodoList {
    /// Goes up by one with every change.
    pub revision: u32,
    /// The items, oldest first. The schema allows none at all.
    #[serde(default, rename = "item")]
    pub items: Vec<Item>,
}

/// One entry of the list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    /// The item's number, given by the server.
    pub id: u32,
    /// Whether the item is finished.
    pub done: bool,
    /// The title is the element's text (`xs:simpleContent`).
    #[serde(rename = "$text")]
    pub title: String,
}

/// The error type of this crate: rexi's errors, as text.
pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// The grammars and options shared by every message.
#[derive(Debug)]
pub struct Codec {
    tables: GrammarSetBuf,
    options: ExiOptions,
}

impl Codec {
    /// Builds the grammars from [`XSD`].
    pub fn new() -> Result<Self, Error> {
        Ok(Self {
            tables: rexi_schema::compile_str(XSD, None)?,
            options: ExiOptions::strict(),
        })
    }

    /// Writes a message as EXI. `HeaderConfig::default()` is the bare header:
    /// no cookie, no options.
    pub fn encode<T: Serialize>(&self, message: &T) -> Result<Vec<u8>, Error> {
        Ok(rexi_serde::to_exi(
            message,
            &self.tables,
            &self.options,
            HeaderConfig::default(),
        )?)
    }

    /// Reads a message from EXI. The options are given here because the header
    /// does not carry them.
    pub fn decode<T: serde::de::DeserializeOwned>(&self, exi: &[u8]) -> Result<T, Error> {
        Ok(rexi_serde::from_exi(
            exi,
            &self.tables,
            Some(self.options.clone()),
        )?)
    }
}

/// Prints the list the same way in every client, so the demo can compare them.
pub fn format_list(list: &TodoList) -> String {
    let mut out = format!("revision {}\n", list.revision);
    for item in &list.items {
        let mark = if item.done { 'x' } else { ' ' };
        out.push_str(&format!("[{mark}] {} {}\n", item.id, item.title));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() -> Result<(), Error> {
        let codec = Codec::new()?;
        for action in [
            Action::Add {
                title: "buy milk".into(),
            },
            Action::Done { id: 3 },
            Action::Remove { id: 4 },
        ] {
            let cmd = Command {
                from: "test".into(),
                action,
            };
            let bytes = codec.encode(&cmd)?;
            assert_eq!(bytes[0], 0x80);
            assert_eq!(codec.decode::<Command>(&bytes)?, cmd);
        }
        for list in [
            TodoList::default(),
            TodoList {
                revision: 7,
                items: vec![
                    Item {
                        id: 1,
                        done: false,
                        title: "a".into(),
                    },
                    Item {
                        id: 2,
                        done: true,
                        title: "b c".into(),
                    },
                ],
            },
        ] {
            let bytes = codec.encode(&list)?;
            assert_eq!(codec.decode::<TodoList>(&bytes)?, list);
        }
        Ok(())
    }

    /// EXIficient 1.0.7 writes the same bytes (strict, bit-packed, bare header), from
    /// `tools/exi-oracle encode-xml --xsd todo.xsd --strict` on the XML in the comments.
    #[test]
    fn same_bytes_as_exificient() -> Result<(), Error> {
        let codec = Codec::new()?;
        // <command xmlns="urn:rexi:example:todo" from="java"><add title="buy milk"/></command>
        let cmd = Command {
            from: "java".into(),
            action: Action::Add {
                title: "buy milk".into(),
            },
        };
        assert_eq!(
            hex(&codec.encode(&cmd)?),
            "80019a985d9840a627579206d696c6b0"
        );
        // <todoList xmlns="urn:rexi:example:todo" revision="7"><item id="1" done="false">a</item>
        //   <item id="2" done="true">b c</item></todoList>
        let list = TodoList {
            revision: 7,
            items: vec![
                Item {
                    id: 1,
                    done: false,
                    title: "a".into(),
                },
                Item {
                    id: 2,
                    done: true,
                    title: "b c".into(),
                },
            ],
        };
        assert_eq!(hex(&codec.encode(&list)?), "8041c0103614081588818e");
        Ok(())
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
}
