#![no_main]
//! The XML Schema compiler on any text: it may refuse the schema, but must not
//! panic. Inputs that name other files (`schemaLocation`) are skipped, because the
//! loader reads them from disk.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    if text.contains("schemaLocation") {
        return;
    }
    let _ = rexi_schema::compile_str(text, None);
});
