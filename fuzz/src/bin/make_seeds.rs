//! Writes a seed corpus for both fuzz targets from the EXIficient fixtures: each
//! strict stream with the selector byte of its schema and options in front, and
//! each schema as text.
//!
//!   make_seeds <fixtures dir> <corpus dir>

use std::{env, fs, path::Path, process::ExitCode};

/// Fixture case names in the order of `rexi_fuzz::grammars()`.
const CASES: [&str; 6] = [
    "order-complex",
    "telemetry-complex",
    "polymorphic-choice",
    "inheritance-extension",
    "cross-schema-main",
    "multilingual",
];

/// Fixture profiles and the options selector (`rexi_fuzz::options`) they need.
const PROFILES: [(&str, u8); 3] = [("strict-bp-opts", 0), ("strict-bp", 1), ("strict-ba", 2)];

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let [_, fixtures, corpus] = args.as_slice() else {
        eprintln!("usage: make_seeds <fixtures dir> <corpus dir>");
        return ExitCode::from(2);
    };
    match run(Path::new(fixtures), Path::new(corpus)) {
        Ok(count) => {
            println!("{count} seeds in {corpus}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("make_seeds: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(fixtures: &Path, corpus: &Path) -> Result<usize, String> {
    let decode = corpus.join("decode");
    let schema = corpus.join("compile_schema");
    for dir in [&decode, &schema] {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let mut count = 0;
    for (index, case) in (0u8..).zip(CASES) {
        for (profile, selector) in PROFILES {
            let name = format!("{case}.{profile}.exi");
            let stream = read(&fixtures.join(&name))?;
            let mut seed = vec![(selector << 4) | index];
            seed.extend_from_slice(&stream);
            write(&decode.join(&name), &seed)?;
            count += 1;
        }
    }
    for entry in fs::read_dir(fixtures).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().is_some_and(|ext| ext == "xsd") {
            let name = path.file_name().unwrap_or_default();
            write(&schema.join(name), &read(&path)?)?;
            count += 1;
        }
    }
    Ok(count)
}

fn read(path: &Path) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|e| format!("{}: {e}", path.display()))
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}
