//! Static grammar tables produced by `rexi-schema-gen` (`scripts/gen-tables.sh`).
//! One module per fixture case, plus the Appendix C options schema.

pub mod cross_schema_main;
pub mod exi_options;
pub mod inheritance_extension;
pub mod multilingual;
pub mod order_complex;
pub mod polymorphic_choice;
pub mod telemetry_complex;

use rexi_core::GrammarSet;

/// The generated tables for a `cases.tsv` case name.
pub fn by_case(name: &str) -> Option<&'static GrammarSet<'static>> {
    Some(match name {
        "order-complex" => &order_complex::GRAMMARS,
        "telemetry-complex" => &telemetry_complex::GRAMMARS,
        "polymorphic-choice" => &polymorphic_choice::GRAMMARS,
        "inheritance-extension" => &inheritance_extension::GRAMMARS,
        "cross-schema-main" => &cross_schema_main::GRAMMARS,
        "multilingual" => &multilingual::GRAMMARS,
        _ => return None,
    })
}

/// Generated module source for a case, for the regeneration check.
pub fn source_of(module: &str) -> Option<&'static str> {
    Some(match module {
        "order_complex" => include_str!("order_complex.rs"),
        "telemetry_complex" => include_str!("telemetry_complex.rs"),
        "polymorphic_choice" => include_str!("polymorphic_choice.rs"),
        "inheritance_extension" => include_str!("inheritance_extension.rs"),
        "cross_schema_main" => include_str!("cross_schema_main.rs"),
        "multilingual" => include_str!("multilingual.rs"),
        "exi_options" => include_str!("exi_options.rs"),
        _ => return None,
    })
}
