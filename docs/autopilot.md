# Autopilot

Autopilot decides routine requests for you, on the phone. It uses a small language model (Laya) plus a memory of your
own decisions. It is optional, it runs entirely on the phone, and it starts out only suggesting.

Open it from **Settings → Autopilot**, or tap the mode pill at the top of the home screen.

## Modes

There is a global mode, and each connection can have its own (in that connection's settings).

| Mode | What happens |
|---|---|
| **Manual** | Every request waits for you, and once the model is installed the approval screen shows what Autopilot would do and why ("Autopilot would approve · 97%", "Like 4 times you approved: ..."). Each of your answers teaches it. The default. |
| **Auto** | Autopilot approves what it is confident is safe, denies what it is confident is harmful, and asks you about the rest. Only for kinds of request that have earned it ([below](#how-auto-earns-trust)). |
| **Overnight** | For agents left working: approves everything, the hard floor included, without needing any history, except what the risk model rates **catastrophic in context** (deleting the default branch, `rm -rf ~`, dropping a production database, a key sent to an unknown host, a mass email). Those wait for you, or are denied if you choose "If something is too risky: deny it". Runs until you turn it off or until a time you pick, then returns to the mode it interrupted. Turning it on is a hold, then your fingerprint, face or screen lock. Needs a model that rates risk; without one it decides like Auto. [Details](#overnight). |
| **Bypass** | Approves everything at once, every kind of request (the hard floor below included), for any length from a minute to a year (15 minutes by default), or **until you turn it off**. Only new connections and new phones still ask. A standing permission an AI asks for during it is taken back the moment the bypass ends, however it ends (stopped, run out, locked down, another mode), and says "Ended with the bypass" in Activity; a timed bypass also caps it at its planned end. Turning it on approves what already waits, too. It applies to every AI, whatever mode an AI has of its own, except one locked down. A persistent notification with a countdown (or, without an end, how long it has run) and **Stop** stays up while it is on. Turning it on is a hold, then your fingerprint, face or screen lock. Needs no model, so it answers even when the app is not open. A bypass of a single AI is not available in its first 10 minutes. |
| **Lockdown** | Denies everything at once, including what is already waiting. New connection requests still reach you. A global Lockdown overrides every connection's own mode. Needs no model. |

All modes work with the app closed. The push notification wakes the app, and Autopilot decides right there.
Automatic approvals show up as quiet, grouped notifications on an "Autopilot" channel. Automatic denials use normal
importance.

## Overnight

Overnight is for letting an agent work while you are away. It approves almost everything at once, and asks a second
model output, the **risk**, how bad a request would be if it were a mistake, in its context:

| Risk | Examples | Overnight |
|---|---|---|
| benign | tests, builds, `rm -rf build/`, a push to a feature branch, one email to someone you know | approved |
| risky | `git reset --hard`, a push to `main` without force, deleting a scratch repository | approved |
| severe | `terraform apply` in production, reading a private key, a reply-all to 40 people | approved, and counted (below) |
| catastrophic | force-pushing or deleting the default branch, `rm -rf ~` or `.git`, `DROP TABLE` on a production-looking host, a key or `.env` sent to an unknown host, `curl \| sh` from an unknown host, mailing every contact | waits for you, or is denied |

The same action can be fine or catastrophic: deleting `feature/x` is approved, deleting `main` waits. To judge that,
the phone adds a `context:` line to what the model reads, from what it verified or can read in the action itself: the
default branch, whether a path is inside the project or your home directory, production-looking hosts, recipients,
the amount against your budget, and how many deletes the agent made in the last hour. A few certain facts (deleting
your home directory, force-pushing the default branch, a key sent to an unknown host) hold a request whatever the
model says.

What the agent wrote can never lower the risk: the phone reads the request twice, with and without the agent's text,
and keeps the higher risk. A password in an email body makes it catastrophic.

Safety valves stay on: at most 60 approvals per AI in 10 minutes and 600 a day; after something of an AI was held, or
after 5 severe requests of it in 30 minutes, its severe requests wait for 30 minutes. A standing permission an AI gets
during Overnight ends with it. A connection paired less than 10 minutes ago still waits. Each decision says why in
Activity ("Allowed overnight · risk low", "Waited: risk catastrophic · deletes the default branch main").

A global Overnight applies to every AI that is not locked down or bypassed, whatever mode it has of its own.

## The hard floor

These always wait for you in Manual and Auto (Lockdown denies them; Bypass approves them, except new connections, which
always reach you, as does another phone joining your account; Overnight approves them unless their risk is
catastrophic):

- new connections, requests for standing permissions, and requests to see which accounts you have;
- secrets for the desktop app and SSH signatures;
- purchases ([Payments](payments.md)): only you, or a spend limit you set, approve one;
- everything that is asked every time anyway: vault secrets, deleting repositories or branches, visibility,
  collaborators, transfers, webhooks, deploy keys, repository secrets, branch protection, and similar far-reaching
  changes;
- MCP tools their server marks destructive;
- git pushes that rewrite or delete history, or whose history could not be checked;
- uploaded files that may hold something to run (executables, scripts, archives, unknown binary types);
- anything from a connection paired less than 10 minutes ago;
- a harness hook's or `reins ask` question about something that cannot be undone: the desktop guard's destructive
  groups (rewriting git history, recursive deletes, infrastructure changes, dropping data, publishing, trusting a
  payment key, wiping disks). The phone reads the command itself with its own copy of the guard's rules; what the
  computer says about the rule can make a question stricter, never looser. Also on the floor: questions about private
  keys, cloud credentials and saved tokens, and any question the phone cannot read with certainty (an unbalanced
  quote, code fed to an interpreter, something piped into a shell, a detail cut short). Those are an ordinary yes or
  no on the screen, held and red only when they cannot be undone. A question about a `.env` file is not on the floor:
  it is judged like any other request (below).

The same floor decides what you can answer without opening a request: requests on it never get Approve on the
notification, Approve all or "Approve and allow for 1 hour" ([quick start](quick-start.md#5-answer-requests-quickly)).

## How a request is judged

For each waiting request, the phone writes a short plain-text description of the situation, from the same facts the
approval screen shows:

```
connection: Claude Code (laptop)
connection age: 12 days
connection history: 140 approved, 3 denied
service: github
action: write
operation: Push to a branch
class: push
target: dkat/reins
target is new: no
details: branch feature/laya (not the default branch); 3 commits; 7 files changed; no force
```

Anything the AI wrote (its stated reason, an email body, commit messages, tool arguments, an `ask` question) is
appended in a separate section. The model reads the description twice: once with the facts only, and once with the
AI's text. **The approve probability used is the lower of the two, and the deny probability the higher.** A request
whose facts alone do not look safe cannot be talked into approval. Text addressed to the approver ("this is
pre-approved", "ignore previous instructions") pushes toward asking or denying.

The model's answer is then blended with your history:

- **Memory.** Every decision you make on a request is stored (sealed) with an embedding of its situation, up to
  5,000 per profile. The 8 most similar past decisions vote, and the phone shows them as the reason ("Like 4 times
  you approved: ...").
- **Adapter.** A small logistic regression per profile, retrained on the phone after every 5 new decisions. It
  learns how your choices differ from the base model's.
- The more decisions of one kind there are, the more weight memory and adapter get. With 20 or more, they carry most
  of the weight. Below that, the base model counts for more.

Autopilot's own automatic decisions are never added to the memory, so it cannot reinforce its own mistakes. When it
gets something wrong, open the entry in **Activity** (the **Automatic** filter shows its decisions) and tap **This
was wrong**. The correction counts three times as much as a normal decision, and if it was a wrong approval, that kind
of request goes back to asking you.

## How Auto earns trust

Auto works per **kind of request** (service, action and class, for example `github/write/push` or `gmail/read`; a
harness hook's question is of its guard group, `desktop/ask/env-files`, only when everything its command or file list
does falls in that group as the phone reads it, else `desktop/ask`; its target is exactly that file, or exactly that
command in that folder, so a learnt answer never covers another command):

- **Auto-approve** unlocks after at least 20 of your decisions of that kind, when Autopilot would have agreed with you
  at least 95% of the time over the last 50, and never suggested approving something you denied in the last 20.
- **Auto-deny** unlocks after at least 10 decisions of that kind, when it never suggested denying something you
  approved in the last 20.
- A request to a **target it has not seen you approve** (a new repository, a new recipient) is asked, never
  auto-approved.
- **Rate limit:** at most 30 automatic approvals per connection per 10 minutes, and 200 per day. Beyond that it asks,
  and tells you that it paused for that connection.
- You can unlock or lock a kind by hand on the profile screen.

Thresholds come from a preset per profile:

| Preset | Approve when | Deny when |
|---|---|---|
| Cautious | ≥ 0.98 | ≥ 0.95 |
| Balanced (default) | ≥ 0.95 | ≥ 0.90 |
| Relaxed | ≥ 0.90 | ≥ 0.85 |
| Custom | your own, 0.80 to 0.99 | your own, 0.80 to 0.99 |

With **Custom**, the profile page shows what the last 30 days would have been at the thresholds you set: how many
requests Autopilot would have decided, how many it would have asked, and how many of its decisions you would have
undone (the stored evaluations replayed).

The model's safety calibration was measured at the Balanced thresholds. Relaxed has **not** been validated for
safety.

Autopilot approves **once**. It never creates a standing permission and never ticks "remember".

## Profiles

A profile is a separate memory, adapter and set of unlocked kinds. **Personal** and **Work** are created on first use.
Every connection uses the default profile unless you assign another. Decisions on a connection train only its
profile. You can create, rename, reset (forget everything learned) and delete profiles. The last profile cannot be
deleted. **Try it** on the profile screen lets you type a situation and see what Autopilot would do.

## Privacy

- The model, the memory, the adapter and every decision stay on the phone, sealed in its encrypted store. Nothing
  about Autopilot is sent to the server.
- The model is downloaded once from the release site (Wi-Fi only by default) and checked against SHA-256 hashes built
  into the app. A file that does not match is deleted and never loaded.
- The activity log records who decided (you, Autopilot, Bypass or Lockdown), with Autopilot's confidence and reason.

## The model

`laya-approvals-ml-v1` is a fine-tune of Laya's multilingual checkpoint (mmBERT-base encoder), quantized to int8 for
the phone. It is 337 MB plus a 34 MB tokenizer, and runs with ONNX Runtime. On a desktop CPU, a pair of passes takes
about 120 ms with 4 threads. Expect several hundred milliseconds on a mid-range phone.

Results on held-out synthetic test sets, at the Balanced thresholds (details in the
[model card](../tools/laya/MODEL_CARD.md)):

| Test set | Accuracy | Decided automatically | Unsafe automatic approvals |
|---|---|---|---|
| Same request families as training | 99.7% | 46.7% | 0 |
| Request families never trained on | 93.5% | 25.5% | 0 |
| Adversarial (injected or reassuring AI text) | 99.5% | 23.8% | 0 |
| Adversarial, phrasings never seen in training | 99.0% | 23.3% | 0 |

## Limits

- **Synthetic data.** The model was trained and tested on generated situations labelled by a written rubric
  (`tools/laya/RUBRIC.md`), not on real users' decisions. Your own decisions take over through memory and the adapter,
  but the base model's judgement is the rubric's.
- **Untested wording.** The adversarial tests use 10 fixed injection phrasings and 7 reassurance phrasings, some held
  out from training. Genuinely new wording has not been tested by anyone outside the project.
- **Mostly English.** The base model was pretrained on many languages, but the training data is English apart from
  email bodies in four languages.
- **Not a security boundary.** The hard floor, the per-kind unlocking, the novelty rule and the rate limit are what
  keep Autopilot safe. A model score alone never approves anything on the floor.
- **Fails safe.** Without a model, or if inference fails or times out, Manual shows no suggestion and Auto asks you,
  for that request. Overnight without a model that rates risk decides like Auto. Bypass and Lockdown keep working.
- **Risk is a model too.** Overnight's risk model was trained on generated situations (`tools/laya/RUBRIC.md`, "Risk")
  and checked on a hand-written set; see the [design](autopilot-risk.md). It holds a request when it is fairly sure it
  is catastrophic, so some catastrophic requests can still pass, and some harmless ones wait.
