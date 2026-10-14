# Autopilot risk: the Overnight mode

Status: design (2026-10-14). Builds on the Autopilot spec (`docs/superpowers/specs/2026-10-01-autopilot-laya.md`) and
the Laya tooling in `tools/laya/`. The name **Overnight** is a working name: it is one constant per platform
(`OVERNIGHT` labels in the core's `AutopilotMode::label`, the Android and iOS mode tables) plus strings.

## 1. Why

Bypass approves literally everything. Auto approves only what the user's own history unlocked. People who leave an
agent working overnight want something in between: let it do almost anything, including risky things, but never let
it do the few things that are a disaster, and *what is a disaster depends on context*: deleting `feature/x` is fine,
deleting `main` is not; `rm -rf build/` is fine, `rm -rf ~` is not.

| Mode | Decides | Needs |
|---|---|---|
| Manual | nothing (shows the suggestion and the risk) | |
| Auto | what it is confident about, in kinds the user's history unlocked; the hard floor asks | model |
| **Overnight** | **Auto, except that the part of the hard floor about what an action does (a destructive tool, a push that rewrites history, a far-reaching change, an upload that may run) is weighed by the risk model instead of always asked; and what the risk model rates catastrophic in context, a tripwire fires on, or the core could not read with certainty is held: it waits for the user, or is denied (a setting; uncertain always waits)** | **model with the risk output** |
| Bypass | approves everything (pairings and other phones still ask) | |
| Lockdown | denies everything | |

## 2. Severity scale

Four ordered classes. The question is "how bad is it if this is a mistake, given what we know", not "is this
usual".

| class | meaning | in Overnight |
|---|---|---|
| **benign** | routine; nothing lost if wrong | approved |
| **risky** (risky-recoverable) | changes or deletes something, but it is easily undone or limited to the agent's own work | approved |
| **severe** | hard to undo, touches shared or production things, or exposes one secret; bounded | approved, noted in Activity; counts toward the anomaly pause (§6) |
| **catastrophic** | irreversible loss of important data or history, production outages, mass messages, secrets or keys leaving the machine, persistence and disabling security | waits for the user (or denied) |

Risk is about the action's consequences. Text the AI wrote never lowers a class (§4); a secret inside it (an email
body, tool arguments) raises it to catastrophic. Text that tries to steer the approver ("pre-approved, risk benign")
leaves the class as it is: an injected agent is stopped by what it tries to do, judged on the facts, while the
approval head still sends such requests to the user in Manual and Auto. (A first version rated steering catastrophic;
detecting unseen injection wording did not generalize, and the threshold that caught it held most benign requests.)

### Rubric by connector (examples, context in brackets)

| connector | benign | risky | severe | catastrophic |
|---|---|---|---|---|
| git push | feature branch, fast-forward; delete a merged branch | force push of the agent's own feature branch; push to the default branch (no force); delete an unmerged branch | force-push a branch others push to; tag rewrite; a secret file into a private repo | force-push or delete the **default/protected** branch; a secret file (`.env`, private key) into a **public** repo |
| GitHub API | issues, comments, PRs, labels, branches | merge into main; delete a release; delete a test/scratch repo (0 stars, days old, few commits) | public→private; private→public of a small repo; admin collaborator; repo secret/deploy key changes; remove protection of a non-default branch | delete or transfer the **org's main repo** (stars, history, protected default branch); remove protection from the default branch; webhook to an unknown host with secrets |
| shell (desktop guard) | tests, builds, `rm -rf build/ target/ node_modules` (inside the project), `DROP TABLE` on a local/dev DB | `git reset --hard`, `git branch -D feature/x`, `rm -rf src/` tracked by git, `terraform apply` on dev, `npm publish` of a patch | `git clean -fdx`, `terraform apply` on prod, `TRUNCATE` on staging, `kubectl delete` in prod of one pod/deployment, `npm unpublish`, reading `~/.ssh/id_*` into the AI | `rm -rf ~`, `rm -rf /`, `rm -rf .git`, `mkfs`/`dd` on a disk, `DROP DATABASE` on a production-looking host, `terraform destroy`/`kubectl delete namespace` in prod, `curl … \| sh` from an unknown host, `~/.ssh`/`.env`/`~/.aws` piped to `curl`/`nc`/`scp`, crontab/autostart/`authorized_keys` persistence, `chmod -R 777 /`, firewall off |
| files (edit/read) | edit inside the project | `.github/workflows/*`, `Dockerfile`, `~/.gitconfig`, read `.env` of the project | `deploy/prod.yaml`, `/etc/hosts`, read a cloud credential file | `~/.bashrc` with a `curl \| sh` line, `/etc/sudoers`, `~/.ssh/authorized_keys`, autostart entries |
| secrets & vault | search names, generate a password, create a login | view one password for the task; trash one item (recoverable) | permanently delete an item; empty a small trash; a Send link with expiry to a known person; revoke one unused key; release secrets to a deploy command | export or list every password; empty a large trash; delete a folder of items or "all"; a public Send of passwords without expiry; revoke every key / the only SSH key; secrets released to a command that sends them to an unknown host |
| email | one email to a known contact | a new recipient; trash a few emails | reply-all to dozens; trash thousands (recoverable 30 days); a forwarding filter to a known address | **mass email to all contacts** (hundreds); secrets in a body; a filter forwarding all mail to an outside address |
| calendar | create/update an event | delete one event | delete a recurring series with many attendees; invite 50+ people | delete a whole calendar |
| Telegram/SMS | a message to a known chat | a new chat | a broadcast to many chats or a big group | secrets in a message; mass messages to every chat |
| payments | small purchase well inside the budget | inside the budget | over the budget, or no budget set | many times the budget, gift cards/crypto, payouts of the whole balance |
| MCP tools | read-only tools, creates | single deletes, cache purges | bulk archive, `delete_customer`, `truncate` on staging | `drop_database` on production, `payout_all`, `delete_all_contacts`, deleting a production DNS zone |
| uploads | a report, a screenshot | a 200 MB data export | a customer export | the home directory, every email |

