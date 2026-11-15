//! Builds and runs the C smoke test (`tests/c`) against the staticlib with CMake,
//! AddressSanitizer and UBSan. Skipped when CMake or a C compiler is missing,
//! unless `REXI_REQUIRE_JUDGES` is set.

use std::path::Path;
use std::process::Command;

fn have(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn run(cmd: &mut Command) -> String {
    let out = cmd
        .output()
        .unwrap_or_else(|e| panic!("cannot run {cmd:?}: {e}"));
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{cmd:?} failed:\n{text}");
    text
}

/// The path of `librexi.a` in Cargo's `compiler-artifact` messages (one JSON
/// object per line, `"filenames":["…/librexi.a", …]`). A plain text search is
/// enough here and needs no JSON crate: on Linux and macOS a path holds no `"`.
fn staticlib_path(messages: &str) -> Option<std::path::PathBuf> {
    messages
        .lines()
        .filter(|l| l.contains(r#""reason":"compiler-artifact""#))
        .flat_map(|l| l.split('"'))
        .find(|s| s.ends_with("/librexi.a"))
        .map(std::path::PathBuf::from)
}

#[test]
fn c_smoke_test() {
    if !have("cmake") || !have("cc") {
        assert!(
            std::env::var_os("REXI_REQUIRE_JUDGES").is_none(),
            "cmake and cc are required"
        );
        eprintln!("cmake or cc missing; skipping the C smoke test");
        return;
    }
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    // `cargo test` links integration tests against the rlib only, so build the
    // staticlib explicitly (the build lock is free while tests run). Cargo says
    // where it wrote librexi.a in its JSON messages; the path is not worked out
    // from this test's own path, because Cargo's directory layout changes
    // between versions (Rust 1.100 moved test binaries).
    let mut build_lib = Command::new(env!("CARGO"));
    build_lib
        .args(["build", "-q", "-p", "rexi-capi", "--lib"])
        .args(["--message-format", "json-render-diagnostics"])
        .current_dir(crate_dir);
    if !cfg!(debug_assertions) {
        build_lib.arg("--release");
    }
    // CARGO_TARGET_TMPDIR is <target dir>/tmp: build into the same target dir.
    if let Some(target_dir) = Path::new(env!("CARGO_TARGET_TMPDIR")).parent() {
        build_lib.arg("--target-dir").arg(target_dir);
    }
    let messages = run(&mut build_lib);
    let lib = staticlib_path(&messages)
        .unwrap_or_else(|| panic!("cargo reported no librexi.a:\n{messages}"));
    assert!(lib.is_file(), "staticlib not found at {}", lib.display());
    let build = Path::new(env!("CARGO_TARGET_TMPDIR")).join("capi-smoke");
    let fixtures = crate_dir.join("../rexi-tests/fixtures/interop");
    let source = crate_dir.join("tests/c");
    // A build directory configured from another source path (a moved checkout or
    // renamed crate) makes CMake refuse to run; start it afresh.
    let cache = std::fs::read_to_string(build.join("CMakeCache.txt")).unwrap_or_default();
    if !cache.is_empty() && !cache.contains(&*source.to_string_lossy()) {
        let _ = std::fs::remove_dir_all(&build);
    }

    run(Command::new("cmake")
        .arg("-S")
        .arg(&source)
        .arg("-B")
        .arg(&build)
        .arg(format!(
            "-DREXI_INCLUDE_DIR={}",
            crate_dir.join("include").display()
        ))
        .arg(format!("-DREXI_LIB={}", lib.display()))
        .arg(format!("-DREXI_FIXTURES={}", fixtures.display()))
        .arg("-DREXI_SANITIZE=ON"));
    run(Command::new("cmake").arg("--build").arg(&build));
    let out = run(Command::new("ctest")
        .arg("--test-dir")
        .arg(&build)
        .arg("--output-on-failure"));
    assert!(out.contains("100% tests passed"), "{out}");
}
