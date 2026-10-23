//! Every exported function must be declared in `include/rexi.h`. cbindgen skips
//! items it cannot parse without failing (0.27 silently ignored edition 2024's
//! `#[unsafe(no_mangle)]`), so check the generated header against the sources.

use std::path::Path;

#[test]
fn header_declares_every_exported_function() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let header = std::fs::read_to_string(dir.join("include/rexi.h")).unwrap_or_default();
    let mut exported = Vec::new();
    for entry in std::fs::read_dir(dir.join("src"))
        .into_iter()
        .flatten()
        .flatten()
    {
        let src = std::fs::read_to_string(entry.path()).unwrap_or_default();
        let mut lines = src.lines();
        while let Some(line) = lines.next() {
            if line.trim() != "#[unsafe(no_mangle)]" {
                continue;
            }
            let Some(sig) = lines.next() else { continue };
            let Some(rest) = sig.split("fn ").nth(1) else {
                continue;
            };
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            exported.push(name);
        }
    }
    assert!(exported.len() >= 25, "found only {exported:?}");
    let missing: Vec<&String> = exported
        .iter()
        .filter(|name| {
            !header.contains(&format!(" {name}(")) && !header.contains(&format!("*{name}("))
        })
        .collect();
    assert!(
        missing.is_empty(),
        "missing from include/rexi.h: {missing:?}"
    );
}
