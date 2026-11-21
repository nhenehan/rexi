#!/usr/bin/env python3
"""Run this repository's EXI tool against the W3C EXI test suite's "strict"
group, with EXIficient 1.0.7 as the reference.

    scripts/w3c-strict.py [--tool PATH] [--jobs N] [--update] [--report FILE]

The same file is in rexi (it drives `rexi`) and in EXIFormat.jl (it drives
`bin/exiformat`, which takes rexi's command line); it finds out which from the
repository it sits in. Keep the two copies the same.

The suite is the pinned exi-testsuite checkout and the reference is the pinned
exip's EXIficient oracle (utils/xsd2exip), both from scripts/fetch-third-party.sh
--build. Nothing from the suite is copied into the repository: the reference
streams are made at run time, under the work folder.

The strict group (config/testCases-interop/strict.xml) is schemaValidDatatypes
(the built-in datatypes, restricted character sets, string tables) plus
schemaInformedDeclaredProductions. The script follows its xi:include lines and
param inheritance the way the suite's Japex driver does: a group's params, then
the included group's, then the test case's. Includes that name a file the suite
does not have are listed and left out. Each case runs bit-packed and
byte-aligned, with the options out of band. For each case and alignment:

  reference  EXIficient encodes the XML, then dumps the events of that stream;
  encode     the tool encodes the XML: its bytes must equal the reference;
  decode     the tool dumps the events of the reference: they must equal EXIficient's;
  re-encode  the tool decodes the reference to XML and encodes that again: the
             bytes must equal the reference.

A case passes when all three steps match. Each result is one of: pass,
unsupported (the tool refuses the stream as an unsupported feature, or the case
needs an option the tool does not have), fail (anything else), skip (no
reference can be made). The results are compared with scripts/w3c-strict.expected,
and the run fails on any difference, so a regression fails and so does a case
that passes without being listed as passing. --update rewrites that file instead.
"""

import argparse
import concurrent.futures
import os
import re
import subprocess
import sys
import xml.etree.ElementTree as ET

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
CFG_REL = "config/testCases-interop"
GROUP = "strict.xml"
P = "org.w3c.exi.ttf."
XI = "{http://www.w3.org/2001/XInclude}include"
ALIGNMENTS = ("bit-packed", "byte-aligned")
EXPECTED = os.path.join(HERE, "w3c-strict.expected")

if os.path.exists(os.path.join(ROOT, "Cargo.toml")):
    TOOL_NAME = "rexi"
    DEFAULT_TOOL = os.path.join(ROOT, "target", "release", "rexi")
    TOOL_HINT = "cargo build --release -p rexi-xml --bin rexi"
    WORK = os.path.join(ROOT, "target", "w3c-strict")
    ORACLE_ENV = "REXI_ORACLE_JAR"
else:
    TOOL_NAME = "EXIFormat"
    DEFAULT_TOOL = os.path.join(ROOT, "bin", "exiformat")
    TOOL_HINT = "it is bin/exiformat in this repository"
    WORK = os.path.join(ROOT, ".w3c-strict")
    ORACLE_ENV = "EXIFORMAT_ORACLE_JAR"


def subst(value, env):
    return re.sub(r"\$\{([^}]+)\}", lambda m: env.get(m.group(1), m.group(0)), value)


def params(elem, inherited):
    out = dict(inherited)
    for p in elem.findall("{*}param"):
        out[p.get("name")] = p.get("value")
    return out


def walk(path, inherited, missing):
    """Yields (name, params) for every test case under the group file `path`."""
    if not os.path.exists(path):
        missing.append(path)
        return
    group = ET.parse(path).getroot()
    gp = params(group, inherited)
    for child in group:
        if child.tag == XI:
            yield from walk(
                os.path.join(os.path.dirname(path), child.get("href")), gp, missing
            )
        elif child.tag.endswith("}testCase"):
            yield child.get("name"), params(child, gp)


def cases(testsuite, missing):
    """(name, params) for the whole strict group, in suite order, params resolved."""
    env = {"japex.exi.ttfms.testCasesDir": os.path.join(testsuite, "data")}
    out = []
    for name, tp in walk(os.path.join(testsuite, CFG_REL, GROUP), {}, missing):
        for _ in range(6):  # params refer to each other
            tp = {k: subst(v, {**env, **tp}) for k, v in tp.items()}
        out.append((name, tp))
    return out


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


def tool_error(err):
    """("unsupported" or "fail", the message) for a failed tool command."""
    msg = last_line(err)
    unsupported = (
        "unsupported EXI feature" in msg  # rexi
        or "not supported: " in msg  # EXIFormat
        or "not supported yet" in msg
    )
    return ("unsupported" if unsupported else "fail"), msg


