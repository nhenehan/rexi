//! Test support for rexi's conformance tests: finding fixtures, the EXIficient
//! reference manifest, the exipd judge, and reports that show where two byte
//! strings differ.
//!
//! The reference bytes come from EXIficient 1.0.7, run by
//! `scripts/gen-fixtures.sh`. rexi never makes them. exipd (the C reference
//! implementation) is only ever used as a second decoder of rexi's output, a
//! "judge" that checks rexi's bytes can be read by another codec.

#[rustfmt::skip]
pub mod generated;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// `crates/rexi-tests/fixtures/interop`.
pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("interop")
}

/// The rexi workspace root.
pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

/// One row of `cases.tsv`: a test case name, its XML document, its main schema,
/// and any extra schemas it imports. Paths are joined onto [`fixtures_dir`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Case {
    pub name: String,
    pub xml: PathBuf,
    pub main_xsd: PathBuf,
    pub import_xsds: Vec<PathBuf>,
}

impl Case {
    /// All schema files, the main one first.
    pub fn xsd_paths(&self) -> Vec<PathBuf> {
        let mut v = vec![self.main_xsd.clone()];
        v.extend(self.import_xsds.iter().cloned());
        v
    }

    /// The reference EXI file for a profile: `<name>.<profile>.exi`. A profile is
    /// one set of encoding options.
    pub fn reference(&self, profile: &str) -> PathBuf {
        fixtures_dir().join(format!("{}.{profile}.exi", self.name))
    }

    pub fn events_file(&self, profile: &str) -> PathBuf {
        fixtures_dir().join(format!("{}.{profile}.events", self.name))
    }

    /// The schemas, themselves encoded as EXI, that exipd needs (`<xsd>.exi`), the
    /// main one first.
    pub fn schema_exis(&self) -> Vec<PathBuf> {
        self.xsd_paths()
            .iter()
            .map(|p| {
                fixtures_dir().join(format!(
                    "{}.exi",
                    p.file_name().unwrap_or_default().to_string_lossy()
                ))
            })
            .collect()
    }
}

/// Reads `cases.tsv`. The first line is a header and is skipped, and so are blank
/// lines. Columns are split on tabs; the fourth column lists imports separated by
/// commas. A missing file gives an empty list.
pub fn cases() -> Vec<Case> {
    let dir = fixtures_dir();
    let text = std::fs::read_to_string(dir.join("cases.tsv")).unwrap_or_default();
    text.lines()
        .skip(1)
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let cols: Vec<&str> = line.split('\t').collect();
            Case {
                name: cols.first().copied().unwrap_or_default().to_string(),
                xml: dir.join(cols.get(1).copied().unwrap_or_default()),
                main_xsd: dir.join(cols.get(2).copied().unwrap_or_default()),
                import_xsds: cols
                    .get(3)
                    .copied()
                    .unwrap_or_default()
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(|s| dir.join(s))
                    .collect(),
            }
        })
        .collect()
}

/// One row of `manifest.tsv`: which reference file belongs to which case and
/// profile, its SHA-256 hash, its size in bytes, and the EXIficient version that
/// made it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestRow {
    pub name: String,
    pub profile: String,
    pub file: String,
    pub sha256: String,
    pub bytes: u64,
    pub exificient_version: String,
}

pub fn manifest() -> Vec<ManifestRow> {
    let text = std::fs::read_to_string(fixtures_dir().join("manifest.tsv")).unwrap_or_default();
    text.lines()
        .skip(1)
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let c: Vec<&str> = line.split('\t').collect();
            ManifestRow {
                name: c.first().copied().unwrap_or_default().to_string(),
                profile: c.get(1).copied().unwrap_or_default().to_string(),
                file: c.get(2).copied().unwrap_or_default().to_string(),
                sha256: c.get(3).copied().unwrap_or_default().to_string(),
                bytes: c.get(4).and_then(|s| s.parse().ok()).unwrap_or(0),
                exificient_version: c.get(5).copied().unwrap_or_default().to_string(),
            }
        })
        .collect()
}

