# SPDX-License-Identifier: Apache-2.0
"""The phone core's own `context` line (docs/autopilot-risk.md section 3), through `laya-context`
(crates/reins-laya/src/bin/laya-context.rs): what the core writes for a command, files or a push, whether a tripwire
fires and whether it could not read the action. The risk data and the golden set use it, so the model is trained on
what the phone shows it.

    cargo build --release -p reins-laya --bin laya-context
"""
import json
import os
import subprocess
from typing import Dict, List, Optional

DEFAULT_BIN = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "target", "release", "laya-context")


class CoreContext:
    def __init__(self, binary: Optional[str] = None):
        path = binary or os.environ.get("REINS_LAYA_CONTEXT") or DEFAULT_BIN
        if not os.path.exists(path):
            raise SystemExit("laya-context not found at %s: cargo build --release -p reins-laya --bin laya-context" % path)
        self.proc = subprocess.Popen([path], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1)
        self.cache: Dict[str, Dict] = {}

    def ask_raw(self, query: Dict) -> Dict:
        key = json.dumps(query, sort_keys=True)
        if key not in self.cache:
            self.proc.stdin.write(key + "\n")
            self.proc.stdin.flush()
            self.cache[key] = json.loads(self.proc.stdout.readline())
        return self.cache[key]

    def command(self, command: str) -> Dict:
        return self.ask_raw({"command": command})

    def files(self, paths: List[str]) -> Dict:
        return self.ask_raw({"files": paths})

    def push(self, refs: List[Dict], default_branch: Optional[str] = None) -> Dict:
        return self.ask_raw({"push": {"refs": refs, "default_branch": default_branch}})

    def ask(self, question: str, detail: str, topic: Optional[str]) -> Dict:
        """A desktop question, read as the phone reads it (risk::ask_context)."""
        return self.ask_raw({"ask": {"question": question, "detail": detail, "topic": topic}})

    def mcp(self, tool: str, arguments: str) -> Dict:
        return self.ask_raw({"mcp": {"tool": tool, "arguments": arguments}})

    def neutral(self, cls: str, target: str, details: List[str], context: List[str], reason: str = "",
                content: str = "") -> Dict:
        """The risk's third view (risk::neutral_facts, risk::neutral_ai): {"target", "details", "context", "reason",
        "content"}."""
        return self.ask_raw({"neutral": {"class": cls, "target": target, "details": details, "context": context,
                                         "reason": reason, "content": content}})
