//! Generates the C header `include/rexi.h` from the `extern "C"` surface with
//! cbindgen as a build dependency (no cbindgen CLI required).

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=cbindgen.toml");

    let Some(crate_dir) = std::env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from) else {
        println!("cargo:warning=CARGO_MANIFEST_DIR not set; rexi.h not generated");
        return;
    };
    let config = cbindgen::Config::from_root_or_default(&crate_dir);
    match cbindgen::generate_with_config(&crate_dir, config) {
        Ok(bindings) => {
            let out = crate_dir.join("include").join("rexi.h");
            if let Some(parent) = out.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            bindings.write_to_file(&out);
        }
        Err(e) => println!("cargo:warning=cbindgen header generation failed: {e}"),
    }
}
