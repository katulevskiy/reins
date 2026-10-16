# Perplexity and Grok: add Reins as a custom connector

Neither has a directory submission for Reins yet. Users add Reins themselves as a custom remote MCP connector. The
address and sign-in are the same as for Claude.ai:

| Setting | Value |
| --- | --- |
| URL | `https://app.reins2fa.com/mcp` (self-hosted: `https://<your host>/mcp`) |
| Transport | Streamable HTTP. Reins has no SSE endpoint. |
| Authentication | OAuth. The client discovers everything from the URL: a 401 points to the metadata, it registers itself (DCR) or uses a client ID metadata document, and it uses PKCE S256. There is no client ID, secret or API key to enter. |
| Sign-in | A browser page asks for your Reins email and shows a two-digit code; tap the same code on your phone and name the connection. |

Checked on 2026-10-16. Items marked **unverified** come from third-party guides or could not be tested without a
paid account.

## Perplexity

Official page: [Perplexity connectors (Agent API)](https://docs.perplexity.ai/docs/agent-api/tools/connectors). It
covers developer Projects, not the Perplexity app. There, a project admin adds a custom connector with a name, MCP
URL, authentication type and transport. The page lists only **API Key** or **None** for authentication. Reins needs
OAuth, so the Agent API route does not work for Reins today.

In the Perplexity app (Pro, Max and Enterprise; **unverified**, and sources disagree on which plans):

1. **Settings → Connectors → Add custom connector → Remote**.
2. Name: Reins. URL: `https://app.reins2fa.com/mcp`. Authentication: **OAuth**.
3. Save, then click the connector card to sign in. Finish the Reins sign-in in the browser tab before closing it.
4. Turn the connector on in each thread where you want it.

Known risk (**unverified**): a
[community report](https://community.perplexity.ai/t/custom-mcp-connector-fails-with-did-not-return-a-client-secret-for-rfc-7591-compliant-public-client-registrations/5172)
says Perplexity's custom connector fails with "did not return a client_secret" against servers that register public
clients, as RFC 7591 allows. Reins registers public clients (no secret, `token_endpoint_auth_method: none`), so it may
hit this. If it does, either wait for Perplexity's fix, or have Reins issue a client secret when a registration asks
for `client_secret_post`. Reins does not support that method today.

## Grok

Official page: [Grok connectors](https://docs.x.ai/grok/connectors).

1. Open [grok.com/connectors](https://grok.com/connectors).
2. **New Connector → Custom**.
3. Enter `https://app.reins2fa.com/mcp` and complete the sign-in (Reins email, then the code on your phone).

On Business and Enterprise, a team admin first provisions the connector in the xAI cloud console. Grok's page does not
say which authentication methods or transports it supports, so OAuth with DCR is **unverified**; Reins offers nothing
else. The server must be reachable from the public internet, which `app.reins2fa.com` is.

## Notes for both

- **Many tools.** Reins lists 281 tools. Clients without on-demand tool loading may truncate the list or spend
  context on it. If a client struggles, a narrower tool set per connection is the fix (not built).
- **Origin header.** Reins accepts browser `Origin` headers only from Claude and ChatGPT. Server-to-server calls send
  none, and that is how these clients are expected to connect. If a connection fails with "Origin not allowed", add the
  client's origin to `ALLOWED_ORIGINS` in `src/api/reins/mcp_routes.rs`. **Unverified** for both clients.
- **Approvals.** Results that say "waiting" mean the phone has not answered. The model is told to call
  `reins_get_result` once the user approves.
