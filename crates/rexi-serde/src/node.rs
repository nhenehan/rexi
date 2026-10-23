//! Decoding an EXI stream into an element tree for the deserializer. The whole
//! document is read before deserializing starts, so the deserializer can look up
//! attributes and children by name in any order.

use crate::prelude::*;
use rexi_core::io::ByteSource;

use rexi_core::{Decoder, GrammarTables, OwnedValue};

use crate::error::{Error, Result};

/// A decoded value: the typed value and its standard text form.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Text {
    pub value: OwnedValue,
    pub lexical: String,
}

/// One decoded element: its name, its `xsi:type` and `xsi:nil`, its attributes as
/// `(uri, local, value)`, its text, and its child elements in document order.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Node {
    pub uri: String,
    pub local: String,
    pub xsi_type: Option<(String, String)>,
    pub nil: bool,
    pub attrs: Vec<(String, String, Text)>,
    pub text: Option<Text>,
    pub children: Vec<Node>,
}

/// Drops the tree with a loop instead of recursion, so a very deep document
/// cannot overflow the stack even when the decoder limits are off.
impl Drop for Node {
    fn drop(&mut self) {
        let mut pending = core::mem::take(&mut self.children);
        while let Some(mut node) = pending.pop() {
            // Each child is dropped here with no children left, so its own `drop`
            // does no more work.
            pending.append(&mut node.children);
        }
    }
}

impl Node {
    /// An element with this name and nothing in it yet.
    fn new(uri: String, local: String) -> Self {
        Node {
            uri,
            local,
            xsi_type: None,
            nil: false,
            attrs: Vec::new(),
            text: None,
            children: Vec::new(),
        }
    }

    /// True if the element has no attributes, no children and no `xsi:type`, so it
    /// can be read as a single value.
    pub(crate) fn is_simple(&self) -> bool {
        self.attrs.is_empty() && self.children.is_empty() && self.xsi_type.is_none()
    }
}

/// Reads every event and builds the element tree, using a stack of open elements.
/// Returns the root element.
pub(crate) fn read_document<R: ByteSource, G: GrammarTables>(
    mut dec: Decoder<R, G>,
) -> Result<Node> {
    let mut stack: Vec<Node> = Vec::new();
    let mut root = None;
    loop {
        // Copy what the event carries, then let go of the borrow on the decoder, so
        // the value can be formatted with the decoder's datatype.
        let ev = {
            let Some(ev) = dec.next_event()? else { break };
            rexi_core::OwnedEvent::from_event(&ev)
                .ok_or_else(|| Error::msg(format!("unexpected {} event", ev.kind_name())))?
        };
        use rexi_core::OwnedEvent as E;
        match ev {
            E::StartDocument | E::EndDocument => {}
            E::StartElement { uri, local } => stack.push(Node::new(uri, local)),
            E::EndElement => {
                let node = stack.pop().ok_or_else(|| Error::msg("unbalanced EE"))?;
                match stack.last_mut() {
                    Some(parent) => parent.children.push(node),
                    None => root = Some(node),
                }
            }
            E::Attribute { uri, local, value } => {
                let text = text(&dec, value)?;
                top(&mut stack)?.attrs.push((uri, local, text));
            }
            E::XsiType { uri, local } => top(&mut stack)?.xsi_type = Some((uri, local)),
            E::XsiNil(nil) => top(&mut stack)?.nil = nil,
            E::Characters(value) => {
                let t = text(&dec, value)?;
                let node = top(&mut stack)?;
                match &mut node.text {
                    // Mixed content (text broken up by child elements): join the
                    // runs of text. The joined value is kept as a plain string.
                    // Both strings grow in place, so many runs cost linear time.
                    Some(prev) => {
                        if !matches!(prev.value, OwnedValue::String(_)) {
                            prev.value = OwnedValue::String(prev.lexical.clone());
                        }
                        if let OwnedValue::String(v) = &mut prev.value {
                            v.push_str(&t.lexical);
                        }
                        prev.lexical.push_str(&t.lexical);
                    }
                    None => node.text = Some(t),
                }
            }
            // An event kind added to rexi-core later.
            other => return Err(Error::msg(format!("unexpected event {other:?}"))),
        }
    }
    root.ok_or_else(|| Error::msg("document has no root element"))
}

fn top(stack: &mut [Node]) -> Result<&mut Node> {
    stack
        .last_mut()
        .ok_or_else(|| Error::msg("event outside the root element"))
}

/// Pairs a decoded value with its standard text form, from the datatype of the
/// event just read.
fn text<R: ByteSource, G: GrammarTables>(dec: &Decoder<R, G>, value: OwnedValue) -> Result<Text> {
    let lexical = dec.format_last(&value.as_value())?;
    Ok(Text { value, lexical })
}
