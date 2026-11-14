#!/usr/bin/env python3
"""Run rexi against the W3C EXI test suite's declaredProductions group, with
EXIficient 1.0.7 as the reference.

    scripts/w3c-declared.py [--rexi PATH] [--update] [--report FILE]

The suite is the pinned exi-testsuite checkout and the reference is the pinned
exip's EXIficient oracle (utils/xsd2exip), both from scripts/fetch-third-party.sh
--build. Nothing from the suite is copied into this repository: the reference
streams are made here, at run time, under target/w3c-declared/.

Each case takes its schema and options from the suite's config
(config/testCases-interop/schemaInformedGrammar/declaredProductions/*.xml: group
params, overridden per case). It runs bit-packed and byte-aligned, with the
options out of band. For each case and alignment:

  reference  EXIficient encodes the XML, then dumps the events of that stream;
  encode     rexi encodes the XML: its bytes must equal the reference;
  decode     rexi dumps the events of the reference: they must equal EXIficient's;
  re-encode  rexi decodes the reference to XML and encodes that again: the bytes
             must equal the reference.

A case passes when all three steps match on both alignments. The result of each
case and alignment is one of: pass, unsupported (rexi refuses the stream with an
"unsupported EXI feature" error, or the case needs an option the rexi tool does
not have), fail (anything else), skip (no reference can be made). The results
are compared with scripts/w3c-declared.expected, and the run fails on any
difference, so a regression fails and so does a case that passes without being
listed as passing. --update rewrites that file instead.
"""

import argparse
import os
import re
import subprocess
import sys
import xml.etree.ElementTree as ET

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
CFG_REL = "config/testCases-interop/schemaInformedGrammar/declaredProductions"
P = "org.w3c.exi.ttf."
ALIGNMENTS = ("bit-packed", "byte-aligned")
EXPECTED = os.path.join(HERE, "w3c-declared.expected")


def params(elem, inherited):
    out = dict(inherited)
    for p in elem.findall("{*}param"):
        out[p.get("name")] = p.get("value")
    return out


def subst(value, env):
    return re.sub(r"\$\{([^}]+)\}", lambda m: env.get(m.group(1), m.group(0)), value)


def cases(testsuite):
    """Yields (name, params) for every test case of the group, in file order."""
    env = {"japex.exi.ttfms.testCasesDir": os.path.join(testsuite, "data")}
    cfg = os.path.join(testsuite, CFG_REL)
    for name in sorted(os.listdir(cfg)):
        if not name.endswith(".xml"):
            continue
        group = ET.parse(os.path.join(cfg, name)).getroot()
        gp = params(group, {})
        for tc in group.findall("{*}testCase"):
            tp = params(tc, gp)
            for _ in range(5):  # params refer to each other
                tp = {k: subst(v, {**env, **tp}) for k, v in tp.items()}
            yield tc.get("name").split(".")[-1], tp


def run(cmd):
    r = subprocess.run(cmd, capture_output=True)
    return r.returncode, r.stdout, r.stderr.decode("utf-8", "replace")


def last_line(text):
    lines = [
        line
        for line in text.strip().splitlines()
        if line.strip() and not line.startswith("SLF4J")
    ]
    return lines[-1][:200] if lines else ""


def rexi_error(err):
    """("unsupported" or "fail", the message) for a failed rexi command."""
    msg = last_line(err)
    kind = "unsupported" if "unsupported EXI feature" in msg or "not supported yet" in msg else "fail"
    return kind, msg


def one(a, name, tp, alignment):
    """Runs one case on one alignment. Returns (status, note)."""
    if tp.get(P + "useSchemas") != "true":
        return "unsupported", "schema-less (built-in grammars)"
    if tp.get(P + "preserve"):
        return "unsupported", "Preserve." + tp[P + "preserve"].replace(" ", ",")
    if tp.get(P + "fragments") == "true":
        # The oracle has no fragment option, so there is no reference.
        return "skip", "fragment: the EXIficient oracle has no --fragment"
    strict = "--no-strict" if tp.get(P + "schemaDeviations") == "true" else "--strict"
    xsd = tp[P + "schemaLocation"]
    xml = tp["japex.inputFile"]
    w = os.path.join(a.work, "%s.%s" % (name, alignment))
    opts = [strict, "--alignment", alignment]

    code, _, err = run(
        [
            "java",
            "-jar",
            a.oracle,
            "encode-xml",
            "--xsd",
            xsd,
            "--xml",
            xml,
            "--out",
            w + ".ref.exi",
            *opts,
        ]
    )
    if code != 0:
        return "skip", "EXIficient refuses the XML: " + last_line(err)
    code, want, err = run(
        [
            "java",
            "-jar",
            a.oracle,
            "dump-events",
            "--xsd",
            xsd,
            "--exi",
            w + ".ref.exi",
            *opts,
        ]
    )
    if code != 0:
        return "skip", "EXIficient cannot read its own stream: " + last_line(err)
    ref = open(w + ".ref.exi", "rb").read()
    # The suite's schemas import each other by relative path; allow any file.
    rexi = [a.rexi]
    schema = ["--xsd", xsd, "--schema-root", "/", *opts]

    code, _, err = run([*rexi, "encode", *schema, xml, w + ".rexi.exi"])
    if code != 0:
        return rexi_error(err)
    got = open(w + ".rexi.exi", "rb").read()
    if got != ref:
        return "fail", "encode: %d bytes, EXIficient %d (%s vs %s)" % (
            len(got),
            len(ref),
            got.hex(),
            ref.hex(),
        )

    code, events, err = run([*rexi, "dump-events", *schema, w + ".ref.exi"])
    if code != 0:
        return rexi_error(err)
    if events != want:
        got_l, want_l = events.decode().splitlines(), want.decode().splitlines()
        i = next(
            (i for i, (x, y) in enumerate(zip(got_l, want_l)) if x != y),
            min(len(got_l), len(want_l)),
        )
        return "fail", "decode: event %d is `%s`, EXIficient `%s`" % (
            i,
            got_l[i] if i < len(got_l) else "<end>",
            want_l[i] if i < len(want_l) else "<end>",
        )

    code, back, err = run([*rexi, "decode", *schema, w + ".ref.exi"])
    if code != 0:
        return rexi_error(err)
    with open(w + ".rexi.xml", "wb") as f:
        f.write(back)
    code, _, err = run([*rexi, "encode", *schema, w + ".rexi.xml", w + ".again.exi"])
    if code != 0:
        return rexi_error(err)
    if open(w + ".again.exi", "rb").read() != ref:
        return "fail", "re-encode: the bytes differ from the reference"
    return "pass", ""


