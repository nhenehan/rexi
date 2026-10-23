//! `rexi-schema-gen <main.xsd> [more.xsd ...] --out tables.rs [--name GRAMMARS]
//! [--core-path rexi_core] [--check]`
//!
//! Compiles the schemas and writes the grammar tables as Rust source. With
//! `--check`, it writes nothing. It compares the new text with the existing
//! `--out` file instead and exits with 1 if the file is out of date. Bad
//! arguments or errors exit with 2.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use rexi_schema_gen::{RenderOptions, render};

const USAGE: &str = "usage: rexi-schema-gen <main.xsd> [more.xsd ...] --out <tables.rs> \
[--name GRAMMARS] [--core-path rexi_core] [--check]";

struct Args {
    xsds: Vec<PathBuf>,
    out: PathBuf,
    name: String,
    core_path: String,
    check: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut xsds = Vec::new();
    let mut out = None;
    let mut name = "GRAMMARS".to_string();
    let mut core_path = "rexi_core".to_string();
    let mut check = false;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let mut value = |flag: &str| it.next().ok_or_else(|| format!("{flag} needs a value"));
        match a.as_str() {
            "--out" | "-o" => out = Some(PathBuf::from(value("--out")?)),
            "--name" => name = value("--name")?,
            "--core-path" => core_path = value("--core-path")?,
            "--check" => check = true,
            "-h" | "--help" => return Err(USAGE.to_string()),
            s if s.starts_with('-') => return Err(format!("unknown flag {s}\n{USAGE}")),
            _ => xsds.push(PathBuf::from(a)),
        }
    }
    if xsds.is_empty() {
        return Err(format!("no schema given\n{USAGE}"));
    }
    let out = out.ok_or_else(|| format!("--out is required\n{USAGE}"))?;
    if !name
        .chars()
        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        || name.starts_with(|c: char| c.is_ascii_digit())
    {
        return Err(format!("--name must be SCREAMING_SNAKE_CASE, got {name}"));
    }
    Ok(Args {
        xsds,
        out,
        name,
        core_path,
        check,
    })
}

/// The "Source:" line for the generated header: the schema file names (without
/// their folders), joined by commas.
fn source_line(xsds: &[PathBuf]) -> String {
    xsds.iter()
        .map(|p| {
            p.file_name().map_or_else(
                || p.display().to_string(),
                |f| f.to_string_lossy().into_owned(),
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn run() -> Result<ExitCode, String> {
    let args = parse_args()?;
    let refs: Vec<&Path> = args.xsds.iter().map(PathBuf::as_path).collect();
    let tables = rexi_schema::compile_files(&refs).map_err(|e| format!("schema: {e}"))?;
    let text = render(
        &tables,
        &RenderOptions {
            name: &args.name,
            core_path: &args.core_path,
            source: &source_line(&args.xsds),
        },
    );
    if args.check {
        let existing = std::fs::read_to_string(&args.out)
            .map_err(|e| format!("{}: {e}", args.out.display()))?;
        if existing == text {
            return Ok(ExitCode::SUCCESS);
        }
        eprintln!("{} is stale; rerun without --check", args.out.display());
        return Ok(ExitCode::from(1));
    }
    std::fs::write(&args.out, text).map_err(|e| format!("{}: {e}", args.out.display()))?;
    Ok(ExitCode::SUCCESS)
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("rexi-schema-gen: {e}");
            ExitCode::from(2)
        }
    }
}