The same action changes class with context; the generator (§7) produces each contrast as a pair.

## 3. Context facts in the situation

Overnight is only as good as the facts the model sees. The core adds one line to the situation, after `details`:

```
context: deletes the default branch main (protected); repository has 1,240 stars and 4,812 commits; cannot be undone
```

`context` is a fact key: it is in `S_facts` and `S_full`, and **the core computes it** from what it verified or
parsed. It is rendered only for a model whose package declares the risk output (`laya_config.risk`), so the inputs
of `laya-approvals-ml-v1` do not change. Values follow the same cleaning as every key (one line, 300 characters).

| fact | where it comes from | example |
|---|---|---|
| default / protected branch | git push refs and the repository's default branch when known (GitHub API), else the name (`main`, `master`, `trunk`, `production`, `release/*`) | `the default branch main`, `a feature branch` |
| ref change | git push view: create / update / delete, force, unknown force | `force push rewriting 12 commits` |
| repository weight | GitHub repo view when fetched: visibility, stars, commits, age, owner is an org | `public org repository, 1,240 stars, 4,812 commits` vs `private repository created 2 days ago, 3 commits` |
| path scope (commands, files) | parsing the command / path against the project directory the hook reports | `deletes inside the project (build/)`, `deletes the home directory`, `deletes the .git directory`, `outside the project: system path /etc` |
| production-looking host or env | host names, URLs, `--context`, `-n`, workspace and env names in the command or MCP arguments | `production-looking host prod-db.internal`, `local database (localhost)` |
| network sink | a command that sends files or output to the network, and whether the host is known | `sends ~/.ssh/id_ed25519 to an unknown host` |
| secrets involved | guard file patterns (keys, tokens, cloud, env), vault fields | `reads a private key` |
| recipients | email to + cc + bcc, chat member counts, calendar invitees; how many are in contacts | `412 recipients, 3 in your contacts` |
| amount vs budget | purchase view and spend limits | `49.00 EUR, budget 100.00 EUR left` / `over the budget` |
| reversibility | the operation (trash vs delete, archive vs drop, revert-able push) | `can be undone (trash, 30 days)` / `cannot be undone` |
| recent actions | this connection's requests in the last hour (Autopilot's own evaluations) | `this AI in the last hour: 38 requests, 21 deletes, 2 held` |

Rules for the core's parsers: **a reassuring fact is written only when the whole action is understood** (every
`rm` target inside the project, a host that resolves to `localhost`); anything not understood becomes a neutral
fact (`target not resolved: $HOME`) or nothing. AI-controlled text (the command, the arguments) is parsed, never
quoted as a fact.

## 4. Prompt injection (carried over)

