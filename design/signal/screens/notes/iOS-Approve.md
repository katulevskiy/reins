Approve a push
IPHONE · THE FIELD, ITS DRILL-INS AND ITS SHEETS
01 · THE ASK
Claude Code · MacBook
0:40
GITHUB · PUSH
Push 8 commits to octo/app
Autopilot would allow · 97%
Branch
feature/passkeys
New
Account
octo-dana
8 commits
+412 −96
12 files
ALLOW FOR
Once
1 hour
1 day
Custom
Deny
Approve
Field, facts, decision. Nothing else on the screen. The field slides in from behind the island in 320 ms; the ask fades in 60 ms later. Approve asks Face ID, plays confirm and send, and the field folds into a row on Activity. Swipe down dismisses without answering.
02 · DRILL IN · COMMITS
Push 8 commits to octo/app
0:31
8 commits
feature/passkeys · +412 −96 · 12 files
a41c9e2
Add passkey sign-in
Dana Kim · 2 h · +84 −12
7be0311
Store credential ids per device
Claude Code · 2 h · +131 −8
0c95d4a
Update lockfile Hidden
Claude Code · 1 h · +96 −70
e2290bf
Tests for the passkey flow
Claude Code · 48 min · +77
4 more · open on GitHub after approving
Once
1 hour
1 day
Custom
Deny
Approve
Never lose the ask. Drilling in shrinks the field to a band that keeps its colour, its sentence and the timer. The decision bar stays pinned, so you can approve from any depth. Hidden marks invisible characters, in amber with a word, not colour alone.
03 · ONE COMMIT · THE DIFF
Push 8 commits to octo/app
0:24
a41c9e2
Add passkey sign-in
Dana Kim · 2 h · +84 −12
src/auth/passkey.ts
+61 −4
@@ -14,6 +14,18 @@
14
export async function signIn(user) {
15
-  return password(user)
15
+  const key = await passkeys.get(user)
16
+  if (!key) return password(user)
17
+  const ok = await webauthn.verify(key)
18
+  return ok ? session(user) : deny()
19
}
src/auth/session.ts
+23 −8
Once
1 hour
1 day
Custom
Deny
Approve
Readable code on a phone. Mono 11.5 with line numbers, additions and removals as soft washes, a hunk header in blue. Files open and close like an accordion with the chevron turning 180°. Pinch to zoom the diff; long lines scroll on their own.
04 · CUSTOM · REMEMBER FOR
GITHUB · PUSH
Push 8 commits to octo/app
Remember for
1 week
30 days
Custom
3
days
Number of uses
up to 1000
Until I revoke
Never expires
WHAT IT COVERS
Only what's shown
Pushes to feature/passkeys
Anything like this
Push to feature/* in octo/app, by Claude Code
Remember for 1 week
A sheet over the field. The ask stays visible above, dimmed. Steppers tick with the pentatonic detent. Choosing Until I revoke turns the button into a red hold, “Remember until I revoke”, because forever is irreversible enough.
05 · AUTOPILOT'S OPINION
GITHUB · PUSH
Push 8 commits to octo/app
AUTOPILOT · PROFILE WORK
Allow
97%

Claude Code pushes to feature branches of octo/app most days, and you have always said yes.

Approve
97%
Deny
3%
Confidence
92%
LIKE THESE OF YOURS
Push to feature/billing
Approved · 2 days ago
93% alike
Push to feature/onboarding
Approved · last week
90% alike
Autopilot settings
Its reasoning, in your words. The verdict is a big serif word with a number in italic. Neighbours are your own past decisions, not abstractions. Bars grow from zero on open (420 ms, staggered 40 ms). Settings closes everything and lands on Trust.