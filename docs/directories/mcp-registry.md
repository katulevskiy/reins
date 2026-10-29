# MCP Registry: submission package

What it takes to list the hosted Reins server in the official [MCP Registry](https://registry.modelcontextprotocol.io),
and what is still open. Nothing here has been published. The entry is [`server.json`](server.json) in this folder.

Sources, checked on 2026-10-29 (registry repository at commit `9c4a96c`, 2026-10-27):

- [Quickstart: publish a server](https://github.com/modelcontextprotocol/registry/blob/main/docs/modelcontextprotocol-io/quickstart.mdx),
  section "Remote-only servers"
- [Publishing remote servers](https://github.com/modelcontextprotocol/registry/blob/main/docs/modelcontextprotocol-io/remote-servers.mdx)
- [Authentication](https://github.com/modelcontextprotocol/registry/blob/main/docs/modelcontextprotocol-io/authentication.mdx)
- [Versioning](https://github.com/modelcontextprotocol/registry/blob/main/docs/modelcontextprotocol-io/versioning.mdx)
- [Official registry requirements](https://github.com/modelcontextprotocol/registry/blob/main/docs/reference/server-json/official-registry-requirements.md)
  and the [`server.json` schema, 2025-12-11](https://static.modelcontextprotocol.io/schemas/2025-12-11/server.schema.json)

The registry is in preview: breaking changes or data resets may happen before general availability. It hosts metadata
only. Clients and other directories read it (see "registry aggregators" in the registry docs), so one entry can feed
several catalogs.

## The entry

`server.json` describes a remote-only server: a `remotes` entry and no `packages` (there is no npm, PyPI or OCI
package of the hosted server). It validates against the 2025-12-11 schema.

| Field | Value | Why |
| --- | --- | --- |
| `$schema` | `https://static.modelcontextprotocol.io/schemas/2025-12-11/server.schema.json` | The current schema; `mcp-publisher publish` rejects older ones. |
| `name` | `com.reins2fa/reins` | Reverse-DNS of `reins2fa.com`, so the namespace is proved with DNS or HTTP (below). Pattern `^[a-zA-Z0-9.-]+/[a-zA-Z0-9._-]+$`, at most 200 characters. Permanent once published. |
| `title` | `Reins` | At most 100 characters. |
| `description` | "Let AI use your Gmail, GitHub, calendar and more, with every action approved on your phone." | 91 of the 100 characters allowed. |
| `version` | `1.0.0` | The workspace version in `Cargo.toml`. Each publication needs a new, unique version; ranges such as `^1.0` are refused. |
| `websiteUrl` | `https://reins2fa.com/developers` | Public. |
| `repository` | `https://github.com/katulevskiy/reins`, `source: github` | |
| `icons` | `https://raw.githubusercontent.com/katulevskiy/reins/main/docs/assets/logo.png`, PNG, 256 x 256 | `src` must be an HTTPS URL of at most 255 characters. |
| `remotes[0]` | `type: streamable-http`, `url: https://app.reins2fa.com/mcp` | No `headers` and no `variables`: OAuth is discovered from the 401 (see below). |

What the remote does, for the reviewer of any catalog that reads this entry:

- Streamable HTTP. Requests go by POST. `GET /mcp` with `Accept: text/event-stream` and a token opens a stream that
  carries only ticket updates; without them it answers 405. There are no sessions, so `DELETE /mcp` answers 405
  (`src/api/reins/mcp_routes.rs`).
- Protocol versions: `2026-07-28` (version in `_meta`) and `2025-11-25`, `2025-06-18`, `2025-03-26` through
  `initialize` (`src/api/reins/mcp.rs`).
- OAuth 2.1: a 401 with `WWW-Authenticate: Bearer resource_metadata="https://app.reins2fa.com/.well-known/oauth-protected-resource/mcp"`;
  authorization server metadata at `/.well-known/oauth-authorization-server` with PKCE `S256`, dynamic client
  registration, client ID metadata documents (CIMD), token endpoint auth `none` and `private_key_jwt`, and the device
  flow. Checked live on 2026-10-29.

## Namespace verification

The registry checks that the publisher controls the namespace in `name`. It does not check the remote URL. A remote
URL can belong to only one server name, so the first entry that names `https://app.reins2fa.com/mcp` keeps it.

| Method | Name it allows | What Reins needs |
| --- | --- | --- |
| **DNS (recommended)** | `com.reins2fa/*` | A TXT record on the apex `reins2fa.com` (not on a subdomain or selector): `v=MCPv1; k=ed25519; p=<base64 public key>`. Remove an old record when rotating keys, or verification fails. |
| HTTP | `com.reins2fa/*` | The same text served at `https://reins2fa.com/.well-known/mcp-registry-auth`. It answers 404 today. The site is not this repository, and the Reins server serves `app.reins2fa.com`, not the apex. |
| GitHub | `io.github.katulevskiy/*` | Works with `mcp-publisher login github` for a personal namespace. An organization namespace needs an org Owner and GitHub Actions OIDC, or a PAT with `read:org`. The device flow cannot publish to organizations (registry issue #1468). |

The key pair (Ed25519; OpenSSL 3, not macOS LibreSSL):

```sh
openssl genpkey -algorithm Ed25519 -out key.pem
PUBLIC_KEY="$(openssl pkey -in key.pem -pubout -outform DER | tail -c 32 | base64)"
echo "reins2fa.com. IN TXT \"v=MCPv1; k=ed25519; p=${PUBLIC_KEY}\""
```

Keep `key.pem` in Infisical, never in the repository.

## Publish steps (not done)

1. **Not done.** Decide the name and the auth method (open questions 1 and 2).
2. **Not done.** Make the key pair and add the TXT record to `reins2fa.com`. Wait for it to propagate.
3. **Not done.** Install `mcp-publisher` (`brew install mcp-publisher`, or the release binary from the registry's
   GitHub releases).
4. **Not done.** Validate: `mcp-publisher validate docs/directories/server.json`.
5. **Not done.** Log in:
   `mcp-publisher login dns --domain reins2fa.com --private-key "$(openssl pkey -in key.pem -noout -text | grep -A3 'priv:' | tail -n +2 | tr -d ' :\n')"`.
6. **Not done.** Publish: `mcp-publisher publish docs/directories/server.json`.
7. **Not done.** Check: `curl "https://registry.modelcontextprotocol.io/v0.1/servers?search=com.reins2fa/reins"`.
8. On each release that changes what the listing says, raise `version` and publish again. A published version and its
   metadata can't be changed.

A GitHub Actions job can do steps 5 and 6 on release (`github-actions.mdx` in the registry docs). Add it only after
the first manual publish works.

## Open questions for Daniel

1. **Name.** `com.reins2fa/reins` (DNS or HTTP on `reins2fa.com`) or `io.github.katulevskiy/reins` (GitHub, no DNS
   change)? The name is permanent, and the remote URL can be claimed only once, so choose before anyone else lists it.
2. **DNS or HTTP.** DNS needs access to the `reins2fa.com` zone. HTTP needs the token at
   `https://reins2fa.com/.well-known/mcp-registry-auth`, served by the site repository (`reins-site`). The Reins server
   has no route for it.
3. **Version.** Follow the Reins release version (from `Cargo.toml`, set by the release tooling), or keep a separate
   listing version that changes only when the listing does?
4. **Icon.** The `raw.githubusercontent.com` URL works but depends on the default branch. A stable URL on
   `reins2fa.com` is better, if the site serves one.
5. **Tool profile.** The registry has no tool review, but the clients that read it may list it in their own
   catalogs. Publish the full server, or a host that runs the proposed directory tool profile
   (`REINS_MCP_TOOL_PROFILE=directory`, on the `feat/directory-tool-profile` branch, not merged)?
6. **Self-hosted servers.** The entry covers only `app.reins2fa.com`. A URL variable entry (`https://{host}/mcp`)
   for self-hosters is possible, but a remote URL can belong to one name only. Is it worth a second entry?
