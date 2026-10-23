//! Maps plain Rust structs to EXI and back with serde, guided by the schema.
//!
//! Fields become attributes or child elements, depending on what the schema
//! says, and in the schema's order. `Option` is an optional item and `Vec` is a
//! repeated one. The full rules are in the `rexi-serde` crate docs.
//!
//! Julia version: `jexi/examples/basics/structs.jl`.

use rexi_basics::data;
use rexi_core::{ExiOptions, HeaderConfig};
use serde::{Deserialize, Serialize};

// ANCHOR: types
// Names follow the XML: `rename` and `rename_all` map Rust field names to them.
#[derive(Serialize, Deserialize)]
struct Order {
    #[serde(rename = "orderId")]
    order_id: String,
    priority: Option<bool>,
    #[serde(rename = "Customer")]
    customer: Customer,
    #[serde(rename = "Item")]
    items: Vec<Item>,
    #[serde(rename = "Notes")]
    notes: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Customer {
    #[serde(rename = "loyaltyId")]
    loyalty_id: Option<String>,
    name: String,
    tier: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Item {
    #[serde(rename = "sku")]
    sku: String,
    description: String,
    quantity: i32,
    price: f64,
}

// ANCHOR_END: types

// ANCHOR: main
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let tables = rexi_schema::compile_files(&[&data("order.xsd")])?;
    let options = ExiOptions::strict();

    let order = Order {
        order_id: "ORD-2026-0007".into(),
        priority: Some(true),
        customer: Customer {
            loyalty_id: Some("L-8821".into()),
            name: "Raven Bookstore".into(),
            tier: "gold".into(),
        },
        items: vec![
            Item {
                sku: "BK-001".into(),
                description: "Distributed Systems Patterns".into(),
                quantity: 2,
                price: 42.95,
            },
            Item {
                sku: "BK-014".into(),
                description: "Practical EXI".into(),
                quantity: 1,
                price: 31.50,
            },
        ],
        notes: Some("gift-wrap requested".into()),
    };

    // The root element's name comes from the struct's name, `Order`.
    let exi = rexi_serde::to_exi(&order, &tables, &options, HeaderConfig::default())?;
    let xml = std::fs::read(data("order.xml"))?;
    let from_xml = rexi_xml::encode_xml(&xml, &tables, &options, HeaderConfig::default())?;
    println!("EXI: {} bytes", exi.len());
    println!("same bytes as encoding order.xml: {}", exi == from_xml);

    let back: Order = rexi_serde::from_exi(&exi, &tables, Some(options.clone()))?;
    let again = rexi_serde::to_exi(&back, &tables, &options, HeaderConfig::default())?;
    println!("decodes and encodes to the same bytes: {}", again == exi);

    println!(
        "order {} for {} ({})",
        back.order_id, back.customer.name, back.customer.tier
    );
    let mut total = 0.0;
    for item in &back.items {
        println!(
            "  {} x {:<30} {:>7.2}",
            item.quantity, item.description, item.price
        );
        total += f64::from(item.quantity) * item.price;
    }
    println!("  total {total:.2}");
    if let Some(notes) = &back.notes {
        println!("  notes: {notes}");
    }
    Ok(())
}
// ANCHOR_END: main
