# The org executor

In Managed mode (organizations, `REINS_ORGS_ENABLED` on the server), approved actions are run by an **executor**: a
separate program with its own key, bound to one org, holding that org's credentials (a GitHub App key, a Google
Workspace service account). It only makes outbound connections: to the Reins server, to GitHub and to Google. It
checks every job on its own before acting: the decision service's countersignature, the issuer-signed policy and
roster, the reviewers' device signatures, the request digest and its own re-evaluation of the request (see the
[security model](security-model.md#decisions-in-managed-mode-o2)).

The same binary runs in every mode (`mode` in the configuration):

| Mode | Where | Who runs it |
|---|---|---|
| `cloud` | Reins Cloud, one isolated process per org | Reins |
| `dedicated` | a single-tenant deployment in the org's region | Reins, for the org |
| `self_hosted` | the org's own network (VPC, data center) | the org |

## Get it

Each release has, for Linux (static, no dependencies):

- `reins-executor-<version>-x86_64-unknown-linux-musl.tar.gz` and `…-aarch64-unknown-linux-musl.tar.gz` (also under
  `reins-executor-<target>.tar.gz` for the latest), listed in the release's `SHA256SUMS`;
- the container image `ghcr.io/katulevskiy/reins-executor:<version>` (and `:latest`), for amd64 and arm64. It holds
  only `reins-executor` and `reins-audit`, runs as user 65532 and keeps its state in `/var/lib/reins-executor`.

Or build it: `cargo build --release --locked -p reins-executor`.

## Set it up

1. **Make its key** on the host that will run it:

   ```sh
   reins-executor keygen --out /etc/reins-executor/executor.key
   ```

   It prints the public key. The key file stays on this host (mode 0600).
2. **Register it** in the console: Executors, a name (1–32 of `a-z`, `0-9`, `-`), the mode and the public key. Two
   admins approve it on their phones.
3. **Write the configuration** (below).
4. **Pin the org's keys:** `reins-executor pin --config executor.toml` prints the issuer and decision service keys'
   fingerprints. Compare them with the console's Executors page, then run it again with `--accept`. The executor
   never takes keys from the server after that.
5. **Run it:** `reins-executor run --config executor.toml`. `REINS_LOG=debug|info|warn` sets the log level (stderr).

## Configuration

TOML. Secrets are never written in it: a secret is named by a file (`{ file = "/path" }`) or an environment variable
(`{ env = "NAME" }`).

```toml
server = "https://reins.example.com"   # the Reins server's origin
org = "4022ebb8-…"                       # the org id (the console's URL)
executor_id = "ex-1"                     # the name it was registered with
mode = "self_hosted"                     # cloud | dedicated | self_hosted (default cloud)
key_file = "/etc/reins-executor/executor.key"
state_dir = "/var/lib/reins-executor"    # leases and the idempotency log; private to this user
max_roster_age_secs = 900                # optional: how old a signed roster may be (default 900)

[pins]                                   # written by `reins-executor pin --accept`
issuer = "iss-…=<base64url>"
decision = "dec-…=<base64url>"

[github]                                 # optional: the org's GitHub App installation
app_id = 12345
installation_id = 67890
owner = "acme"                           # only this account's repositories are served
private_key = { file = "/etc/reins-executor/github-app.pem" }
# api_base = "https://api.github.com"    # GitHub Enterprise Server: its API URL

[google]                                 # optional: Google Workspace (domain-wide delegation)
service_account = { env = "REINS_GOOGLE_SERVICE_ACCOUNT" }   # the service account's JSON key
# api_base = "https://www.googleapis.com"
# token_uri = "…"                        # overrides the key's token_uri
```

| Key | Required | Meaning |
|---|---|---|
| `server` | yes | the Reins server's origin; signed web approvals must come from it |
| `org` | yes | the org this executor serves (one org per executor) |
| `executor_id` | yes | its registered name |
| `mode` | no | `cloud`, `dedicated` or `self_hosted` |
| `key_file` | yes | its Ed25519 key from `keygen` |
| `state_dir` | yes | where leases and the idempotency log live |
| `max_roster_age_secs` | no | the oldest signed roster it accepts (seconds, > 0) |
| `pins.issuer`, `pins.decision` | to run | the org's keys, from `pin --accept` |
| `github.*` | for GitHub | `app_id`, `installation_id`, `owner`, `private_key`, optional `api_base` |
| `google.*` | for Google | `service_account`, optional `api_base`, `token_uri` |

## Run it

**Container:**

```sh
docker run -d --name reins-executor --restart unless-stopped \
  -v /etc/reins-executor:/etc/reins-executor:ro \
  -v reins-executor-state:/var/lib/reins-executor \
  -e REINS_GOOGLE_SERVICE_ACCOUNT="$(cat sa.json)" \
  ghcr.io/katulevskiy/reins-executor:latest
```

The image runs `run --config /etc/reins-executor/executor.toml`; make the mounted files readable by user 65532.
`keygen` and `pin` run the same way (`docker run --rm … ghcr.io/katulevskiy/reins-executor:latest keygen --out
/etc/reins-executor/executor.key` with that directory mounted read-write once).

**systemd** (one unit per org):

```ini
# /etc/systemd/system/reins-executor@.service
[Service]
ExecStart=/usr/local/bin/reins-executor run --config /etc/reins-executor/%i.toml
DynamicUser=yes
StateDirectory=reins-executor/%i
LoadCredential=key:/etc/reins-executor/%i.key
ProtectSystem=strict
NoNewPrivileges=yes
Restart=always
```

`systemctl enable --now reins-executor@<org>`.

## Network

Outbound only, all HTTPS: the Reins server, `api.github.com` (or the GitHub Enterprise API) and Google's APIs for
the connectors configured. Nothing connects to the executor. On Dedicated and Self-hosted, run it in its own network
zone, apart from the decision service (08 R5).

## Removing it

Remove it in the console (two admins). The server stops giving it work and its leases end; delete its key file and
state directory.
