# Autopilot: on-phone automatic approvals with Laya

Status: building (2026-10-01). Owner of the decisions: the phone. Nothing here runs on the server or the desktop.

## 1. What the user gets

A run mode per connection (and a global default), like Claude Code's permission modes:

| Mode | What happens to a request |
|---|---|
| **Manual** | Today: every request waits for the user. |
| **Assisted** | Every request still waits, but the approval screen shows Autopilot's suggestion ("Autopilot would approve — 97%") and why (similar past decisions). Every answer the user gives is a training example. The default once a model is installed. |
| **Auto** | Autopilot approves when confident it is safe, denies when confident it is unsafe, otherwise asks the user. Only for *action classes* that have earned it (§5.4). |
| **Bypass** | Approves everything except the hard floor (§2). Time-boxed (15 min, 1 h max), per connection or global, shows a persistent notification with "Stop" while on. Needs no model. |
| **Lockdown** | Denies everything immediately (pairings still reach the user). Needs no model. |

All modes work with the app closed: the FCM push wakes the core (PushWorker), the core parks the request, and Autopilot decides right there (§7).

## 2. The hard floor (never automatic, in any mode except Lockdown which denies)

- Pairing requests (new connections) and grant/standing-permission requests (`ApprovalKind::Grant`), accounts sharing (`ApprovalKind::Accounts`).
- Anything with `no_standing == true` (passwords and other vault secrets, destructive/once-only changes: force push that rewrites history, repo/branch delete, visibility, collaborators, transfers, secrets, deploy keys, webhooks, …). This flag is already computed by the core for exactly this purpose.
- Secret release to the desktop (`secrets`), SSH signatures (`ssh`).
- Uploaded files (`PendingKind::Blob`) when the sniffer flagged the content (executable, archive with executables) — other uploads may be automatic.
- A request from a connection paired less than 10 minutes ago (bypass excluded: a just-paired connection cannot be put in bypass for its first 10 minutes either).
Floor items always wait for the user; in Bypass they are notified with high priority.

## 3. Decision pipeline (core, per parked request)

```
parked request ──► ApprovalView (what the user would see, all phone-computed)
                │
                ├─► mode for this connection = Lockdown → deny (audit "auto: lockdown")
                ├─► hard floor? → wait for user
                ├─► mode = Bypass (unexpired) → approve once (audit "auto: bypass")
                ├─► mode = Manual → wait
                └─► Assisted/Auto:
                     situation text  S_full  (facts + AI-written content)
                     situation text  S_facts (facts only, §4)
                     model(S_facts), model(S_full)  → p_base, embedding e (from S_full)
                     memory kNN(e, profile)          → p_knn, neighbours
                     adapter(profile)(features)      → p_user
                     combine → p_approve, p_deny (calibrated), confidence
                     gates (§5.4): class unlocked? novelty? rate limit? floor?
                     Auto & p_approve ≥ θ_a(class) & all gates → approve once
                     Auto & p_deny ≥ θ_d(class)               → deny
                     else wait for user (suggestion attached to the item)
```

