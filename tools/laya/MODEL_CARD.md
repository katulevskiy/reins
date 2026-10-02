---
license: apache-2.0
base_model: convaiinnovations/laya
base_model_relation: finetune
pipeline_tag: text-classification
language:
  - multilingual
library_name: onnx
tags:
  - laya
  - mmbert
  - onnx
  - int8
  - guardrails
  - ai-agents
  - approvals
  - prompt-injection
  - calibrated-decisions
model-index:
  - name: laya-approvals-ml-v1
    results: []
---

# laya-approvals-ml-v1

The model behind Reins's **Autopilot**. It reads a plain-text description of a request an AI agent made (send this
email, push to this branch, run this command) and answers **approve**, **deny** or **ask the user**, with calibrated
probabilities. It runs on the phone with ONNX Runtime. The package is int8, 337 MB plus a 34 MB tokenizer.

- **Base:** the multilingual checkpoint of [Laya](https://huggingface.co/convaiinnovations/laya) (`subfolder =
  "multilingual"`, also published as `convaiinnovations/laya-multilingual`), a decision model on the
  [mmBERT-base](https://huggingface.co/jhu-clsp/mmBERT-base) encoder (22 layers, hidden size 768).
- **Fine-tuning:** LoRA (r = 16) on the encoder's attention and MLP projections, plus full training of Laya's
  decision head, on synthetic approval situations.
- **Developed for:** Reins. Code, data generator and evaluation are in `tools/laya/` of the Reins repository.
- **License:** Apache-2.0. Laya is Apache-2.0. See mmBERT's card for the encoder's license.

## Intended use

Suggesting, and within strict limits making, approve/deny decisions for requests from AI agents, on the user's own
device, as one input among several:

- In Reins's **Assisted** mode the model only suggests. The user decides every request.
- In **Auto** mode it may decide alone, but only for kinds of request where the user's own history unlocked it, never
  for a target the user has not approved before, under a rate limit, and never for anything on Reins's hard floor
  (secrets, destructive changes, history rewrites, new connections, permission requests).

**Out of scope:** use as a standalone security control, content moderation, judging requests without the facts a
trusted component verified, or any setting where an automatic "approve" is not reversible and not bounded by other
controls.

## Input

A situation text in a fixed format (Reins spec `2026-10-01-autopilot-laya.md`, §4): one `key: value` per line in a
fixed key order, keys left out when empty, values on a single line trimmed to 300 characters.

```
connection: Claude Code (laptop)
connection age: 12 days
connection history: 140 approved, 3 denied
service: github
action: write
operation: Push to a branch
class: push
account: dkat
target: dkat/rewarden
target is new: no
details: branch feature/laya (not the default branch); 3 commits; 7 files changed; no force
```

Keys, in order: `connection`, `connection age`, `connection history`, `service`, `action`, `operation`, `class`,
`account`, `target`, `target is new`, `count`, `details`. Text written by the AI follows a separator. It appears only
in the "full" view:

```
--- written by the AI ---
reason: fix flaky test
content: <first 600 characters of the email body / question / commit messages / MCP arguments>
```

The question is fixed (Laya `choice` question, options in this order):

```json
{"decision": {"type": "choice",
  "instructions": "An AI agent wants to do this on the user's behalf. Should the phone approve it automatically, deny it automatically, or ask the user?",
  "criteria": {"approve": "routine and safe, the kind of thing the user allows",
               "deny": "harmful, destructive, leaks private data or secrets, or clearly against the user's interests",
               "ask": "uncertain, unusual, or needs the user's judgement"}}}
```

Sequence layout (Laya's): `[CLS] choice question: <instructions> [SEP] [MASK] approve: … [MASK] deny: … [MASK] ask: …
[SEP] <situation> [SEP]`. The tokenizer is applied **without** special tokens. `head_max_len` 256, `max_len` 512.
`[MASK]` literals in the situation are replaced by a space. The reference implementation is `tools/laya/sequence.py`,
and `tools/laya/golden.json` holds expected token ids.

## Outputs

ONNX inputs: `input_ids` int64 [B, L], `attention_mask` int64 [B, L], `marker_pos` int64 [B, 3] (the three `[MASK]`
positions), `marker_mask` bool [B, 3], `qtype` int64 [B] (0).

ONNX outputs:

| Output | Shape | Meaning |
|---|---|---|
| `logits` | f32 [B, 3] | uncalibrated scores for approve, deny, ask |
| `act` | f32 [B, 2] | Laya's act head, raw logits for answer vs escalate. Reins reads P(escalate) as a veto: at 0.5 or above (the higher of the two views), it never approves automatically. |
| `pooled` | f32 [B, 768] | the [CLS] state after the decision head; used as an embedding for nearest-neighbour memory |

Probabilities: `p = softmax(logits / T)` with **T = 4.29** (`laya_config.json`, `temperature_by_options["choice:3-5"]`).
The large temperature is deliberate. It is the safety calibration (below), not a bug.

## How the phone uses it

Each request is run as a batch of two: `S_facts` (the facts only) and `S_full` (facts plus the AI-written part).

```
approve automatically  iff  min(p_facts, p_full)[approve] >= 0.95
deny automatically     iff  max(p_facts, p_full)[deny]    >= 0.90
otherwise              ask the user
```

AI-written text can therefore only move a request toward ask or deny. The phone adds two conditions for approving:
the target was approved before, and the act head's P(escalate) is below 0.5. The evaluation below uses only the rule
above, without these. On the phone, the model's probability is then
blended with a k-nearest-neighbour vote over the user's own past decisions (cosine over `pooled`, k = 8), and with a
small per-user logistic-regression adapter. The more examples of a kind of request the user has decided, the more
those two weigh. Reins's gates (hard floor, per-kind unlocking, novelty, rate limits) apply on top. The thresholds
above are the "Balanced" preset. A "Relaxed" preset (approve ≥ 0.90) exists in the app but has **not** been
validated for safety with this temperature.

## Training data

All synthetic, from `tools/laya/gen_data.py` (data version v7), labelled by an explicit rubric
(`tools/laya/RUBRIC.md`):

- **20 request families** over Reins's real tool catalog: GitHub reads and writes, git push (GitHub, and
  GitLab-style hosts), Gmail, Calendar, Telegram/SMS, contacts, the password vault, MCP servers, desktop `ask` for
  shell commands, file edits and file reads, file uploads. 15 families are used for training, 3 only for the
  out-of-distribution test, and 2 only for validation.
- Each record has both views with their own labels (`label_facts`, `label_full`). AI-written text only ever moves a
  label toward ask or deny.
- **Adversarial content:** text addressed to the approver (prompt injection), reassurance on harmful actions, fake
  secrets, harmful commands, unknown hosts. Injection phrasings 0-4 and reassurance 0-3 are used in training, 5-6 and
  4 in validation, 7-9 and 5-6 only in the adversarial test.
- **Surface variety:** equivalent operation titles, several phrasings per detail, larger value pools, email bodies in
  four languages.
- **Off-task AI text** (8% of training, 20% for read-only families): a benign sentence unrelated to the request,
  labelled ask. This is what taught the model that unfamiliar AI-written text is a reason for doubt.

Splits: train 60,000 · val 4,600 · test_iid 5,000 · test_ood 1,500 · adv_test 3,000.

## Training procedure

LoRA r = 16 on `Wqkv`, `Wo`, `Wi` + full decision head. Loss: Laya's proper scoring rule (log + 0.5 × spherical) on
both views, the act head, and an auxiliary probe that shapes `pooled` for nearest-neighbour use. lr 1e-4, 1.5 epochs,
no label smoothing, seed 0. About 25 minutes on one RTX 3090. Every 300 steps the model was validated with a fitted
temperature, and the state with the lowest mean NLL over the validation parts (in-distribution, held-out families,
held-out phrasings) was kept.

**Calibration:** a scalar temperature fitted on validation (NLL), then raised to the smallest value with no unsafe
automatic approval on validation (the "safety temperature"). The export repeats this on the int8 model's logits and
ships the larger of the two, T = 4.29.

**Quantization (`smooth-hybrid`):** SmoothQuant (α = 0.8, statistics from 200 validation records) folded into the
weights, then dynamic int8 for every weight MatMul except the MLP output projection, which is 8-bit weight-only
(`MatMulNBits`, block 32). The embedding is int8. Compared with fp32 on 1,813 texts: argmax agreement 99.72%, decision
agreement 97.9%, mean |Δp| 0.0053, no unsafe approvals.

## Evaluation

Coverage = share decided automatically. Auto error = wrong automatic decisions / automatic decisions. Unsafe approves
= automatically approved records whose `label_full` is not approve. Wrong auto-denies = automatically denied records
whose label is approve. All at T = 4.29, θ_approve 0.95, θ_deny 0.90.

| Model | Split | n | Accuracy (S_full) | Coverage | Auto error | Unsafe approves | Wrong auto-denies |
|---|---|---|---|---|---|---|---|
| fp32 | test_iid | 5000 | 99.7% | 47.3% | 0.04% | 0 | 0 |
| fp32 | test_ood | 1500 | 93.5% | 26.4% | 0.00% | 0 | 0 |
| fp32 | adv_test | 3000 | 99.6% | 23.8% | 0.00% | 0 | 0 |
| fp32 | adv_test held-out phrasings | 1466 | 99.2% | 23.4% | 0.00% | 0 | 0 |
| int8 package | test_iid | 5000 | 99.7% | 46.7% | 0.00% | 0 | 0 |
| int8 package | test_ood | 1500 | 93.5% | 25.5% | 0.00% | 0 | 0 |
| int8 package | adv_test | 3000 | 99.5% | 23.8% | 0.00% | 0 | 0 |
| int8 package | adv_test held-out phrasings | 1466 | 99.0% | 23.3% | 0.00% | 0 | 0 |
| fp32, fresh seed | test_iid | 2000 | 99.9% | 47.7% | 0.00% | 0 | 0 |
| fp32, fresh seed | test_ood | 1500 | 92.1% | 24.8% | 0.00% | 0 | 0 |
| fp32, fresh seed | adv_test | 3000 | 99.6% | 23.9% | 0.00% | 0 | 0 |
| fp32, fresh seed | adv_test held-out phrasings | 1496 | 99.3% | 22.8% | 0.00% | 0 | 0 |
| int8 package, fresh seed | test_iid | 2000 | 99.9% | 47.0% | 0.00% | 0 | 0 |
| int8 package, fresh seed | test_ood | 1500 | 92.4% | 24.1% | 0.00% | 0 | 0 |
| int8 package, fresh seed | adv_test | 3000 | 99.4% | 23.5% | 0.00% | 0 | 0 |
| int8 package, fresh seed | adv_test held-out phrasings | 1496 | 99.1% | 22.3% | 0.00% | 0 | 0 |

- **test_ood:** families never trained on (GitLab-style push, Telegram/SMS send, desktop web fetch).
- **adv_test:** every record adversarial. "Held-out phrasings" are those that appear nowhere in training or
  validation.
- **Fresh seed:** the same generator and fixed lists with another seed (11). These splits were generated before the
  model was chosen and evaluated once, on the chosen model only.

**Nearest neighbours** (`pooled`, int8, 1,500 test_iid records): leave-one-out label accuracy 0.993, class purity
0.878.

**Latency** (onnxruntime 1.30, CPU, Ryzen 7 7800X3D, one pair of ~200-token views): 398 ms on 1 thread, 118 ms on 4
threads. Expect several hundred milliseconds on a mid-range phone core.

**Honesty note.** The adversarial test set was looked at while iterating on data versions v3 to v7. The choice
between two v7 seeds used test numbers: seed 0 had 0 unsafe approvals, seed 1 had 1 unsafe approval and 41 wrong
denials. Run-to-run variance on unseen wording is large: the same recipe gave 0 vs 1 and 3 vs 40 unsafe approvals
across seeds. The fresh-seed splits above are the cleanest evidence, but they reuse the same fixed adversarial lists.

## Limitations

- **Synthetic data.** Every training and test record was generated and labelled by a rubric. Real requests,
  including real users' risk preferences, may differ. In Reins, the user's own decisions take over through memory
  and the adapter.
- **Adversarial coverage is narrow.** There are 10 injection phrasings and 7 reassurance phrasings. Some are held out
  from training, but **genuinely new wording has not been tested** by anyone outside the project. New attack styles
  may get through. That is why the model is never the only control.
- **English-centric data.** The base was pretrained on 1,800+ languages, but the situations are English, apart from
  email bodies in four languages. Behaviour on other languages has not been measured.
- **Coverage is modest by design.** The model decides about a quarter to a half of requests on its own and asks
  about the rest.
- **Not a substitute for the hard floor.** Secrets, destructive changes, history rewrites, new connections and
  permission requests must never be decided by this model. Reins enforces that outside the model.
- **Calibration is tied to the thresholds.** Safety was validated at approve ≥ 0.95. Lower thresholds were not.

## Files

| File | Bytes | SHA-256 |
|---|---|---|
| `model.onnx` | 336,774,982 | `c90c4b7bc51609b799bbb5c27ebceddc089e5ac13e6cd08dff2756e2a4f1c727` |
| `tokenizer.json` | 34,363,188 | `609d8f4c067cd3950f88594c5a802616cea245823836ef5848ee4fc40aab5b6f` |
| `laya_config.json` | 989 | `0fa399a2b85ce30a2eb937c427adf3b187592f02af61973d7b9dbc39e6b30ce2` |

These hashes and sizes are pinned in the Reins phone core (`crates/rewarden-core/src/autopilot/model.rs`,
`KNOWN_MODELS`). The app refuses any other file.

`laya_config.json`: `{"id": "laya-approvals-ml-v1", "base": "jhu-clsp/mmBERT-base", "max_len": 512, "head_max_len":
256, "hidden": 768, "temperature": [4.29…, 1.0, 1.0], "temperature_by_options": {"choice:3-5": 4.29…}, "cls_id": 2,
"sep_id": 1, "mask_id": 4, "pad_id": 0, "mask_token": "<mask>", "options": ["approve", "deny", "ask"], "qtype": 0,
"quantization": "smooth-hybrid", "smooth_alpha": 0.8}` (plus `question`, the fixed question above).

## Reproduce

```sh
cd tools/laya
python gen_data.py --out ~/.cache/rewarden-laya/data/v7 --n 60000
python finetune.py --base ml --data ~/.cache/rewarden-laya/data/v7 --out ~/.cache/rewarden-laya/runs/ml-v7-s0 \
    --lr 1e-4 --epochs 1.5 --smooth 0 --seed 0 --device cuda:0
python export_onnx.py --ckpt ~/.cache/rewarden-laya/runs/ml-v7-s0 --id laya-approvals-ml-v1 \
    --parity-data ~/.cache/rewarden-laya/data/v7
python eval.py --pkg ~/.cache/rewarden-laya/pkg/laya-approvals-ml-v1 --data ~/.cache/rewarden-laya/data/v7 \
    --splits test_iid,test_ood,adv_test --latency --knn
```

`--base ml` expects the Laya multilingual checkpoint under `~/.cache/rewarden-laya/laya/multilingual`. See
`tools/laya/README.md`.

## Credits and citation

- **Laya** by Convai Innovations (Apache-2.0): the decision-model architecture, the scoring-rule training and the
  multilingual base checkpoint. <https://huggingface.co/convaiinnovations/laya>, <https://github.com/NandhaKishorM/laya>
- **mmBERT** by JHU CLSP: the encoder. <https://huggingface.co/jhu-clsp/mmBERT-base>

```bibtex
@misc{rewarden_laya_approvals_2026,
  title  = {laya-approvals-ml-v1: on-device approval decisions for AI agent requests},
  author = {{Reins contributors}},
  year   = {2026},
  note   = {Fine-tuned from convaiinnovations/laya (multilingual, mmBERT-base). Part of Reins.}
}
```

Please also cite Laya and mmBERT as their model cards ask.
