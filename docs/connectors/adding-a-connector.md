# Adding a connector

A connector lets an AI use one service (Gmail, GitHub, Telegram, ...) through Reins. Each one has two halves:

- a **manifest** in `reins-proto`, which says what the connector is and lists its tools; and
- an implementation of the **`Connector` trait** in `reins-core`, which runs on the phone and talks to the service.

The manifest table is the one list of connectors. The server builds `tools/list` from it without the core, the
phone's Integrations screen lists from it, and the core finds a connector's manifest by its service id. Credentials
never leave the phone. The manifest is plain data, and the implementation reads the account's token from the phone's
secret store when it makes a request.

`crates/reins-core/src/connector/example/` is a complete template: a made-up notes service with a pasted token, one
read, one write and mocked HTTP tests. It is compiled only for tests and is never registered. Copy it to start.

## 1. Describe the connector: the manifest

Add a service id constant next to the others in `crates/reins-proto/src/connector.rs`, and put the connector's tools
in their own module (`crates/reins-proto/src/connector/<service>.rs`, with a `tools()` function, as `gmail.rs` and
`github.rs` do). Then add one `ConnectorManifest` to `MANIFESTS` in `crates/reins-proto/src/connector/manifest.rs`:

| Field | What it is |
| --- | --- |
| `service` | The service id (`"gmail"`). Tool calls, accounts, grants, secrets and the core's registry carry it. |
| `name` | The display name (`"Gmail"`). |
| `auth` | How an account is added: `AuthType::Google`, `PhoneLogin`, `Token`, `Device`, `Vault`, `Payments` or `Builtin`. `AuthType::kind()` is the `kind` the apps' Integrations screen switches on. |
| `scopes` | The access the sign-in asks the service for (OAuth scopes), when it asks for any. |
| `classes` | Its kinds of change, so that a standing permission can name the ones it allows. Leave it empty if there are none. |
| `screen` | The connector's place on the Integrations screen, or `None` if it is not listed there (only `Builtin` ones). |
| `tools` | The function that builds its tools. |

The order of `MANIFESTS` is the order of `tools/list`. Add a new connector at the end, so that clients see the tools
they already knew in the same places, with the new ones after them.

Each tool is a `ToolSpec` built with `tool(...)` in `connector.rs`. It has a name, the service, an `op` (what the
core's `ConnectorCall` carries), an `Effect` (`List`, `Read`, `Search` or `Write`), a title, a description for the
AI, its parameters, and the parameter that names the resource it acts on (a chat, a repository), which standing
permissions are given for. The builders add the rest:

- `.in_class("...")` gives the kind of change, one of the connector's `classes`.
- `.once()` makes the tool ask every time, so that a standing permission never covers it.
- `.scoped(&["..."])` gives the OAuth scopes the tool needs. Each one must be in the connector's `scopes`, and a test
  checks that.

How a tool is approved follows from these facts and nothing else. A read releases only what the user chooses on the
phone. A write is shown on the phone (its `preview`) and runs only once the user approves it, or when a standing
permission covers its resource and class. A `.once()` write is asked for every time. The MCP annotations an AI client
sees follow from the same facts: `destructiveHint` comes from words in the tool's name (delete, send, update, ...)
unless `DESTRUCTIVE_OVERRIDES` says otherwise, so check what your new tools get in the snapshot below.

## 2. Run it: the `Connector` trait

`crates/reins-core/src/connector/mod.rs` defines the trait. A connector implements only what it supports. The
defaults refuse the rest.

| Method | What it does |
| --- | --- |
| `service()` | The service id from the manifest. |
| `fetch(account, call)` | Runs a list, read or search and returns `Item`s. The user chooses which ones are released. |
| `preview(account, call)` | Spells out a write for the approval screen before anything is sent. Show the exact text, the account, the recipient or target, and any media, one `lines` entry each, as Gmail's send does. Fail here, not after approval, when the account is not connected. |
| `perform(account, call)` | Makes the approved write. |
| `sign_in_token`, `identify`, `login_begin` / `login_code` / `login_password` | Adding an account, one of them per `AuthType`: `Token`, `Google` and `PhoneLogin`. |
| `status(account)`, `forget(account)` | Whether an account still works, and disconnecting it (delete its secrets). |
| `party`, `choose_account`, `commit_change`, `fetch_headers`, `unavailable` | Optional extras that some connectors use. |

Register the connector in `Engine::with_connectors` (`crates/reins-core/src/engine.rs`) with `connectors.add(...)`.
`every_connector_has_a_manifest_and_every_manifest_a_connector` in `crates/reins-core/src/services.rs` fails if you
add a manifest without registering a connector, or the other way round.

### Tokens and other secrets

The template shows the pattern every connector with a credential follows:

- `sign_in_token` checks the token with the service first, then keeps it with
  `store.secret_put(SERVICE, &account, ...)` under the account the service says it belongs to. The connector struct
  holds no credential.
- Each request reads the token back with `store.secret_get(SERVICE, account)` into a `Zeroizing` buffer and sends it
  in a header, never in a URL.
- Errors are built from the HTTP status. Never put the token or the service's reply body in an error or a log line,
  because a reply can echo what was sent.
- `forget` calls `store.secret_delete(SERVICE, account)`.

## 3. Test it

- Mock every HTTP request with `wiremock`. Tests never call a live service.
- Cover each tool, its errors and its approval text, and check that requests carry the kept token.
- Make sure that no token appears in errors (the template's `a_rejected_token_is_not_kept_and_never_shown`) or logs
  (`crates/reins-core/tests/no_secrets_in_logs.rs`).
- Two snapshots pin the tool list: `crates/reins-proto/src/connector/testdata/tool_specs.txt` (every connector's
  name, classes and scopes, and every tool with its effect, class, annotations and scopes) and
  `src/api/reins/testdata/tools_list.txt` (the names an AI client sees, in order). Update them with
  `REINS_UPDATE_SNAPSHOTS=1 cargo test -p reins-proto snapshot` and
  `REINS_UPDATE_SNAPSHOTS=1 cargo test --features sqlite the_listed_tools_match_the_snapshot`, then review the diff.
  It should show your new connector and its tools and nothing else.

## 4. The apps

A connector with an existing `AuthType` needs no new screen. A new `AuthType` needs both apps' Integrations screen to
handle its `kind` (Android `ui/services/ServiceScreen.kt`, iOS `Screens/Integrations/ServiceScreen.swift`). Both apps
also keep some per-service text and marks, such as where to create a token and the service's colour (Android
`design/Avatars.kt` and `design/Tags.kt`, iOS `Screens/Integrations/IntegrationsLogic.swift`). Add yours there.

## 5. Document it

Add `docs/connectors/<service>.md`. Explain how to set the connector up, what each tool does, what the service
requires (plans, scopes, limits) and where those requirements are documented.