**Prompt-injection rule.** AI-written text (an email body, a reason, a PR description, a commit message, an `ask` question's wording) can only push toward deny/ask: the approve probability used is `min(p(S_facts), p(S_full))`, the deny probability is `max(...)`. A request whose facts alone do not look safe cannot be talked into approval.

**Approve means approve once.** Autopilot never creates standing grants and never ticks "remember". The default selection (all items, as the screen would preselect) is used.

## 4. The situation text (shared by training and the phone — keep in sync)

A plain-text block, one `key: value` per line, fixed key order, keys omitted when empty. Rendered by the core from `ApprovalView` + connection facts. The ML tooling (`tools/laya/`) generates training states in exactly this format.

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
and, only in `S_full`, the AI-written part after a separator line:
```
--- written by the AI ---
reason: fix flaky test
content: <first 600 chars of the email body / ask detail / commit messages / MCP arguments>
```

Keys, in order: `connection`, `connection age`, `connection history`, `service`, `action` (ApprovalView.action), `operation` (op_title), `class`, `account`, `target` (repo / chat / calendar / recipient list / MCP server+tool / command for an `ask`), `target is new` (yes/no — never seen in approved history for this connection), `count`, `details` (preview lines and git push facts joined with `; `), then the AI part. Values are single-line, trimmed to 300 chars (content 600). Phone-verified facts (recipient in contacts, repo owned by the user, branch is default) go into `details`. For an `ask` from a harness hook, `target` is the command (it *is* the action) and `content` holds the question wording.

The **question** asked of the model is fixed:

```json
{"decision": {"type": "choice",
  "instructions": "An AI agent wants to do this on the user's behalf. Should the phone approve it automatically, deny it automatically, or ask the user?",
  "criteria": {"approve": "routine and safe, the kind of thing the user allows",
               "deny": "harmful, destructive, leaks private data or secrets, or clearly against the user's interests",
               "ask": "uncertain, unusual, or needs the user's judgement"}}}
```
(option order fixed: approve, deny, ask). The sequence layout is Laya's: `[CLS] "choice question: <instructions>" [SEP] [MASK] approve: … [MASK] deny: … [MASK] ask: … [SEP] <state> [SEP]`, see `tools/laya/sequence.py` (reference implementation; the Rust port must produce identical token ids — golden vectors in `tools/laya/golden.json`).

## 5. Profiles, memory, adapter, thresholds

### 5.1 Profiles
A profile = a named decision personality: `{id, name, emoji/icon, memory, adapter, thresholds, unlocked classes}`. Built-ins created on first use: **Personal** (default for every connection) and **Work**. The user can create, rename, delete (not the last), reset (forget memory). Each connection is assigned one profile (default: the global default profile). Decisions on a connection train only its profile.

### 5.2 Decision memory
Each human decision (approve / deny on a request that went through the pipeline, plus corrections from Activity) stores `{request_id, at, connection_id, class key, situation_facts, embedding (f32 × D, L2-normalised), label (approve|deny), source (user|correction)}` sealed with the data key (`autopilot.memory` AAD) in table `autopilot_memory`. Cap 5000 per profile (oldest evicted). **Automatic decisions never enter the memory** (no self-reinforcement); a correction in Activity ("this should have been denied") enters it with weight 3 and immediately re-locks the class if it was an auto-approve gone wrong.

kNN: cosine over the profile's memory, k = 8, similarity-weighted vote with a temperature; returns `p_knn(approve)` and the neighbours (shown in the UI as "Like 4 times you approved: …").

### 5.3 Adapter (the trainable, per-profile part)
A small model trained on the profile's memory: logistic regression (L2) over `[e (D dims), base logits (3), knn vote (2), class one-hot (hashed, 32), novelty (1)]`. Trained in the core (pure Rust, Adam, a few hundred steps — milliseconds) after every 5 new decisions and on demand; stored sealed in `autopilot_profiles`. Evaluated by leave-one-out-ish holdout to produce the per-class accuracy that unlocks Auto (§5.4).

Final probability: weighted blend `p = w_b·p_base + w_k·p_knn + w_a·p_adapter`, weights rising with the number of examples of that class (no examples → base only; ≥ 20 → mostly memory/adapter). Laya's temperatures calibrate p_base; the blend is re-calibrated by Platt scaling on the holdout when ≥ 30 examples exist.

### 5.4 Gates (Auto only)
- **Class key** = `service/action/class` (e.g. `github/write/push`, `desktop/ask/command`, `gmail/read`).
- A class is *unlocked* for auto-approve when the profile has ≥ 20 human decisions in it and shadow accuracy (what Autopilot would have done vs what the user did, over the last 50) ≥ 95% with zero "would approve / user denied" in the last 20. Auto-deny needs ≥ 10 decisions and no "would deny / user approved" in the last 20. The user can also unlock a class manually from the profile screen (with a warning) or lock it.
- **Novelty**: `target is new` = yes → ask (auto-deny still allowed).
- **Rate limit**: at most 30 auto-approvals per connection per 10 min and 200 per day; beyond that, ask (and notify "Autopilot paused for Claude Code: unusual volume").
- **Thresholds** (per profile, per class, defaults): θ_a = 0.95, θ_d = 0.90; the user picks a preset: Cautious (0.98/0.95), Balanced (0.95/0.90), Relaxed (0.90/0.85).

## 6. Model on the phone

### 6.1 Package
Not in the APK. Downloaded on demand (Wi-Fi by default) from the official site, `<site>/models/<id>/` (`rewarden_proto::official_site!`), and verified against SHA-256 pinned **in the core binary** (`autopilot::model::KNOWN_MODELS`). Files: `model.onnx` (int8), `tokenizer.json`, `laya_config.json` (`{id, max_len, head_max_len, temperature, temperature_by_options, hidden, mask/cls/sep/pad ids}`). Stored under `<data_dir>/models/<id>/`. A model that fails verification is deleted and never loaded.

### 6.2 ONNX I/O
Inputs: `input_ids` int64 [B,L], `attention_mask` int64 [B,L], `marker_pos` int64 [B,K], `marker_mask` bool [B,K], `qtype` int64 [B].
Outputs: `logits` f32 [B,K] (uncalibrated), `act` f32 [B,2] (raw logits), `pooled` f32 [B,H] (the [CLS] state after the decision head; the embedding).

### 6.3 Runtime
The core owns tokenization (Rust `tokenizers`, `fancy-regex`, no C deps), sequence building, calibration and everything after. Inference goes through a foreign trait implemented in Kotlin with ONNX Runtime for Android (and in Rust with the `ort` crate for desktop tests/tools):

```rust
#[uniffi::export(with_foreign)]
pub trait ModelRuntime: Send + Sync {
    /// Loads (or keeps loaded) the ONNX file at `path`.
    fn load(&self, path: String) -> Result<(), ForeignError>;
    fn run(&self, input: ModelInput) -> Result<ModelOutput, ForeignError>;
    fn unload(&self);
}
pub struct ModelInput { batch: u32, seq_len: u32, k: u32, input_ids: Vec<i64>, attention_mask: Vec<i64>, marker_pos: Vec<i64>, marker_mask: Vec<u8>, qtype: Vec<i64> }
pub struct ModelOutput { logits: Vec<f32>, act: Vec<f32>, pooled: Vec<f32>, hidden: u32 }
```
Without an installed model (or if inference fails), Assisted/Auto degrade to Manual for that request (audit notes it); Bypass and Lockdown keep working.

## 7. Background operation

`handle_push` / `sync` park requests exactly as today, then call `Engine::autopilot_pass()` which evaluates every newly parked request (not already evaluated) and approves/denies through the same `approve`/`deny` code the user's taps use. Decisions not made automatically get the suggestion stored with the parked item. The notifier then:
- auto-approved: a quiet, grouped notification on channel "Autopilot" ("Autopilot approved: push to feature/laya — Claude Code"), with an "Undo/Report" action that opens Activity (an approval cannot be undone once executed; "Report" records a correction).
- auto-denied: same channel, default importance.
- needs the user: the normal approval notification, with the suggestion in the text when Assisted.
PushWorker runs expedited; model load + 2 forward passes must fit comfortably (target < 3 s on a mid-range phone with the base checkpoint).

Bypass: a foreground-service-free persistent notification (ongoing, low importance, "Bypass on for Claude Code · 42 min left · Stop"); an alarm/WorkManager job ends it at expiry; the core enforces expiry itself regardless.

## 8. Audit and Activity

`AuditRecord`/`ActivityEntry` gain `decided_by: String` ("" = user, "autopilot", "bypass", "lockdown") and `autopilot: Option<AutopilotNote>` = `{mode, p_approve, p_deny, confidence, profile_id, profile_name, neighbours: Vec<String> (short labels), reason: String}`. Activity gets an "Automatic" filter chip; entries show a small "Autopilot" badge; tapping shows the confidence, the neighbours and **"This was wrong"** (correction → memory, class re-locked).

## 9. Core API (UniFFI, on `RewardenCore`)

As built (`crates/rewarden-core/src/api.rs`, records in `autopilot/types.rs`). All async methods run on the core's runtime.

```rust
fn set_model_runtime(runtime: Arc<dyn ModelRuntime>)            // replaces (and unloads) an earlier one
async fn autopilot_settings() -> Result<AutopilotSettings, CoreError>
async fn set_autopilot_mode(connection_id: Option<String>, mode: Option<AutopilotMode>, minutes: Option<u32>) -> Result<(), CoreError>
    // connection_id None = global; mode None = back to the default (global) / follow the global mode (connection).
    // Bypass: minutes default 15, 1..=60, refused for a connection paired < 10 min ago. Lockdown also denies what waits.
async fn set_autopilot_wifi_only(wifi_only: bool) -> Result<(), CoreError>
async fn autopilot_profiles() -> Result<Vec<ProfileView>, CoreError>          // Personal + Work created on first use
async fn create_profile(name: String, icon: Option<String>) -> Result<ProfileView, CoreError>
async fn rename_profile(profile_id: String, name: String, icon: Option<String>) -> Result<(), CoreError>
async fn delete_profile(profile_id: String) -> Result<(), CoreError>         // not the last one
async fn reset_profile(profile_id: String) -> Result<(), CoreError>          // memory, adapter, manual locks
async fn set_default_profile(profile_id: String) -> Result<(), CoreError>
async fn assign_profile(connection_id: String, profile_id: Option<String>) -> Result<(), CoreError>  // None = default
async fn set_class_lock(profile_id: String, class_key: String, locked: Option<bool>) -> Result<(), CoreError>
    // Some(true) locked by hand, Some(false) unlocked by hand, None = the numbers decide
async fn set_preset(profile_id: String, preset: Preset) -> Result<(), CoreError>
async fn autopilot_suggestion(request_id: String) -> Result<Option<SuggestionView>, CoreError>
async fn correct_decision(activity_id: i64, should_have: Verdict) -> Result<(), CoreError>  // Approve or Deny
async fn model_status() -> ModelStatus
async fn download_model(progress: Arc<dyn DownloadProgress>) -> Result<ModelStatus, CoreError>
async fn delete_model() -> Result<(), CoreError>
async fn autopilot_evaluate(profile_id: Option<String>, situation: String) -> Result<SuggestionView, CoreError>
```

Foreign traits: `ModelRuntime { load(path: String) -> Result<(), ForeignError>; run(input: ModelInput) -> Result<ModelOutput, ForeignError>; unload() }` (§6.3; called through `spawn_blocking`, 30 s timeout per load/batch, a failure or timeout leaves the request to the user) and `DownloadProgress { progress(downloaded: u64, total: u64) }`.

`Notifier` gains `auto_decided(decision: AutoDecisionView)` and `autopilot_changed(event: AutopilotEvent)`; `PendingItem` gains `suggestion: Option<String>` ("Autopilot would approve · 97%"). `ActivityEntry` gains `decided_by: String` and `autopilot: Option<AutopilotNote>` (§8; rows written before Autopilot read as `""` / `None`).

Enums: `AutopilotMode { Manual, Assisted, Auto, Bypass, Lockdown }`, `Verdict { Approve, Deny, Ask }`, `Preset { Cautious, Balanced, Relaxed }`, `ModelState { NotInstalled, Downloading, Installed, Failed }`, `AutopilotEvent { ModeChanged { connection_id: Option<String> }, BypassEnded { connection_id: Option<String> }, Paused { connection_id, connection_label, reason: String } }`.

Records:
- `AutopilotSettings { mode, base_mode: AutopilotMode, bypass_until: Option<i64>, default_profile_id: String, wifi_only: bool, model: ModelStatus, connections: Vec<ConnectionAutopilot> }`
- `ConnectionAutopilot { connection_id: String, base_mode: Option<AutopilotMode>, bypass_until: Option<i64>, mode: AutopilotMode, profile_id: String }`
- `ModelStatus { state: ModelState, id, label, version: String, size_bytes, downloaded_bytes: u64, error: Option<String>, runtime_ready: bool }`
- `ProfileView { id, name: String, icon: Option<String>, preset: Preset, is_default: bool, memory_count: u32, connections: Vec<String>, classes: Vec<ClassView>, trained_at: Option<i64> }`
- `ClassView { class_key, label: String, decisions, approved, denied: u32, shadow_accuracy: Option<f32>, auto_approve, auto_deny: bool, manual: Option<bool>, decisions_to_unlock: u32 }`
- `SuggestionView { request_id: String, verdict: Verdict, mode: AutopilotMode, p_approve, p_deny, confidence: f32, reason: String, neighbours: Vec<NeighbourView>, profile_id, profile_name, class_key: String, novel, floor, judged: bool }`
- `NeighbourView { label: String, verdict: Verdict, similarity: f32, at: i64 }`
- `AutopilotNote { mode: AutopilotMode, suggested: Verdict, p_approve, p_deny, confidence: f32, profile_id, profile_name: String, neighbours: Vec<String>, reason: String, correctable: bool }`
- `AutoDecisionView { request_id: String, kind: PendingKind, connection_id, connection_label, title: String, verdict: Verdict, decided_by: String, p_approve, confidence: f32, activity_id: Option<i64> }`
- `ModelInput { batch, seq_len, k: u32, input_ids, attention_mask, marker_pos: Vec<i64>, marker_mask: Vec<u8>, qtype: Vec<i64> }`, `ModelOutput { logits, act, pooled: Vec<f32>, hidden: u32 }`

`RewardenCore::new` is unchanged: the runtime is given with `set_model_runtime` (the app may create it after the core). The download base URL (`CoreConfig::models_base`) and the trusted models (`CoreConfig::models`) are overridable only in Rust (tests); the app always gets `DEFAULT_MODELS_BASE` and `KNOWN_MODELS`.

Store: migration 7 (`PRAGMA user_version` 8) adds `autopilot_settings`, `autopilot_profiles` (sealed), `autopilot_memory` (sealed, AAD `autopilot.memory:<profile>:<request>`), `autopilot_suggestions` (sealed), `autopilot_rate` and `autopilot_targets` (keyed hashes of approved targets, for `target is new`).

## 10. Android

- Autopilot screen (Settings → Autopilot, and a mode pill on the home header): big mode selector (Manual / Assisted / Auto / Bypass / Lockdown) with Zeron-style motion and haptics; model card (download with progress, size, version, delete); profiles list → profile detail (classes with progress rings toward unlock, accuracy, lock/unlock, preset, memory count, reset, "Try it" playground); per-connection mode/profile in the connection detail screen.
- Approval screen: suggestion strip ("Autopilot: approve · 97%", neighbours on tap).
- Activity: "Automatic" filter, Autopilot badge, "This was wrong".
- Bypass: persistent notification + red pill with countdown in the header.
- ONNX Runtime Android implementation of `ModelRuntime`; model download as a WorkManager job (Wi-Fi constraint by default).
- Sounds and haptics from the sensory port (AutoApproved, AutoDenied, BypassOn/Off, LockdownOn, mode changes).

## 11. ML tooling (`tools/laya/`, weights never committed)

- `sequence.py` reference sequence builder + `golden.json`.
- `gen_data.py`: synthetic approval situations in the §4 format over the real tool catalog (GitHub, git push, Gmail, Calendar, Telegram, SMS, vault, MCP, desktop `ask` for Bash/Edit/Write from harness hooks, uploads), labels from an explicit rubric, adversarial content (prompt injection in AI-written fields, exfiltration).
- `finetune.py`: LoRA on the encoder + full decision head, on GPU; temperature fit on a held-out split; writes a checkpoint.
- `export_onnx.py`: ONNX with the I/O of §6.2, int8 dynamic quantization, parity check vs PyTorch.
- `eval.py`: accuracy, auto-coverage at θ, unsafe-approve rate (must be 0 on the adversarial split), latency.
