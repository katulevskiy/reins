#!/usr/bin/env python3
"""Cancel the runs of a pull request that was just closed or merged.

Run by .github/workflows/cancel-closed-pr-runs.yml on `pull_request: closed`. A bundle merge closes many pull requests
at once, and their runs that were still queued or running kept taking macOS runners the release needed. This cancels
every queued or running run of WORKFLOWS whose head is the closed pull request's branch, in the pull request's own
repository. It never touches a push run (main's CI, which each release waits for), a release run, or a run of another
repository's branch of the same name (a fork's `main`).

Environment: GITHUB_TOKEN (actions: write), GITHUB_REPOSITORY, HEAD_REF and HEAD_REPO (the closed pull request's),
DEFAULT_BRANCH.
"""

from __future__ import annotations

import json
import os
import sys
import urllib.error
import urllib.request
from urllib.parse import quote

# CI, iOS, the iOS core cache and the agents' iOS jobs: what runs on a pull request's branch.
WORKFLOWS = {"ci.yml", "ios.yml", "ios-core.yml", "ios-job.yml"}
ACTIVE = {"requested", "waiting", "pending", "queued", "in_progress"}


def to_cancel(runs: list[dict], head_ref: str, head_repo: str, default_branch: str) -> list[dict]:
    """The active runs of the closed pull request's branch, never main's or a release's."""
    if not head_ref or not head_repo:
        return []
    chosen = []
    for run in runs:
        workflow = (run.get("path") or "").rsplit("/", 1)[-1]
        if workflow not in WORKFLOWS or run.get("status") not in ACTIVE:
            continue
        if run.get("event") == "push" or run.get("head_branch") != head_ref:
            continue
        if (run.get("head_repository") or {}).get("full_name") != head_repo:
            continue
        # A branch named like the default branch is main's own only in the base repository; that case never cancels.
        if head_ref == default_branch and head_repo == os.environ.get("GITHUB_REPOSITORY"):
            continue
        chosen.append(run)
    return chosen


def request(method: str, path: str):
    url = os.environ.get("GITHUB_API_URL", "https://api.github.com") + path
    req = urllib.request.Request(url, method=method, headers={
        "Authorization": f"Bearer {os.environ['GITHUB_TOKEN']}",
        "Accept": "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
    })
    with urllib.request.urlopen(req, timeout=30) as response:
        text = response.read()
        return json.loads(text) if text else None


def main() -> int:
    repo = os.environ["GITHUB_REPOSITORY"]
    head_ref = os.environ.get("HEAD_REF", "")
    head_repo = os.environ.get("HEAD_REPO", "")
    default_branch = os.environ.get("DEFAULT_BRANCH", "main")
    runs: list[dict] = []
    page = 1
    while page <= 5:
        batch = request("GET", f"/repos/{repo}/actions/runs?branch={quote(head_ref, safe='')}"
                               f"&per_page=100&page={page}")["workflow_runs"]
        runs.extend(batch)
        if len(batch) < 100:
            break
        page += 1
    cancelled = []
    for run in to_cancel(runs, head_ref, head_repo, default_branch):
        try:
            request("POST", f"/repos/{repo}/actions/runs/{run['id']}/cancel")
            cancelled.append(run)
        except (OSError, urllib.error.HTTPError) as error:  # finished meanwhile, or not cancellable
            print(f"::warning::could not cancel run {run['id']}: {error}")
    lines = [f"Cancelled {len(cancelled)} run(s) of `{head_ref}`:"]
    lines += [f"- [{(r.get('path') or '').rsplit('/', 1)[-1]} {r['id']}]({r.get('html_url', '')}) ({r.get('status')})"
              for r in cancelled]
    text = "\n".join(lines)
    print(text)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a") as f:
            f.write(text + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
