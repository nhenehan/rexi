//! Grammar-structure check against EXIficient's grammar dumps for the five fixtures.
//!
//! Both dumps use the same text format but number grammars differently (EXIficient
//! shares identical type grammars between elements; our emitter may keep separate
//! copies). Both are renumbered by a depth-first walk from the document grammar and
//! the string-table references before comparing, so what is compared is the
//! reachable grammar structure: terminals, datatypes, transitions and flags.

use std::collections::HashMap;
use std::path::Path;

fn fixtures() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../rexi-tests/fixtures/interop")
}

const CASES: &[(&str, &[&str])] = &[
    ("order-complex", &["order-complex.xsd"]),
    ("telemetry-complex", &["telemetry-complex.xsd"]),
    ("polymorphic-choice", &["polymorphic-choice.xsd"]),
    ("inheritance-extension", &["inheritance-extension.xsd"]),
    (
        "cross-schema-main",
        &["cross-schema-main.xsd", "cross-schema-common.xsd"],
    ),
];

/// References to grammar numbers inside a dump line, in order of appearance.
fn refs(line: &str) -> Vec<(usize, usize, u32)> {
    let mut out = Vec::new();
    for key in [
        "grammar=",
        "-> ",
        "type-grammar=",
        "global-element=",
        "element-content=",
        "fragment-grammar ",
    ] {
        let mut start = 0;
        while let Some(pos) = line[start..].find(key) {
            let num_start = start + pos + key.len();
            let num_end = line[num_start..]
                .find(|c: char| !c.is_ascii_digit())
                .map_or(line.len(), |i| num_start + i);
            if let Ok(n) = line[num_start..num_end].parse::<u32>() {
                out.push((num_start, num_end, n));
            }
            start = num_end.max(start + pos + 1);
        }
    }
    out.sort_unstable();
    // "type-grammar=" also matches "grammar="; keep one reference per position.
    out.dedup_by_key(|r| r.0);
    out
}

/// Canonical form of a dump: grammars are first merged by partition refinement
/// (two grammars are equivalent when their text is identical after replacing every
/// grammar reference by its equivalence class), then the classes are renumbered in
/// depth-first order from the document grammar and the string-table references.
/// Unreachable grammars are dropped.
fn canonicalise(dump: &str) -> String {
    let lines: Vec<&str> = dump.lines().filter(|l| !l.starts_with('#')).collect();
    let mut blocks: HashMap<u32, Vec<&str>> = HashMap::new();
    let mut preamble: Vec<&str> = Vec::new();
    let mut current: Option<u32> = None;
    for line in &lines {
        if let Some(rest) = line.strip_prefix("grammar ") {
            let n: u32 = rest
                .split_whitespace()
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(u32::MAX);
            current = Some(n);
            blocks.entry(n).or_default().push(line);
        } else if let Some(n) = current {
            blocks.entry(n).or_default().push(line);
        } else {
            preamble.push(line);
        }
    }
    // Text of a block with every grammar reference replaced through `map`.
    let render = |block: &[&str], map: &dyn Fn(u32) -> String| -> String {
        let mut out = String::new();
        for (j, line) in block.iter().enumerate() {
            let line: String = if j == 0 {
                format!(
                    "grammar {}",
                    line.trim_start_matches("grammar ")
                        .split_once(' ')
                        .map_or("", |x| x.1)
                )
            } else {
                (*line).to_string()
            };
            let mut last = 0;
            for (a, b, n) in refs(&line) {
                out.push_str(&line[last..a]);
                out.push_str(&map(n));
                last = b;
            }
            out.push_str(&line[last..]);
            out.push('\n');
        }
        out
    };
    // Partition refinement.
    let ids: Vec<u32> = blocks.keys().copied().collect();
    let mut class: HashMap<u32, u32> = ids.iter().map(|&n| (n, 0)).collect();
    loop {
        let mut sigs: HashMap<String, u32> = HashMap::new();
        let mut next: HashMap<u32, u32> = HashMap::new();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        for &n in &sorted {
            let sig = render(&blocks[&n], &|m| {
                class.get(&m).map_or("?".to_string(), u32::to_string)
            });
            let len = sigs.len() as u32;
            let c = *sigs.entry(sig).or_insert(len);
            next.insert(n, c);
        }
        let stable = sorted.iter().all(|n| next[n] == class[n])
            && sigs.len() == {
                let mut seen: Vec<u32> = class.values().copied().collect();
                seen.sort_unstable();
                seen.dedup();
                seen.len()
            };
        class = next;
        if stable {
            break;
        }
    }
    // Representative block per class.
    let mut rep: HashMap<u32, u32> = HashMap::new();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    for &n in &sorted {
        rep.entry(class[&n]).or_insert(n);
    }
    // DFS numbering over classes.
    let mut order: Vec<u32> = Vec::new();
    let mut number: HashMap<u32, u32> = HashMap::new();
    let visit = |n: u32, order: &mut Vec<u32>, number: &mut HashMap<u32, u32>| {
        if let Some(&c) = class.get(&n)
            && let std::collections::hash_map::Entry::Vacant(slot) = number.entry(c)
        {
            slot.insert(order.len() as u32);
            order.push(c);
        }
    };
    visit(0, &mut order, &mut number);
    for line in &preamble {
        for (_, _, n) in refs(line) {
            visit(n, &mut order, &mut number);
        }
    }
    let mut i = 0;
    while i < order.len() {
        let block = &blocks[&rep[&order[i]]];
        for line in block {
            for (_, _, m) in refs(line) {
                visit(m, &mut order, &mut number);
            }
        }
        i += 1;
    }
    let map_num = |n: u32| -> String {
        class
            .get(&n)
            .and_then(|c| number.get(c))
            .map_or("?".to_string(), u32::to_string)
    };
    let mut out = String::new();
    for line in &preamble {
        let mut last = 0;
        for (a, b, n) in refs(line) {
            out.push_str(&line[last..a]);
            out.push_str(&map_num(n));
            last = b;
        }
        out.push_str(&line[last..]);
        out.push('\n');
    }
    for (k, c) in order.iter().enumerate() {
        let text = render(&blocks[&rep[c]], &map_num);
        out.push_str(&text.replacen("grammar ", &format!("grammar {k} "), 1));
    }
    out.trim().to_string()
}

#[test]
fn reachable_grammars_match_exificient() {
    let mut failures = Vec::new();
    for (name, xsds) in CASES {
        let dir = fixtures();
        let paths: Vec<std::path::PathBuf> = xsds.iter().map(|x| dir.join(x)).collect();
        let refs: Vec<&Path> = paths.iter().map(|p| p.as_path()).collect();
        let tables = rexi_schema::compile_files(&refs).unwrap_or_else(|e| panic!("{name}: {e}"));
        let actual = canonicalise(&rexi_schema::dump(&tables));
        let oracle =
            std::fs::read_to_string(dir.join(format!("{name}.grammars.txt"))).unwrap_or_default();
        let expected = canonicalise(&oracle);
        if actual != expected {
            let a: Vec<&str> = actual.lines().collect();
            let e: Vec<&str> = expected.lines().collect();
            let first = a
                .iter()
                .zip(e.iter())
                .position(|(x, y)| x != y)
                .unwrap_or(a.len().min(e.len()));
            failures.push(format!(
                "{name}: differs at line {} ({} vs {} lines)\n  ours:   {}\n  oracle: {}",
                first + 1,
                a.len(),
                e.len(),
                a.get(first).unwrap_or(&"<eof>"),
                e.get(first).unwrap_or(&"<eof>")
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