def read_expected():
    out = {}
    if not os.path.exists(EXPECTED):
        return out
    for line in open(EXPECTED):
        line = line.split("#", 1)[0].strip()
        if line:
            name, alignment, status = line.split()
            out[(name, alignment)] = status
    return out


def write_expected(results):
    with open(EXPECTED, "w") as f:
        f.write(
            "# Expected results of scripts/w3c-declared.py: <case> <alignment> <status>.\n"
            "# Rewrite with scripts/w3c-declared.py --update, after checking the change.\n"
        )
        for (name, alignment), (status, note) in results.items():
            line = "%s %s %s" % (name, alignment, status)
            if note and status != "pass":
                line += "  # " + note[:120]
            f.write(line + "\n")


def main():
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument(
        "--rexi",
        default=os.path.join(ROOT, "target", "release", "rexi"),
        help="the rexi tool (default: target/release/rexi; "
        "build it with cargo build --release -p rexi-xml --bin rexi)",
    )
    ap.add_argument(
        "--update", action="store_true", help="rewrite scripts/w3c-declared.expected"
    )
    ap.add_argument(
        "--report", metavar="FILE", help="also write a Markdown table of every result"
    )
    a = ap.parse_args()

    tp_dir = subprocess.run(
        [os.path.join(HERE, "fetch-third-party.sh"), "--print-dir"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout.strip()
    testsuite = os.path.join(tp_dir, "exi-testsuite")
    a.oracle = os.environ.get("REXI_ORACLE_JAR") or os.path.join(
        tp_dir, "exip", "utils", "xsd2exip", "target", "xsd2exip-cli.jar"
    )
    for path, what in (
        (a.rexi, "the rexi tool"),
        (a.oracle, "the EXIficient oracle"),
        (testsuite, "exi-testsuite"),
    ):
        if not os.path.exists(path):
            sys.exit(
                "w3c-declared: %s not found at %s (run scripts/fetch-third-party.sh --build)"
                % (what, path)
            )
    a.work = os.path.join(ROOT, "target", "w3c-declared")
    os.makedirs(a.work, exist_ok=True)

    results = {}
    for name, tp in cases(testsuite):
        for alignment in ALIGNMENTS:
            results[(name, alignment)] = one(a, name, tp, alignment)

    names = list(dict.fromkeys(n for n, _ in results))
    passing = [
        n for n in names if all(results[(n, al)][0] == "pass" for al in ALIGNMENTS)
    ]
    print(
        "W3C declaredProductions: %d of %d cases give EXIficient's bytes and events"
        % (len(passing), len(names))
    )
    counts = {}
    for status, _ in results.values():
        counts[status] = counts.get(status, 0) + 1
    print(
        "  per case and alignment: "
        + ", ".join("%s %d" % kv for kv in sorted(counts.items()))
    )

    if a.report:
        with open(a.report, "w") as f:
            f.write("| case | alignment | result | note |\n|---|---|---|---|\n")
            for (name, alignment), (status, note) in results.items():
                f.write(
                    "| %s | %s | %s | %s |\n"
                    % (name, alignment, status, note.replace("|", "\\|"))
                )

    if a.update:
        write_expected(results)
        print("wrote " + os.path.relpath(EXPECTED, ROOT))
        return 0

    expected = read_expected()
    diffs = []
    for key, (status, note) in results.items():
        want = expected.get(key, "<missing>")
        if status != want:
            diffs.append(
                "  %s %s: %s, expected %s  %s" % (key[0], key[1], status, want, note)
            )
    for key in expected.keys() - results.keys():
        diffs.append(
            "  %s %s: expected %s, but the suite has no such case"
            % (key[0], key[1], expected[key])
        )
    if diffs:
        print("results differ from scripts/w3c-declared.expected:")
        print("\n".join(diffs))
        print("If the change is right, run scripts/w3c-declared.py --update.")
        return 1
    print("results match scripts/w3c-declared.expected")
    return 0


if __name__ == "__main__":
    sys.exit(main())