def one(a, name, tp, alignment):
    """Runs one case on one alignment. Returns (status, note)."""
    if tp.get(P + "useSchemas") != "true":
        return "unsupported", "schema-less (built-in grammars)"
    if tp.get(P + "preserve"):
        return "unsupported", "Preserve." + tp[P + "preserve"].replace(" ", ",")
    fragment = tp.get(P + "fragments") == "true"
    if fragment and not a.oracle_fragment:
        return "skip", "fragment: the EXIficient oracle has no --fragment"
    strict = "--no-strict" if tp.get(P + "schemaDeviations") == "true" else "--strict"
    xsd = tp[P + "schemaLocation"]
    xml = tp["japex.inputFile"]
    opts = [strict, "--alignment", alignment]
    if fragment:
        opts.append("--fragment")
    for param, flag in (
        ("valueMaxLength", "--value-max-length"),
        ("valuePartitionCapacity", "--value-partition-capacity"),
    ):
        if tp.get(P + param):
            opts += [flag, tp[P + param]]
    w = os.path.join(a.work, "%s.%s" % (name, alignment))

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
    if fragment and not a.tool_fragment:
        return "unsupported", "fragment: %s has no --fragment" % TOOL_NAME

    # The suite's schemas import each other by relative path; allow any file.
    schema = ["--xsd", xsd, "--schema-root", "/", *opts]

    code, _, err = run([a.tool, "encode", *schema, xml, w + ".tool.exi"])
    if code != 0:
        return tool_error(err)
    got = open(w + ".tool.exi", "rb").read()
    if got != ref:
        return "fail", "encode: %d bytes, EXIficient %d (%s vs %s)" % (
            len(got),
            len(ref),
            got.hex()[:40],
            ref.hex()[:40],
        )

    code, events, err = run([a.tool, "dump-events", *schema, w + ".ref.exi"])
    if code != 0:
        return tool_error(err)
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

    code, back, err = run([a.tool, "decode", *schema, w + ".ref.exi"])
    if code != 0:
        return tool_error(err)
    with open(w + ".tool.xml", "wb") as f:
        f.write(back)
    code, _, err = run([a.tool, "encode", *schema, w + ".tool.xml", w + ".again.exi"])
    if code != 0:
        return tool_error(err)
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
            "# Expected results of scripts/w3c-strict.py for %s: <case> <alignment> <status>.\n"
            "# Rewrite with scripts/w3c-strict.py --update, after checking the change.\n"
            % TOOL_NAME
        )
        for (name, alignment), (status, note) in results.items():
            line = "%s %s %s" % (name, alignment, status)
            if note and status != "pass":
                line += "  # " + note[:120]
            f.write(line + "\n")


def help_has(cmd, flag):
    code, out, err = run(cmd)
    return flag in out.decode("utf-8", "replace") + err



def main():
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument(
        "--tool",
        default=DEFAULT_TOOL,
        help="the %s tool (default: %s; %s)"
        % (TOOL_NAME, os.path.relpath(DEFAULT_TOOL, ROOT), TOOL_HINT),
    )
    ap.add_argument(
        "--jobs",
        type=int,
        default=max(1, (os.cpu_count() or 2) // 2),
        help="cases run at once",
    )
    ap.add_argument(
        "--update", action="store_true", help="rewrite scripts/w3c-strict.expected"
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
    a.oracle = os.environ.get(ORACLE_ENV) or os.path.join(
        tp_dir, "exip", "utils", "xsd2exip", "target", "xsd2exip-cli.jar"
    )
    for path, what in (
        (a.tool, "the %s tool" % TOOL_NAME),
        (a.oracle, "the EXIficient oracle"),
        (testsuite, "exi-testsuite"),
    ):
        if not os.path.exists(path):
            sys.exit(
                "w3c-strict: %s not found at %s (run scripts/fetch-third-party.sh --build)"
                % (what, path)
            )
    a.oracle_fragment = help_has(["java", "-jar", a.oracle, "--help"], "--fragment")
    a.tool_fragment = help_has([a.tool, "encode", "--help"], "--fragment")
    a.work = WORK
    os.makedirs(a.work, exist_ok=True)

    missing = []
    all_cases = cases(testsuite, missing)
    for path in missing:
        print(
            "note: the suite includes %s, which it does not have"
            % os.path.relpath(path, testsuite)
        )

    jobs = [(name, tp, al) for name, tp in all_cases for al in ALIGNMENTS]
    with concurrent.futures.ThreadPoolExecutor(max_workers=a.jobs) as pool:
        done = list(pool.map(lambda j: one(a, *j), jobs))
    results = {(name, al): r for (name, _, al), r in zip(jobs, done)}

    names = list(dict.fromkeys(n for n, _ in results))
    passing = [
        n for n in names if all(results[(n, al)][0] == "pass" for al in ALIGNMENTS)
    ]
    print(
        "W3C strict group (%s): %d of %d cases give EXIficient's bytes and events"
        % (TOOL_NAME, len(passing), len(names))
    )
    for part, prefix in (
        ("schemaValidDatatypes", None),
        ("declaredProductions", "schemaInformed.declared."),
    ):
        part_names = [
            n
            for n in names
            if (
                n.startswith(prefix)
                if prefix
                else not n.startswith("schemaInformed.declared.")
            )
        ]
        print(
            "  %s: %d of %d"
            % (part, len([n for n in part_names if n in passing]), len(part_names))
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
        print("results differ from scripts/w3c-strict.expected:")
        print("\n".join(diffs))
        print("If the change is right, run scripts/w3c-strict.py --update.")
        return 1
    print("results match scripts/w3c-strict.expected")
    return 0


if __name__ == "__main__":
    sys.exit(main())
