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
    // staticlib explicitly (the build lock is free while tests run).
    // target/<profile>/deps/<test> → target/<profile>/librexi.a
    let exe = std::env::current_exe().unwrap_or_else(|e| panic!("{e}"));
    let profile_dir = exe
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let mut build_lib = Command::new(env!("CARGO"));
    build_lib
        .args(["build", "-q", "-p", "rexi-capi", "--lib"])
        .current_dir(crate_dir);
    if profile_dir.file_name().is_some_and(|n| n == "release") {
        build_lib.arg("--release");
    }
    if let Some(target_dir) = profile_dir.parent() {
        build_lib.arg("--target-dir").arg(target_dir);
    }
    run(&mut build_lib);
    let lib = profile_dir.join("librexi.a");
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
