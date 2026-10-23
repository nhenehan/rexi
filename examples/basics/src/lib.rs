//! Small helpers shared by the example programs in `src/bin`.

use std::path::PathBuf;

/// Grammar tables for `data/order.xsd`, written ahead of time by `rexi-schema-gen`
/// (see `prebuilt_tables.rs`).
#[rustfmt::skip]
pub mod order_tables;

/// The path of a file in the `data` folder.
pub fn data(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("data")
        .join(name)
}

/// Bytes as hex, 16 to a line, each line indented by two spaces.
pub fn hex_lines(bytes: &[u8]) -> String {
    let mut out = String::new();
    for line in bytes.chunks(16) {
        let pairs: Vec<String> = line.iter().map(|b| format!("{b:02x}")).collect();
        out.push_str("  ");
        out.push_str(&pairs.join(" "));
        out.push('\n');
    }
    out
}

/// The first `n` bytes as hex on one line.
pub fn hex_start(bytes: &[u8], n: usize) -> String {
    let pairs: Vec<String> = bytes.iter().take(n).map(|b| format!("{b:02x}")).collect();
    pairs.join(" ")
}
