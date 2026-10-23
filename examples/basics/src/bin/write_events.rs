//! Builds the order document event by event, with no XML text at all.
//!
//! This is the lowest-level way to write EXI. Each call is one EXI event: start
//! element (SE), attribute (AT), characters (CH), end element (EE)
//! ([EXI 4](https://www.w3.org/TR/exi/#eventTypes)). The encoder checks every
//! call against the schema and fails if it is not allowed at that point.
//!
//! Julia version: `jexi/examples/basics/write_events.jl`.

use rexi_basics::data;
use rexi_core::{Encoder, ExiOptions, HeaderConfig, Int, Value};

const NS: &str = "urn:rexi:interop:order";

// ANCHOR: main
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let tables = rexi_schema::compile_files(&[&data("order.xsd")])?;
    let mut enc = Encoder::new(
        Vec::new(),
        &tables,
        ExiOptions::strict(),
        HeaderConfig::default(),
    )?;

    enc.start_document()?;
    enc.start_element(NS, "Order")?;
    // The schema puts attributes in no namespace, so their URI is "". Strict EXI
    // wants attributes sorted by name, which is also the order here.
    enc.attribute("", "orderId", &Value::String("ORD-2026-0007"))?;
    // A typed value goes in as it is, with no text to parse.
    enc.attribute("", "priority", &Value::Boolean(true))?;

    enc.start_element(NS, "Customer")?;
    enc.attribute("", "loyaltyId", &Value::String("L-8821"))?;
    text_element(&mut enc, "Name", Value::String("Raven Bookstore"))?;
    text_element(&mut enc, "Tier", Value::String("gold"))?;
    enc.end_element()?;

    let items = [
        ("BK-001", "Distributed Systems Patterns", 2, "42.95"),
        ("BK-014", "Practical EXI", 1, "31.50"),
    ];
    for (sku, description, quantity, price) in items {
        enc.start_element(NS, "Item")?;
        enc.attribute("", "sku", &Value::String(sku))?;
        text_element(&mut enc, "Description", Value::String(description))?;
        text_element(&mut enc, "Quantity", Value::Integer(Int::from(quantity)))?;
        // `Lexical` is text that the encoder parses with the schema's type, here
        // xs:decimal, just as if it came from an XML file.
        text_element(&mut enc, "Price", Value::Lexical(price))?;
        enc.end_element()?;
    }

    text_element(&mut enc, "Notes", Value::String("gift-wrap requested"))?;
    enc.end_element()?;
    enc.end_document()?;
    let exi = enc.finish()?;

    // The same document, encoded from the XML file, gives the same bytes.
    let xml = std::fs::read(data("order.xml"))?;
    let from_xml = rexi_xml::encode_xml(
        &xml,
        &tables,
        &ExiOptions::strict(),
        HeaderConfig::default(),
    )?;
    println!("EXI: {} bytes", exi.len());
    println!("same bytes as encoding order.xml: {}", exi == from_xml);
    Ok(())
}
// ANCHOR_END: main

/// Writes `<name>value</name>` in the order namespace.
fn text_element<G: rexi_core::GrammarTables>(
    enc: &mut Encoder<Vec<u8>, G>,
    name: &str,
    value: Value<'_>,
) -> rexi_core::ExiResult<()> {
    enc.start_element(NS, name)?;
    enc.characters(&value)?;
    enc.end_element()
}
