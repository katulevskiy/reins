#!/usr/bin/env python3
"""CI emergency: give one pull request the macOS runners, now.

Run by .github/workflows/ci-emergency.yml (started by hand; needs write access). It:

  1. adds the `ci:lock` label to the pull request (removing it first if it was there, which restarts its
     CI_LOCK_HOURS), so admission keeps a pull request's worth of macOS runners for it and lets its runs go first
     until it is merged, closed or unlabelled (scripts/ci/admit.py);
  2. cancels every other pull request's CI run that is queued, waiting for admission or running;
  3. with CANCEL_BACKGROUND (the default), cancels ios.yml's runs on main and at night too: they gate nothing;
  4. re-runs the pull request's latest CI run if none is in progress and the last one did not pass.

Main's CI runs (each release waits for its commit's) and releases are never cancelled. The cancelled pull requests
need a push or a re-run afterwards.
"""

from __future__ import annotations

import json
import os
import sys
import urllib.request
from urllib.parse import quote

LOCK_LABEL = "ci:lock"
ACTIVE = ("queued", "in_progress", "waiting", "pending", "requested")


def to_cancel(runs: list[dict], head_sha: str) -> list[dict]:
    """Other pull requests' active CI runs."""
    return [run for run in runs
            if run.get("status") in ACTIVE and run.get("head_sha") != head_sha and run.get("event") == "pull_request"]


def background(runs: list[dict]) -> list[dict]:
    """ios.yml's active runs on main and at night (never a hand run: someone is waiting for it)."""
    return [run for run in runs if run.get("status") in ACTIVE and run.get("event") in ("push", "schedule")]


def to_rerun(runs: list[dict], head_sha: str) -> dict | None:
    """The pull request's latest CI run, if none is active and it did not pass."""
    own = sorted((r for r in runs if r.get("head_sha") == head_sha and r.get("event") == "pull_request"),
                 key=lambda r: r["id"], reverse=True)
    if not own or any(r.get("status") in ACTIVE for r in own):
        return None
    return own[0] if own[0].get("conclusion") != "success" else None


def request(method: str, path: str, body: dict | None = None):
    url = os.environ.get("GITHUB_API_URL", "https://api.github.com") + path
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(url, data=data, method=method, headers={
        "Authorization": f"Bearer {os.environ['GITHUB_TOKEN']}",
        "Accept": "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
    })
    with urllib.request.urlopen(req, timeout=30) as response:
        text = response.read()
        return json.loads(text) if text else None


def workflow_runs(repo: str, workflow: str) -> list[dict]:
    runs: list[dict] = []
    for status in ACTIVE:
        page = 1
        while True:
            batch = request("GET", f"/repos/{repo}/actions/workflows/{workflow}/runs"
                                   f"?status={status}&per_page=100&page={page}")["workflow_runs"]
            runs.extend(batch)
            if len(batch) < 100:
                break
            page += 1
    return runs


def cancel(repo: str, runs: list[dict]) -> list[dict]:
    done = []
    for run in runs:
        try:
            request("POST", f"/repos/{repo}/actions/runs/{run['id']}/cancel")
            done.append(run)
        except OSError as error:  # already finished, or not cancellable
            print(f"::warning::could not cancel run {run['id']}: {error}")
    return done


def main() -> int:
    repo = os.environ["GITHUB_REPOSITORY"]
    number = int(os.environ["PR"])
    pr = request("GET", f"/repos/{repo}/pulls/{number}")
    if pr["state"] != "open":
        print(f"::error::#{number} is {pr['state']}")
        return 1
    head = pr["head"]["sha"]

    if any(label["name"] == LOCK_LABEL for label in pr.get("labels", [])):
        # Removed and added again, which restarts its CI_LOCK_HOURS. If the removal fails the old lock stays; the
        # label is still added below (a no-op then), and the cancellations still help.
        try:
            request("DELETE", f"/repos/{repo}/issues/{number}/labels/{quote(LOCK_LABEL)}")
        except OSError as error:
            print(f"::warning::could not remove {LOCK_LABEL} to restart it: {error}")
    request("POST", f"/repos/{repo}/issues/{number}/labels", {"labels": [LOCK_LABEL]})
    lines = [f"### CI emergency for #{number}", "", f"Locked: `{LOCK_LABEL}` added to #{number}."]

    active = workflow_runs(repo, "ci.yml")
    cancelled = cancel(repo, to_cancel(active, head))
    if os.environ.get("CANCEL_BACKGROUND", "true") == "true":
        cancelled += cancel(repo, background(workflow_runs(repo, "ios.yml")))
    lines.append(f"Cancelled {len(cancelled)} other run(s):")
    lines += [f"- [{r.get('display_title', r['id'])}]({r.get('html_url', '')}) ({r.get('event')})" for r in cancelled]

    recent = request("GET", f"/repos/{repo}/actions/workflows/ci.yml/runs?head_sha={head}&per_page=20")["workflow_runs"]
    rerun = to_rerun(active + recent, head)
    if rerun:
        try:
            request("POST", f"/repos/{repo}/actions/runs/{rerun['id']}/rerun")
            lines.append(f"Re-ran #{number}'s CI run {rerun['id']}.")
        except OSError as error:
            print(f"::warning::could not re-run CI run {rerun['id']}: {error}")
            lines.append(f"Could not re-run #{number}'s CI run {rerun['id']} ({error}): re-run it by hand.")
    else:
        lines.append(f"#{number}'s CI is running or passed; nothing to re-run.")
    lines += ["", f"Remove `{LOCK_LABEL}` (or merge or close #{number}) to release the runners."]

    text = "\n".join(lines)
    print(text)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a") as f:
            f.write(text + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