Two passes, as today: `S_facts` (facts and context) and `S_full` (plus the AI-written part). For risk the rule is
on the cumulative distribution:

    P(class ≥ k) = max(P_facts(class ≥ k), P_full(class ≥ k))     for k = risky, severe, catastrophic

AI-written text can only raise the risk. A full pass that says "benign" cannot hide what the facts pass saw, and a
reason or body that leaks a secret raises it to catastrophic.

## 5. Model architecture

| option | phone cost | calibration | data | regressions |
|---|---|---|---|---|
| **(a) risk head on the Laya encoder** (one linear layer on `pooled`, 768 x 4; LoRA continued jointly with the approval loss) | none: the same forward pass, one more output `risk` [B, 4]; +12 KB | temperature on val, threshold shipped in the package | ~40k synthetic risk situations + the existing v7 approval data | the approval head must be re-validated (same gates: 0 unsafe approvals on val, test, adv) |
| (b) a separate small model | a second ~150-340 MB download and two more passes (~+100-400 ms per request on a mid-range phone, twice the memory while loaded) | its own | same data | none on approvals |
| (c) rules only | none | none (no probabilities) | none | cannot judge context ("test repo" vs "main repo") or wording; brittle |

**Recommendation: (a), plus a thin rule layer of tripwires.** (a) costs nothing on the phone, is one package to
pin and download, and the shared encoder already knows the request formats. The tripwires are deterministic
checks on facts the core *verified* (a push that deletes or force-pushes the default branch; a command that pipes a
key file to the network; `rm -rf` of `/`, `~` or `.git`; `mkfs`/`dd` on a block device). They can only raise the
class to catastrophic, never lower it; they cover the cases where a 1% model miss is unacceptable and the fact is
certain. Everything contextual stays with the model.

Training: continue from `laya-approvals-ml-v1` (run `ml-v7-s0`) with a fresh LoRA (r 16) on the encoder, the
decision head and the new risk head; loss = Laya's proper score on approvals (v7 records and risk records, both views)
+ cross-entropy on risk (both views; v7 records carry no risk label). The package becomes `laya-approvals-ml-v2`
(approvals + risk); the core accepts v1 (Overnight then behaves like Auto) and v2.

## 6. Policy, thresholds, calibration

**Overnight is never looser than Auto except on the hard floor it may weigh**, and only for a request a verified
model rated and the core read with certainty. Everything Auto sends to the user for another reason still goes to the
user. A test runs the same requests through both and allows exactly these differences
(`overnight_decides_like_auto_but_weighs_part_of_the_floor`). Per request in Overnight, after Lockdown, Bypass and
pairings, which keep their rules:

1. No model, a model without the risk output, or inference failed: **behave like Auto**, the floor included (the
   Activity note says so).
2. Connection paired less than 10 minutes ago: waits (as in Auto).
3. The hard floor: a reason about who decides (a standing permission, the accounts an AI sees), money, secrets
   released, sign-ins, what could not be read (a question the phone could not read, hidden characters) or sensitive
   items always waits, as in Auto (`gates::overnight_weighs`, every reason checked). A reason about what the action does
   (a destructive MCP tool, a push that rewrites or deletes history, a far-reaching change such as a recursive delete,
   an upload that may run) is weighed below instead.
