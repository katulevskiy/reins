# Command presentation

The phone core derives `RequestLabel.command` for terminal requests. Android and iOS render the same heading,
actionable source text, setup, and notices. This is a native view field: serde skips it in encrypted history and
synchronization. No migration or paired-client protocol change is needed. Existing stored labels are refreshed from
their retained request when read. Older native clients retain their previous presentation.

`command_presentation::of` does not execute a shell, expand variables, inspect the filesystem, or ask a model what a
command means. Strict shell reading is a prerequisite, followed by a smaller, bounded presentation reader. It only
peels literal leading environment assignments, a plain `env` assignment wrapper, an exact `sh -c`/`-lc`-style wrapper,
and leading `cd path &&`. The remaining command spelling, arguments, additional commands, and operators stay
intact. Quoted or escaped assignment names are not treated as shell assignments. Unrecognized shell paths are not
unwrapped. Redirections, substitutions, comments, unsupported syntax, too many tokens, or oversized input use an
explicit fallback requiring inspection of the original invocation.

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
say "Approved running …" in history instead of claiming that the command ran.

Validation covers literal environment + Gradle, shell and cwd setup, multiple and destructive commands, privileged
wrappers, runtime-injection variables, redirections and substitutions, quoted assignment names, unknown shell paths,
long originals, legacy labels, and the unchanged exact-approval identity. Native Android flow tests verify concealed
values and explicit inspection; screenshot fixtures cover light, dark, and larger text. iOS formatters have matching
tests and Linux compile checks; actual iOS simulator/device rendering requires macOS.


Native Android visual evidence is attached to [PR #176](https://github.com/katulevskiy/reins/pull/176).
