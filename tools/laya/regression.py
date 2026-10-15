# SPDX-License-Identifier: Apache-2.0
"""Non-regression of the core's risk reading: every input must be at least as guarded on this branch as on main.

Two `laya-context` binaries, main's and this branch's, read the same inputs:
- the hostile fixtures and their look-alikes (`RISK_FIXTURES_OUT=… cargo test -p reins-core --lib dump_fixtures --
  --ignored`);
- the golden set's actions;
- a corpus of generated actions (`gen_risk.py`'s families).

A desktop question is read as each version reads it: this branch through `{"ask": …}` (risk::ask_context), main
through `{"command": <the detail after "Command:">}` or `{"files": …}`, which is what its refine read. Each outcome is
ranked read < uncertain < tripwire, and the run fails when any input ranks lower on the branch. An MCP call had no
hold on main, so whatever the branch reads is at least as guarded. More holds are reported, never failed.

    python regression.py --main /path/laya-context-main --branch /path/laya-context --fixtures /tmp/fixtures.json

Engine-level decisions too: the ignored test `dump_decisions` (crates/reins-core/src/autopilot/tests.rs), run on main
and on the branch over the same commands, writes each command's end in Manual, Auto, Overnight and Bypass;
`--decisions main.json branch.json` fails when the branch approves anything main left waiting or denied.
"""
import argparse
import collections
import json
import os
import random
import sys

from core_context import CoreContext

RANK = {"read": 0, "uncertain": 1, "tripwire": 2}


def rank(ans):
    return "tripwire" if ans["tripwire"] else "uncertain" if ans["unreadable"] else "read"


def main_query(q):
    """What main read for the same input."""
    if "ask" in q:
        detail = q["ask"]["detail"] or ""
        if detail.startswith("Command:"):
            return {"command": detail[len("Command:"):].lstrip(" \t").lstrip("\n")}
        if detail.startswith("Files:"):
            paths = [p.strip() for p in detail[len("Files:"):].splitlines() if p.strip() and not p.strip().startswith("In: ")]
            return {"files": paths}
        return None
    if "mcp" in q:
        return None
    return q


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--main", required=True)
    ap.add_argument("--branch", required=True)
    ap.add_argument("--fixtures", required=True)
    ap.add_argument("--golden", default=os.path.join(os.path.dirname(os.path.abspath(__file__)), "golden.json"))
    ap.add_argument("--corpus", type=int, default=20000)
    ap.add_argument("--decisions", nargs=2, metavar=("MAIN", "BRANCH"), help="dump_decisions outputs to compare")
    args = ap.parse_args()
    looser_decisions = 0
    if args.decisions:
        old_d, new_d = (json.load(open(p)) for p in args.decisions)
        changed = collections.Counter()
        for mode, ends in old_d.items():
            for cmd, end in ends.items():
                new_end = new_d.get(mode, {}).get(cmd)
                if new_end == "approve" and end != "approve":
                    looser_decisions += 1
                    print("  LOOSER decision in %s: main %s -> branch approve | %s" % (mode, end, cmd[:200]))
                elif new_end != end:
                    changed[(mode, end, new_end)] += 1
        print("decisions: %d commands x %d modes; approved on the branch but not on main: %d" % (
            len(next(iter(old_d.values()))), len(old_d), looser_decisions))
        for k, n in changed.most_common():
            print("  %5d %s" % (n, k))
    old, new = CoreContext(args.main), CoreContext(args.branch)

    inputs = []  # (source, branch query)
    for cmd in json.load(open(args.fixtures)):
        inputs.append(("fixture", {"command": cmd}))
        inputs.append(("fixture as a hook", {"ask": {"question": "Claude Code wants to run: " + cmd.split("\n")[0],
                                                     "detail": "Command:\n" + cmd, "topic": None}}))
    for c in json.load(open(args.golden))["risk"]["cases"]:
        if c.get("core"):
            inputs.append(("golden " + c["id"], c["core"]))

    os.environ.setdefault("REINS_LAYA_CONTEXT", args.branch)
    import gen_risk as g
    import gen_data as gd
    g.CORE = new
    rng = random.Random(23)
    fams = [f for f in g.FAMILIES]
    for _ in range(args.corpus):
        fam = rng.choice(fams)
        conn, trust = gd.connection(rng)
        conn = dict(conn)
        conn["_me"] = rng.choice(gd.ME)
        c = g.FAMILIES[fam](rng, conn, trust)
        if not c.core:
            continue
        kind = c.core[0]
        q = (c.ai or {}).get("reason") or ""
        if kind == "command":
            cmd, root = c.core[1], c.core[2]
            inputs.append(("corpus " + fam, {"ask": {"question": q, "detail": "Command:\n%s\n\nIn: %s" % (cmd, root),
                                                    "topic": g.hook_topic("command", cmd)}}))
        elif kind == "files":
            inputs.append(("corpus " + fam, {"ask": {"question": q, "detail": "Files:\n" + "\n".join(c.core[1]),
                                                    "topic": None}}))
        elif kind == "push":
            inputs.append(("corpus " + fam, {"push": {"refs": c.core[1], "default_branch": c.core[2]}}))
        elif kind == "mcp":
            inputs.append(("corpus " + fam, {"mcp": {"tool": c.core[1], "arguments": c.core[2]}}))
        else:
            inputs.append(("corpus " + fam, {"command": c.core[1]}))

    looser, stricter, why = [], collections.Counter(), collections.Counter()
    for source, q in inputs:
        ans = new.ask_raw(q)
        b = rank(ans)
        mq = main_query(q)
        m = rank(old.ask_raw(mq)) if mq is not None else "read"
        if RANK[b] < RANK[m]:
            looser.append((source, m, b, q))
        elif RANK[b] > RANK[m]:
            stricter[(source.split(" ")[0], m, b)] += 1
            # what holds it now: the first phrase, its quoted values left out
            first = (ans["lines"] or ["?"])[0]
            why[(source.split(" ")[0] + " " + (source.split(" ")[1] if source.startswith("corpus") else ""),
                 first.split('"')[0].strip())] += 1
    print("inputs %d; less guarded on the branch: %d" % (len(inputs), len(looser)))
    for source, m, b, q in looser[:40]:
        print("  LOOSER %s: main %s -> branch %s | %s" % (source, m, b, json.dumps(q)[:300]))
    print("more holds (source, main -> branch):")
    for k, n in stricter.most_common():
        print("  %5d %s" % (n, k))
    print("what holds them now:")
    for k, n in why.most_common(40):
        print("  %5d %s" % (n, k))
    sys.exit(1 if looser or looser_decisions else 0)


if __name__ == "__main__":
    main()
