# Adding a connector

A connector lets an AI use one service (Gmail, GitHub, Telegram, ...) through Reins. Each one is a plugin with two
parts:

- a **manifest** in `reins-proto`, which says what the connector is and lists its actions; and
- an implementation of the **`Connector` trait** in `reins-core`, which runs on the phone and talks to the service.

Credentials never leave the phone: the manifest is plain data, and the implementation reads the account's token
from the phone's store when it makes a request. Every action goes through the same approval flow as before, whatever
its risk. Reads release only what the user shares, and writes are shown on the phone and approved there.

`crates/reins-core/src/connector/example/` is a complete template: a made-up notes service with one read, one write
and mocked HTTP tests. It is compiled only for tests and is never registered. Copy it to start.

## 1. Describe the connector: the manifest

`crates/reins-proto/src/connector/manifest.rs` has one `ConnectorManifest` per connector in `MANIFESTS`:

| Field | What it is |
| --- | --- |
| `service` | The service id (`"gmail"`). Tool calls carry it, and the core's registry is keyed by it. Add a constant next to the others in `connector.rs`. |
| `name` | The display name (`"Gmail"`). |
| `auth` | How an account is added: `AuthType::Google`, `PhoneLogin`, `Token`, `Device`, `Vault`, `Payments` or `Builtin`. `AuthType::kind()` is the `kind` the apps' Integrations screen switches on. |
| `scopes` | The access the connector asks the service for (OAuth scopes), when it has any. |
| `classes` | Its kinds of change, so that a standing permission can name the ones it allows. Leave it empty if there are none. |
| `screen` | The connector's place on the Integrations screen, or `None` if it is not listed there. |
| `tools` | The function that builds its tools. |

The order of `MANIFESTS` is the order of `tools/list`. Add a new connector at the end so that existing clients see
the same list with the new tools after it.

Each tool is a `ToolSpec` built with `tool(...)` in `connector.rs`. It has a name, an `op`, an `Effect` (`List`,
`Read`, `Search` or `Write`), a title, a description for the AI and its parameters. The builders add the rest:

- `.in_class("...")` gives the kind of change.
- `.once()` makes the action ask every time, so that a standing permission never covers it.
- `.scoped(&["..."])` gives the scopes the action needs.
- `.risk(RiskLevel::...)` names the risk when the derived one is wrong.

`manifest::actions()` returns every action as an `ActionManifest` with `tool`, `service`, `op`, `effect`, `risk`,
`scopes`, `class`, `once_only` and `desktop_only`. If an action names no risk, it is derived as follows:

| Risk | When |
| --- | --- |
| `Low` | Lists, reads and searches. |
| `Medium` | Other writes. |
| `High` | Writes that delete, send or publish (MCP's `destructiveHint`). |
| `Critical` | `once_only` writes. |

The risk only describes the action. It never makes an approval stricter or looser.

## 2. Run it: the `Connector` trait

`crates/reins-core/src/connector/mod.rs` defines the trait. A connector implements only what it supports. The
defaults refuse the rest.

| Method | What it does |
| --- | --- |
| `service()` | The service id from the manifest. |
| `manifest()` | Its manifest. The default looks it up by `service()`. |
| `fetch(account, call)` | Runs a list, read or search and returns `Item`s. The user chooses which ones are released. |
| `preview(account, call)` | Spells out a write for the approval screen. Show the exact text, the account, the recipient or target, and any media, one `lines` entry each, as Gmail's send does. |
| `perform(account, call)` | Makes the approved write. |
| `status`, `identify`, `sign_in_token`, `login_*`, `forget` | Account set-up and state. |
| `party`, `choose_account`, `commit_change`, `fetch_headers` | Optional extras that some connectors use. |

Register the connector in `Engine::new` (`crates/reins-core/src/engine.rs`) with `connectors.add(...)`.
`Registry::manifests()` lists the connectors that a phone runs, and `Registry::missing()` lists the ones it does not.

## 3. Test it

- Mock every HTTP request with `wiremock`. Tests never call a live service.
- Cover each action group, its errors and its approval text.
- Make sure that no token appears in logs or errors. `crates/reins-core/tests/no_secrets_in_logs.rs` shows how.
- `crates/reins-proto/src/connector/testdata/tool_specs.txt` and `src/api/reins/testdata/tools_list.txt` pin the
  tool list. Run `REINS_UPDATE_SNAPSHOTS=1 cargo test -p reins-proto snapshot` and
  `REINS_UPDATE_SNAPSHOTS=1 cargo test --features sqlite the_listed_tools_match_the_snapshot` to update them.
  Then review the diff: it should show your new tools and nothing else.

## 4. Document it

Add `docs/connectors/<service>.md`. Explain how to set the connector up, what each action does, what the service
requires (plans, scopes, limits) and where those requirements are documented.