/// Bit offset of the first difference, or `None` if equal. Bits are counted from
/// the most significant bit of byte 0, the order EXI writes them. If one input is
/// a prefix of the other, the answer is the bit just past the shorter one.
pub fn first_bit_difference(a: &[u8], b: &[u8]) -> Option<usize> {
    for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        if x != y {
            let diff = x ^ y;
            return Some(i * 8 + diff.leading_zeros() as usize);
        }
    }
    if a.len() != b.len() {
        Some(a.len().min(b.len()) * 8)
    } else {
        None
    }
}

/// A comparison that people can read, for assertion messages. It shows up to 12
/// bytes of each input in hex, starting 4 bytes before the first difference.
pub fn describe_diff(label: &str, actual: &[u8], expected: &[u8]) -> String {
    match first_bit_difference(actual, expected) {
        None => format!("{label}: identical ({} bytes)", actual.len()),
        Some(bit) => {
            let byte = bit / 8;
            let lo = byte.saturating_sub(4);
            let hex = |s: &[u8]| {
                s.iter()
                    .skip(lo)
                    .take(12)
                    .map(|b| format!("{b:02x}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            format!(
                "{label}: first difference at bit {bit} (byte {byte}); actual {} bytes, expected {} bytes\n  actual   [{lo}..]: {}\n  expected [{lo}..]: {}",
                actual.len(),
                expected.len(),
                hex(actual),
                hex(expected)
            )
        }
    }
}

/// The third-party cache that `scripts/fetch-third-party.sh` fills:
/// `$REXI_THIRD_PARTY`, or else `$XDG_CACHE_HOME/rexi/third_party`, or else
/// `$HOME/.cache/rexi/third_party`. Empty variables count as unset. It is never
/// inside the working tree.
pub fn third_party_dir() -> PathBuf {
    let env = |k: &str| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    if let Some(dir) = env("REXI_THIRD_PARTY") {
        return dir;
    }
    let cache = env("XDG_CACHE_HOME")
        .or_else(|| env("HOME").map(|h| h.join(".cache")))
        .unwrap_or_else(|| PathBuf::from(".cache"));
    cache.join("rexi").join("third_party")
}

/// Path of the exipd binary if `scripts/fetch-third-party.sh --build` built it.
pub fn exipd_binary() -> Option<PathBuf> {
    let p = third_party_dir().join("exip/build/examples/simpleDecoding/exipd");
    p.is_file().then_some(p)
}

/// Out-of-band option mask for exipd: strict, bit-packed, no options in the stream.
///
/// exipd's mask has five fields split by `%`: preserve options, other options
/// (`v` = strict, `a` = byte-aligned), valuePartitionCapacity, valueMaxLength, and
/// blockSize. `-` means "none set" (see `exipd -help`).
pub const EXIPD_OPS_STRICT_BITPACKED: &str = "-%v%-%-%-";
/// Out-of-band option mask for exipd: strict, byte-aligned.
pub const EXIPD_OPS_STRICT_BYTEALIGNED: &str = "-%va%-%-%-";

/// Decodes `stream` with exipd, passing the stream on stdin. (exipd finds its
/// arguments with `strstr`, so a file name that contains `-schema` would be read
/// wrong.) Returns exipd's output on success, or a description of the failure.
/// A run counts as failed if exipd exits with an error or prints `Error`.
pub fn run_exipd(
    ops_mask: Option<&str>,
    schema_exis: &[PathBuf],
    stream: &[u8],
) -> Result<String, String> {
    use std::io::Write;
    let Some(bin) = exipd_binary() else {
        return Err("exipd not built; run scripts/fetch-third-party.sh --build".into());
    };
    let mut cmd = Command::new(bin);
    if let Some(mask) = ops_mask {
        cmd.arg(format!("-ops={mask}"));
    }
    let schemas: Vec<String> = schema_exis
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    cmd.arg(format!("-schema={}", schemas.join(",")));
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("spawn exipd: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(stream)
            .map_err(|e| format!("write exipd stdin: {e}"))?;
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("wait exipd: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() && !stdout.contains("Error") {
        Ok(stdout)
    } else {
        Err(format!(
            "exipd exit {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
            out.status.code()
        ))
    }
}

/// Whether the W3C/exip judges must be present. Set `REXI_REQUIRE_JUDGES=1` in CI.
/// Locally, a missing exipd only skips the check. Any value counts as set.
pub fn judges_required() -> bool {
    std::env::var_os("REXI_REQUIRE_JUDGES").is_some()
}
