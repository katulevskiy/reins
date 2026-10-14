#!/usr/bin/env python3
"""Clears the way for this release run (release.yml, job `supersede`): older release runs on main that have not
reached publication are cancelled, and those publishing are waited for.

An older run is cancelled only when it is a push run on main, this run is a push run too, and none of its
publication jobs (PUBLISH_STAGE) exists yet. GitHub creates a job only once the jobs it needs are done, so a run whose
job list has none of them has published nothing and will publish nothing more. A run that has reached publication is
never cancelled: while its "Publish the release" job runs, this run waits for it, because the next version comes from
the tags (scripts/next-version.sh) and that job creates the older run's tag. Runs started by hand
(workflow_dispatch, including verify_only) and runs on other branches are never cancelled.

    GH_REPO=owner/repo REINS_RUN_ID=<id> REINS_RUN_NUMBER=<n> REINS_EVENT=push scripts/supersede-releases.py

Needs `gh` with a token that may cancel runs (actions: write). REINS_POLL_SECONDS and REINS_POLL_ATTEMPTS set the
wait (default: every 15 s for 45 minutes; publication itself waits up to 30 minutes for CI), REINS_GH_RETRY_SECONDS
the pause before asking `gh` again after an API error.
"""

import json
import os
import subprocess
import sys
import time

WORKFLOW = "release.yml"
PUBLISH = "Publish the release"
# The publication job and the jobs that run after it.
PUBLISH_STAGE = frozenset({PUBLISH, "Google Play upload", "Homebrew cask", "AUR (reins-bin)"})


def decide(run, jobs, event):
    """What to do about an older, unfinished release run: ("cancel" | "wait" | "leave", why)."""
    started = sorted({job["name"] for job in jobs} & PUBLISH_STAGE)
    if started:
        publish = next((job for job in jobs if job["name"] == PUBLISH), None)
        if publish is not None and publish["status"] != "completed":
            return "wait", f"{PUBLISH!r} is running; its tag comes before this run's version"
        return "leave", f"past publication ({', '.join(started)})"
    if run.get("headBranch") != "main":
        return "leave", f"a run on {run.get('headBranch')!r}, not main"
    if run.get("event") != "push":
        return "leave", f"started by hand ({run.get('event')}); never cancelled"
    if event != "push":
        return "leave", "this run was started by hand; only a newer push supersedes a release"
    return "cancel", "a newer push is being released, and it has not reached publication"


def gh(*args, attempts=3):
    """`gh` with a few tries: an API error is asked again before it fails this job."""
    delay = float(os.environ.get("REINS_GH_RETRY_SECONDS", "5"))
    for attempt in range(1, attempts + 1):
        result = subprocess.run(["gh", *args], capture_output=True, text=True, check=False)
        if result.returncode == 0:
            return result.stdout
        if attempt == attempts:
            raise RuntimeError(f"gh {' '.join(args)}: {result.stderr.strip() or result.returncode}")
        time.sleep(delay * attempt)
    return ""


def jobs_of(run_id):
    return json.loads(gh("run", "view", str(run_id), "--json", "jobs") or "{}").get("jobs", [])


def label(run):
    return f"release run #{run['number']} ({run['databaseId']}, {run.get('event')}, {run.get('headSha', '')[:8]})"


def main(environ=os.environ):
    current = int(environ["REINS_RUN_ID"])
    number = int(environ["REINS_RUN_NUMBER"])
    event = environ.get("REINS_EVENT", "")
    seconds = float(environ.get("REINS_POLL_SECONDS", "15"))
    attempts = int(environ.get("REINS_POLL_ATTEMPTS", "180"))
    runs = json.loads(gh("run", "list", "--workflow", WORKFLOW, "--branch", "main", "--limit", "100", "--json",
                         "databaseId,number,event,status,headBranch,headSha") or "[]")
    older = [run for run in runs
             if run["status"] != "completed" and run["number"] < number and run["databaseId"] != current]
    waiting = []
    for run in sorted(older, key=lambda run: run["number"]):
        action, why = decide(run, jobs_of(run["databaseId"]), event)
        if action == "cancel":
            # Asked again right before cancelling: publication may have started since.
            action, why = decide(run, jobs_of(run["databaseId"]), event)
        if action == "cancel":
            try:
                gh("run", "cancel", str(run["databaseId"]))
            except RuntimeError:
                status = json.loads(gh("run", "view", str(run["databaseId"]), "--json", "status")).get("status")
                if status != "completed":
                    raise
                print(f"{label(run)} finished before it could be cancelled")
                continue
            print(f"::notice::Cancelled {label(run)}: {why}")
        elif action == "wait":
            print(f"Waiting for {label(run)}: {why}")
            waiting.append(run)
        else:
            print(f"Left {label(run)} alone: {why}")
    for run in waiting:
        for attempt in range(1, attempts + 1):
            publish = next((job for job in jobs_of(run["databaseId"]) if job["name"] == PUBLISH), None)
            if publish is None or publish["status"] == "completed":
                print(f"{label(run)} has finished publishing ({(publish or {}).get('conclusion') or 'gone'})")
                break
            if attempt == attempts:
                print(f"::error::{label(run)} is still publishing; its tag may collide with this run's version "
                      "(the publish job refuses a tag that exists)", file=sys.stderr)
                return 1
            time.sleep(seconds)
    if not older:
        print("No older release run is in progress")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except RuntimeError as error:
        print(f"::error::{error}", file=sys.stderr)
        sys.exit(1)
