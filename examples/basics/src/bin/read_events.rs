//! Reads an EXI stream one event at a time with the pull decoder, and prints each
//! event with the type of its value.
//!
//! The decoder gives typed values (a bool, an integer, a decimal), not just text,
//! because the schema says what type each value has
//! ([EXI 7](https://www.w3.org/TR/exi/#encodingDatatypes)).
//!
//! Julia version: `jexi/examples/basics/read_events.jl`.

use rexi_basics::data;
use rexi_core::{Decoder, Event, ExiOptions, HeaderConfig, Value};

// ANCHOR: main
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let tables = rexi_schema::compile_files(&[&data("order.xsd")])?;
    let xml = std::fs::read(data("order.xml"))?;
    let exi = rexi_xml::encode_xml(
        &xml,
        &tables,
        &ExiOptions::strict(),
        HeaderConfig::default(),
    )?;

    let mut dec = Decoder::new(&exi[..], &tables, Some(ExiOptions::strict()))?;
    let mut depth = 0;
    // `next_event` gives `None` after the end of the document.
    while let Some(event) = dec.next_event()? {
        let indent = "  ".repeat(depth);
        match event {
            Event::StartDocument => println!("start document"),
            Event::EndDocument => println!("end document"),
            Event::StartElement(name) => {
                println!("{indent}{}", name.local);
                depth += 1;
            }
            Event::EndElement => depth -= 1,
            Event::Attribute(name, value) => {
                println!("{indent}@{} = {}", name.local, describe(&value));
            }
            Event::Characters(value) => println!("{indent}{}", describe(&value)),
            other => println!("{indent}{}", other.kind_name()),
        }
    }
    Ok(())
}
// ANCHOR_END: main

/// The value as text, followed by the kind of value the decoder gave.
fn describe(value: &Value<'_>) -> String {
    let kind = match value {
        Value::String(_) => "string",
        Value::Boolean(_) => "boolean",
        Value::Integer(_) => "integer",
        Value::Decimal(_) => "decimal",
        _ => "other",
    };
    let text = rexi_xml::plain_format(&value.to_owned_value());
    format!("{text} ({kind})")
}