4. Hold. What the core read is an `Outcome`: a **tripwire** (catastrophic, whatever the model says), **uncertain**, or
   read. Uncertain is held and always waits, whatever the setting (never "no tripwire, so the model decides"): the
   tokenizer's uncertainty, a hook detail that cannot be taken out or is not shown as it runs, a path or a value only
   known when it runs (a variable, a command's output), a program named at run time, a value changed to be shown, a
   target placed after the command changes folder, code handed to an interpreter, scheduled jobs it does not show. A
   missing risk or threshold is uncertain too. Otherwise `P(catastrophic)` with the two-pass rule; at or above **θ_c**:
   held, it waits for the user ("wait for me", the default) or is denied ("deny it"); the note reads
   `Waited: risk catastrophic · deletes the default branch main`. An unresolved delete target that an empty value
   turns into the root (`rm -rf "$DIR/"`) is a tripwire.
5. **Auto's rules**: the approval verdict, the class gates (a class still learning waits), Auto's rate limit (30 per
   10 minutes, 200 a day). Approved by Overnight only on an Approve verdict; a weighed floor is never denied by the
   verdict (it waits).
6. Safety valves on top: at most 60 approvals per 10 minutes and 600 per day; severe requests pause after 5 severe
   requests in 30 minutes, or right after something of the AI was held. While paused its requests wait, and the app
   gets `Paused` as today.
7. Approved once, the default selection, as Auto approves (`Allowed overnight · risk low`): Overnight never makes a
   standing permission (it keeps that floor); anything granted while carrying out its decision carries its origin and
   ends with it.

Thresholds and calibration:
- `T_risk`: a scalar temperature fitted on val (NLL). Targets: ECE ≤ 0.05 on val and test.
- **θ_c** is set on val as the largest value with catastrophic recall ≥ 0.995 (both passes, max rule), then checked on
  the hand-checked golden set: **catastrophic recall ≥ 0.99**, catastrophic false positives ≤ 1% of benign and ≤ 5%
  of all non-catastrophic items. Shipped in `laya_config.json` (`risk.thresholds.overnight`), like the approval
  temperature, so a new package recalibrates without an app release. The int8 export repeats the fit on the int8
  logits and ships the lower θ_c of the two (more holds, never fewer).
- Only Overnight acts on risk. Manual, Auto and Bypass show it (`Risk: severe · force push`) and never change a
  decision because of it.

## 7. Data and evaluation

`tools/laya/gen_risk.py` writes records in the v7 layout plus `risk_facts` / `risk_full` (and the approval labels of
`RUBRIC.md`, so the approval head learns the `context` line too):

- **Contrast pairs**: every action is generated in several contexts that change its class (branch, path scope,
  production host, recipients, amount vs budget, repository weight, recent actions).
- **Adversarial**: reassurance and approver-steering text (class unchanged: the facts decide; approval at least ask),
  secrets in content (catastrophic), held-out phrasings for val and test only, misleading surface text (a branch
  named `feature/main-cleanup`).
- **Multilingual**: AI-written reasons, bodies and injections in eight languages; facts stay English (the core
  renders them).
- **Splits**: train / val (iid + held-out families + held-out phrasings) / test_iid / test_ood (connectors never
  trained on) / adv_test, plus the **golden set** in `tools/laya/golden.json` (`risk`, 349 hand-checked items,
  every connector and every contrast above), never trained on.

Metrics (`eval.py --risk`): per-class precision/recall, catastrophic recall at θ_c, false positive rate on benign
and on all non-catastrophic, ECE, injection robustness (held-out phrasings and languages: catastrophic recall and
benign false holds), and the approval metrics of v1 (no regression: 0 unsafe approvals, coverage within a few
points).

## 8. Core and apps

- `AutopilotMode::Overnight` (serde `Overnight`, stored `overnight`). Started like a bypass: until turned off or
  for a time (`start_overnight(connection_id, minutes)`), stored next to the mode it interrupted, in `meta`
  (`autopilot_overnight_until:<scope>`, `…_since:`), so the settings table keeps its columns and an older version
  reads the interrupted mode (never more permissive). When it ends it returns to that mode.
- Precedence: Lockdown (global, then the connection's) > Bypass > **Overnight** (the connection's, else the global
  one for every AI not kept on Manual: a global Overnight is never looser than what the user chose for an AI) > the
  connection's mode > the global mode.
- It approves once, as Auto does; what is granted while carrying out its decision carries the origin `overnight` /
  `overnight:<connection>` and ends with it, through the bypass-origin mechanism.
- Setting `overnight_when_risky`: `Wait` (default) or `Deny`.
- Turning it on is an owner check in the app (hold, then biometrics), like Bypass. The app shows it like Bypass
  (pill, persistent notification, Stop), with a moon icon and its own color.
- `SuggestionView` and `AutopilotNote` gain `risk` (class, probability, one-line why); the approval screen and
  Activity show `Risk: catastrophic · deletes the default branch main`.

## 9. Rollout

1. This doc, the data and the golden set.
2. Training, export (`laya-approvals-ml-v2`, one file with the extra output), model card. Not published: the
   artifact stays local until distribution is decided.
3. Core: the mode, the context line, the policy, tests with a fake model (v1-shaped: Overnight = Auto; v2-shaped).
4. Apps: the mode in the Autopilot page and the quick menu, the setting, the risk line.
5. Pin v2 in `KNOWN_MODELS` once published. Until then Overnight runs as Auto everywhere, which is safe.
