//! The `rexi` binary's schema-free commands and its argument handling.

use std::path::PathBuf;
use std::process::Command;

fn rexi() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rexi"))
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../rexi-tests/fixtures/interop")
        .join(name)
}

#[test]
fn xml_events_dumps_the_parsed_document() {
    let out = rexi()
        .arg("xml-events")
        .arg(fixture("inheritance-extension.xml"))
        .output()
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.starts_with("SD\nSE {urn:rexi:interop:inheritance}ReadingEvent\nAT {}category telemetry\nAT {}sensorId sensor-17\n"));
    assert!(text.ends_with("EE\nED\n"));
}

#[test]
fn errors_exit_with_one() {
    let out = rexi().output().unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("usage:"));

    let out = rexi()
        .args(["encode", "--no-strict", "--xsd", "x.xsd", "a.xml", "b.exi"])
        .output()
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("not supported yet"));

    let out = rexi()
        .args(["encode", "a.xml", "b.exi"])
        .output()
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--xsd is required"));

    let out = rexi()
        .args(["xml-events", "/nonexistent/file.xml"])
        .output()
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot read"));

    let out = rexi()
        .args(["frobnicate"])
        .output()
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(out.status.code(), Some(1));
}

/// `rexi bench` writes the reference bytes / events once and reports one JSON
/// line with one timing per iteration.
#[test]
fn bench_reports_timings_and_writes_outputs() {
    let dir = std::env::temp_dir().join(format!("rexi-bench-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    let xsd = fixture("order-complex.xsd");
    let run = |args: &[&std::ffi::OsStr]| {
        let out = rexi().args(args).output().unwrap_or_else(|e| panic!("{e}"));
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let exi = dir.join("o.exi");
    for from in ["xml", "events"] {
        // `--warmup-ms` keeps warming up until that much time has passed.
        let start = std::time::Instant::now();
        let json = run(&[
            "bench".as_ref(),
            "encode".as_ref(),
            "--xsd".as_ref(),
            xsd.as_os_str(),
            "--from".as_ref(),
            from.as_ref(),
            "--warmup".as_ref(),
            "1".as_ref(),
            "--warmup-ms".as_ref(),
            "25".as_ref(),
            "--iterations".as_ref(),
            "3".as_ref(),
            "--out".as_ref(),
            exi.as_os_str(),
            fixture("order-complex.xml").as_os_str(),
        ]);
        assert!(start.elapsed() >= std::time::Duration::from_millis(25));
        assert!(json.starts_with(&format!(
            "{{\"impl\":\"rexi\",\"version\":\"{}\",\"op\":\"encode\",\"input\":\"{from}\"",
            env!("CARGO_PKG_VERSION")
        )));
        for key in [
            "\"input_bytes\":538",
            "\"output\":129",
            "\"setup_ns\":",
            "\"iterations\":3",
        ] {
            assert!(json.contains(key), "{key} missing from {json}");
        }
        let timings = json.split("\"ns\":[").nth(1).unwrap_or_default();
        assert_eq!(
            timings.trim_end().trim_end_matches("]}").split(',').count(),
            3,
            "{json}"
        );
        assert_eq!(
            std::fs::read(&exi).unwrap_or_default(),
            std::fs::read(fixture("order-complex.strict-bp.exi")).unwrap_or_default()
        );
    }
    let events = dir.join("o.events");
    let json = run(&[
        "bench".as_ref(),
        "decode".as_ref(),
        "--xsd".as_ref(),
        xsd.as_os_str(),
        "--iterations".as_ref(),
        "2".as_ref(),
        "--events-out".as_ref(),
        events.as_os_str(),
        fixture("order-complex.strict-bp.exi").as_os_str(),
    ]);
    assert!(json.contains("\"op\":\"decode\""), "{json}");
    assert_eq!(
        std::fs::read_to_string(&events).unwrap_or_default(),
        std::fs::read_to_string(fixture("order-complex.strict-bp.events")).unwrap_or_default()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A scratch directory with one schema, holding an integer `i` and a list `l`,
/// and the EXI of `xml` encoded by `rexi encode`.
struct LimitCase {
    dir: PathBuf,
    xsd: PathBuf,
    exi: PathBuf,
}

impl LimitCase {
    fn new(name: &str, xml: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("rexi-limits-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        let xsd = dir.join("l.xsd");
        std::fs::write(
            &xsd,
            r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
              <xs:element name="i" type="xs:integer"/>
              <xs:element name="l"><xs:simpleType><xs:list itemType="xs:int"/></xs:simpleType></xs:element>
            </xs:schema>"#,
        )
        .unwrap_or_else(|e| panic!("{e}"));
        let src = dir.join("in.xml");
        std::fs::write(&src, xml).unwrap_or_else(|e| panic!("{e}"));
        let exi = dir.join("in.exi");
        let out = rexi()
            .arg("encode")
            .arg("--xsd")
            .arg(&xsd)
            .arg(&src)
            .arg(&exi)
            .output()
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        LimitCase { dir, xsd, exi }
    }

    /// Runs `command` (with `extra` flags) on the EXI file; returns success and
    /// stderr.
    fn run(&self, command: &[&str], extra: &[&str]) -> (bool, String) {
        let out = rexi()
            .args(command)
            .arg("--xsd")
            .arg(&self.xsd)
            .args(extra)
            .arg(&self.exi)
            .output()
            .unwrap_or_else(|e| panic!("{e}"));
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }
}

impl Drop for LimitCase {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn max_depth_flag() {
    let case = LimitCase::new("depth", "<i>5</i>");
    let (ok, err) = case.run(&["decode"], &["--max-depth", "0"]);
    assert!(ok, "{err}");
    // `<i>` is one element deep, so 1 is enough. The order fixture nests deeper,
    // so 1 stops it.
    let (ok, err) = case.run(&["decode"], &["--max-depth", "1"]);
    assert!(ok, "{err}");
    let xsd = fixture("order-complex.xsd");
    let out = rexi()
        .args(["dump-events", "--max-depth", "1", "--xsd"])
        .arg(&xsd)
        .arg(fixture("order-complex.strict-bp.exi"))
        .output()
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("decoder limit exceeded: nesting depth is over 1"),
        "{err}"
    );
}

#[test]
fn max_list_items_flag() {
    let case = LimitCase::new("list", "<l>1 2 3</l>");
    let (ok, err) = case.run(&["dump-events"], &["--max-list-items", "2"]);
    assert!(!ok && err.contains("list items is over 2"), "{err}");
    let (ok, err) = case.run(&["dump-events"], &["--max-list-items", "3"]);
    assert!(ok, "{err}");
}

#[test]
fn max_integer_bits_flag() {
    // 5 fits in one 7-bit group.
    let case = LimitCase::new("int", "<i>5</i>");
    let (ok, err) = case.run(&["decode"], &["--max-integer-bits", "6"]);
    assert!(!ok && err.contains("integer bits is over 6"), "{err}");
    let (ok, err) = case.run(&["decode"], &["--max-integer-bits", "7"]);
    assert!(ok, "{err}");
}

#[test]
fn max_string_bytes_flag() {
    let xsd = fixture("order-complex.xsd");
    let exi = fixture("order-complex.strict-bp.exi");
    let run = |limit: &str| {
        rexi()
            .args(["bench", "decode", "--iterations", "1", "--warmup", "0"])
            .args(["--max-string-bytes", limit, "--xsd"])
            .arg(&xsd)
            .arg(&exi)
            .output()
            .unwrap_or_else(|e| panic!("{e}"))
    };
    let out = run("1");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success() && err.contains("string bytes is over 1"),
        "{err}"
    );
    let out = run("0");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn no_limits_flag() {
    // About 4300 bits: over the default of 4096.
    let big = "9".repeat(1300);
    let case = LimitCase::new("nolimits", &format!("<i>{big}</i>"));
    let (ok, err) = case.run(&["decode"], &[]);
    assert!(!ok && err.contains("integer bits is over 4096"), "{err}");
    let (ok, err) = case.run(&["decode"], &["--no-limits"]);
    assert!(ok, "{err}");
}

/// Limit flags apply in order: a later flag wins over an earlier `--no-limits`,
/// and a later `--no-limits` wins over an earlier flag.
#[test]
fn limit_flags_apply_in_order() {
    let case = LimitCase::new("order", "<i>5</i>");
    let (ok, err) = case.run(&["decode"], &["--no-limits", "--max-integer-bits", "6"]);
    assert!(!ok && err.contains("integer bits is over 6"), "{err}");
    let (ok, err) = case.run(&["decode"], &["--max-integer-bits", "6", "--no-limits"]);
    assert!(ok, "{err}");
}

#[test]
fn schema_root_flag() {
    // maindoc/order.xsd includes ../common/types.xsd: refused by default, allowed
    // once the root is the folder above both.
    let set = std::env::temp_dir().join(format!("rexi-cli-root-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&set);
    for dir in ["common", "maindoc"] {
        std::fs::create_dir_all(set.join(dir)).unwrap_or_else(|e| panic!("{e}"));
    }
    std::fs::write(
        set.join("common/types.xsd"),
        r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xs:simpleType name="T"><xs:restriction base="xs:string"/></xs:simpleType></xs:schema>"#,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let main = set.join("maindoc/order.xsd");
    std::fs::write(
        &main,
        r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xs:include schemaLocation="../common/types.xsd"/><xs:element name="r" type="T"/></xs:schema>"#,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let out = rexi()
        .args(["dump-grammar", "--xsd"])
        .arg(&main)
        .output()
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("refused"));
    let out = rexi()
        .args(["dump-grammar", "--schema-root"])
        .arg(&set)
        .arg("--xsd")
        .arg(&main)
        .output()
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
