# Command presentation

The phone core derives `RequestLabel.command` for terminal requests. Android and iOS render the same heading,
actionable source text, setup, and notices. This is a native view field: serde skips it in encrypted history and
synchronization. No migration or paired-client protocol change is needed. Existing stored labels are refreshed from
their retained request when read. Older native clients retain their previous presentation.

`command_presentation::of` does not execute a shell, expand variables, inspect the filesystem, or ask a model what a
command means. It uses a bounded display lexer independently of the strict safety reader. A known observed
operation can be summarized even when semantic certainty is unavailable; notices describe shell expressions,
compound actions, and other limits. Literal leading environment, an exact shell wrapper, and `cd path &&` become
setup. Known build options/properties move into counted technical configuration with credential values concealed.
Unknown/effectful flags, every additional command/operator, privilege wrapper, and input/output redirection stay in
the actionable text. Redirect overwrite/append targets and descriptor joins are also prominent notices. Quoted
assignment names remain executable names. Unicode formatting controls become visible replacements, while the
retained original stays exact. Oversized/token-heavy input still requires inspecting the original.

Known Gradle Android tasks have purpose-oriented headings such as “Build Android release and test APKs.” Unfamiliar
tasks remain named explicitly; Maven, Cargo, package scripts, Make/CMake, file operations, Git and network tools get
observed operation/target headings. These are descriptions of a request, never proof of a successful build or safety.

Environment values are concealed in setup. Runtime-affecting variables such as `PATH`, `JAVA_TOOL_OPTIONS`, and
loader configuration have visible notices. Privilege-changing wrappers remain in the command and have a notice.
Recognizing Gradle does not establish safety: its notice explains that project scripts and plugins run with the
process's available permissions and credentials. The existing risk, certainty, mandatory-review, and Auto logic
continues to evaluate the original request.

The complete pending invocation remains available on an explicit inspection page, including text beyond the legacy
8,000-character `AskView.detail` limit. Its raw field is excluded from serde and concealed in Debug. This is not an
authorization scope. One-time approvals and standing command identity remain bound to the original command,
environment, working directory, and computer.

History conceals terminal invocation text until explicitly revealed, retaining its existing copy behavior. Existing
history stores at most 8 KiB per request field. Missing originals and retained text ending in an ellipsis have visible
limitations; this change cannot recover text that was never retained. The audit and sync storage budget is unchanged.

A successful desktop ask records permission rather than an execution report. New terminal presentations therefore
say “Approved: <core heading>” in history instead of claiming that the command ran. Native views lead with the
operation, complete readable actions and notices. Literal setup/configuration sits in collapsed Technical details;
the exact original has a separate explicit reveal with a credentials warning.

Validation covers literal environment + Gradle, shell and cwd setup, multiple and destructive commands, privileged
wrappers, runtime-injection variables, redirections and substitutions, quoted assignment names, unknown shell paths,
long originals, legacy labels, and the unchanged exact-approval identity. Native Android flow tests verify concealed
values and explicit inspection; screenshot fixtures cover light, dark, and larger text. iOS formatters have matching
tests and Linux compile checks; actual iOS simulator/device rendering requires macOS.


Native Android visual evidence is attached to [PR #176](https://github.com/katulevskiy/reins/pull/176).

## Reported Gradle case and guard provenance

The regression fixture includes long SDK environment setup, release/test APK tasks, signing properties, and
`> /tmp/log 2>&1`. The old display reader rejected every redirect even though the independent strict reader already
supports these literal redirects. Its all-or-nothing fallback hid the useful tasks behind generic shell wording.
The new actionable view contains tasks and redirects; configuration remains inspectable separately, with passwords
concealed. Pending requests and old Activity labels are both rebuilt from retained original source. Missing history
or an ellipsis remains explicitly incomplete; a missing tail is never invented.

The `.keystore` option reference matches the existing protected-key group and still requires review every time.
A rule match proves a reference, not a file read, key exfiltration, successful signature, or harmlessness. The core
uses “References signing configuration or a signing key” for that observed case, including old generic Activity
reason text when retained source supports it. Actual file-read wording is unchanged.

Tests also pin the separate uncertainty provenance: the exact command and risk context are readable, but a reported
`file:*.keystore` topic is also read as strict shell text and its wildcard program makes that overall guard reading
uncertain. Raw and clamped duplicate questions agree. This change preserves that reading and every existing guard,
risk, no-standing and automatic-approval floor; it does not reinterpret topic authority or auto-trust build scripts.

Dynamic assignments and build properties are not folded into technical setup. Executable `$()`/backtick source
remains visible in the action, even when a sensitive assignment's literal value is concealed. Single-quoted
substitutions remain literal configuration. URL userinfo is concealed in headings, action text and redirect
notices. These are display-only distinctions; original source and all guard floors remain unchanged.
