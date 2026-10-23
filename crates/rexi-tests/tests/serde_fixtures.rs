//! The five interop fixtures modeled as plain serde types.
//! Serializing them reproduces EXIficient's bytes in every strict profile, and
//! deserializing the reference bytes yields the same values, through both
//! runtime-compiled and generated static tables.

mod common;

use std::fmt::Debug;

use common::{PROFILES, Profile, compile, reference};
use rexi_core::{ExiOptions, GrammarTables};
use rexi_tests::{Case, cases, describe_diff, generated};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

// order-complex ------------------------------------------------------------

#[derive(Debug, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Customer {
    #[serde(rename = "loyaltyId")]
    loyalty_id: Option<String>,
    name: String,
    tier: String,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Item {
    #[serde(rename = "sku")]
    sku: String,
    description: String,
    quantity: i32,
    price: f64,
}

fn order() -> Order {
    Order {
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
    }
}

// telemetry-complex --------------------------------------------------------

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct TelemetryBatch {
    site: String,
    #[serde(rename = "generatedAt")]
    generated_at: String,
    #[serde(rename = "Device")]
    devices: Vec<Device>,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Device {
    id: String,
    model: Option<String>,
    #[serde(rename = "Reading")]
    readings: Vec<Reading>,
    #[serde(rename = "Alarm", default)]
    alarms: Vec<Alarm>,
    #[serde(rename = "Healthy")]
    healthy: bool,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Reading {
    #[serde(rename = "sample")]
    sample: i32,
    r#type: String,
    value: f64,
    unit: String,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Alarm {
    code: String,
    severity: String,
}

fn telemetry() -> TelemetryBatch {
    let reading = |sample, t: &str, value, unit: &str| Reading {
        sample,
        r#type: t.into(),
        value,
        unit: unit.into(),
    };
    TelemetryBatch {
        site: "north-hub".into(),
        generated_at: "2026-04-15T12:00:00Z".into(),
        devices: vec![
            Device {
                id: "dev-01".into(),
                model: Some("alpha".into()),
                readings: vec![
                    reading(1, "temperature", 21.75, "C"),
                    reading(2, "humidity", 44.10, "percent"),
                ],
                alarms: vec![],
                healthy: true,
            },
            Device {
                id: "dev-02".into(),
                model: None,
                readings: vec![reading(1, "temperature", 85.00, "C")],
                alarms: vec![Alarm {
                    code: "OVER_TEMP".into(),
                    severity: "critical".into(),
                }],
                healthy: false,
            },
        ],
    }
}

// polymorphic-choice -------------------------------------------------------

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Envelope {
    #[serde(rename = "batchId")]
    batch_id: String,
    #[serde(rename = "Entry")]
    entries: Vec<Entry>,
    #[serde(rename = "Summary")]
    summary: Option<String>,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Entry {
    source: String,
    /// The `xs:choice`: the element is named after the variant.
    #[serde(rename = "$value")]
    content: EntryContent,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
enum EntryContent {
    TempReading(TempReading),
    StatusMessage(StatusMessage),
    Alert(Alert),
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct TempReading {
    value: f64,
    unit: String,
}

/// Simple content with an attribute.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct StatusMessage {
    lang: Option<String>,
    #[serde(rename = "$text")]
    text: String,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Alert {
    #[serde(rename = "ticket")]
    ticket: Option<String>,
    code: String,
    severity: String,
    acknowledged: Option<bool>,
}

fn envelope() -> Envelope {
    Envelope {
        batch_id: "batch-2026-04-15".into(),
        entries: vec![
            Entry {
                source: "sensor-1".into(),
                content: EntryContent::TempReading(TempReading {
                    value: 19.5,
                    unit: "C".into(),
                }),
            },
            Entry {
                source: "gateway-1".into(),
                content: EntryContent::StatusMessage(StatusMessage {
                    lang: Some("en".into()),
                    text: "operational".into(),
                }),
            },
            Entry {
                source: "sensor-9".into(),
                content: EntryContent::Alert(Alert {
                    ticket: Some("INC-77".into()),
                    code: "OVERHEAT".into(),
                    severity: "high".into(),
                    acknowledged: Some(false),
                }),
            },
        ],
        summary: Some("3 entries processed".into()),
    }
}

// inheritance-extension ----------------------------------------------------

/// Field order deliberately puts the attributes last: they are written in
/// schema order regardless.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ReadingEvent {
    event_id: String,
    timestamp: String,
    source: String,
    metric: String,
    value: f64,
    unit: String,
    quality: Option<String>,
    #[serde(rename = "sensorId")]
    sensor_id: String,
    #[serde(rename = "category")]
    category: String,
}

fn reading_event() -> ReadingEvent {
    ReadingEvent {
        event_id: "evt-2026-04-15-001".into(),
        timestamp: "2026-04-15T17:00:00Z".into(),
        source: "edge-gateway-2".into(),
        metric: "temperature".into(),
        value: 73.25,
        unit: "F".into(),
        quality: Some("good".into()),
        sensor_id: "sensor-17".into(),
        category: "telemetry".into(),
    }
}

// cross-schema-main --------------------------------------------------------

/// Children in `urn:rexi:interop:common` (Location's, AuditInfo) need no
/// namespace annotations: the grammar supplies them.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct IncidentReport {
    #[serde(rename = "status")]
    status: String,
    id: String,
    severity: Severity,
    location: Location,
    audit_info: AuditInfo,
}

/// `c:SeverityType`, an enumeration.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Severity {
    Low,
    Medium,
    High,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Location {
    site: String,
    zone: String,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct AuditInfo {
    created_by: String,
    created_at: String,
}

fn incident() -> IncidentReport {
    IncidentReport {
        status: "open".into(),
        id: "INC-2026-0109".into(),
        severity: Severity::High,
        location: Location {
            site: "plant-7".into(),
            zone: "north".into(),
        },
        audit_info: AuditInfo {
            created_by: "operator-12".into(),
            created_at: "2026-04-15T17:45:00Z".into(),
        },
    }
}

// --------------------------------------------------------------------------

fn case(name: &str) -> Case {
    cases()
        .into_iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("no case {name}"))
}

fn oob(case: &Case, profile: &Profile) -> Option<ExiOptions> {
    (!profile.header.include_options).then(|| (profile.options)(case))
}

/// Serializes `value` and deserializes the reference bytes for every profile.
fn check<T, G>(name: &str, value: &T, tables: G, failures: &mut Vec<String>)
where
    T: Serialize + DeserializeOwned + PartialEq + Debug,
    G: GrammarTables + Copy,
{
    let case = case(name);
    for profile in PROFILES {
        let expected = reference(&case, profile);
        match rexi_serde::to_exi(value, tables, &(profile.options)(&case), profile.header) {
            Ok(actual) if actual == expected => {}
            Ok(actual) => failures.push(describe_diff(
                &format!("{name} {} to_exi", profile.name),
                &actual,
                &expected,
            )),
            Err(e) => failures.push(format!("{name} {} to_exi: {e}", profile.name)),
        }
        match rexi_serde::from_exi::<T, _>(&expected, tables, oob(&case, profile)) {
            Ok(decoded) if &decoded == value => {}
            Ok(decoded) => failures.push(format!(
                "{name} {} from_exi: got {decoded:?}\nwant {value:?}",
                profile.name
            )),
            Err(e) => failures.push(format!("{name} {} from_exi: {e}", profile.name)),
        }
    }
}

fn check_all(tables_for: impl Fn(&str) -> Box<dyn Fn(&mut Vec<String>)>) {
    let mut failures = Vec::new();
    for name in [
        "order-complex",
        "telemetry-complex",
        "polymorphic-choice",
        "inheritance-extension",
        "cross-schema-main",
    ] {
        tables_for(name)(&mut failures);
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// Runs `check` for the case's value with tables from `get`.
macro_rules! checker {
    ($get:expr) => {
        |name: &str| -> Box<dyn Fn(&mut Vec<String>)> {
            let name = name.to_string();
            Box::new(move |f: &mut Vec<String>| {
                let tables = $get(&name);
                let t = &tables;
                match name.as_str() {
                    "order-complex" => check(&name, &order(), t, f),
                    "telemetry-complex" => check(&name, &telemetry(), t, f),
                    "polymorphic-choice" => check(&name, &envelope(), t, f),
                    "inheritance-extension" => check(&name, &reading_event(), t, f),
                    "cross-schema-main" => check(&name, &incident(), t, f),
                    other => f.push(format!("no model for {other}")),
                }
            })
        }
    };
}

#[test]
fn serde_matches_exificient_runtime_tables() {
    check_all(checker!(|n: &str| compile(&case(n))));
}

#[test]
fn serde_matches_exificient_generated_tables() {
    check_all(checker!(
        |n: &str| *generated::by_case(n).unwrap_or_else(|| panic!("no generated tables for {n}"))
    ));
}
