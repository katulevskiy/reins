# X

The X connector lets an AI read and act on an X account through Reins. It uses the official
[X API v2](https://docs.x.com/x-api/introduction) only. It never scrapes x.com and never uses X's private web API.

Every request is made from the phone with the account's own OAuth 2.0 token, which stays sealed in the phone's store.
Reads hand over only what the user shares. Direct messages are sensitive, so the user ticks each one before it is
shared. Every write is shown on the phone exactly as it would be sent, and the user approves it there.

## What you need

- **An X developer account and app.** Sign up at the [X Developer Console](https://console.x.com) and create an app
  (a "Native App", which is a public client with no client secret). See
  [Getting access](https://docs.x.com/x-api/getting-started/getting-access).
- **OAuth 2.0 turned on for the app**, with:
  - **Type of app:** Native App (public client).
  - **App permissions:** Read and write and Direct message.
  - **Callback URI / Redirect URL:** `com.reins2fa.app://x-oauth`. This must match exactly.
  - **Website URL:** any page about your Reins setup.
- **The app's OAuth 2.0 Client ID.** Build the core with it in the environment:
  `REINS_X_CLIENT_ID=<client id> cargo build ...`. Without it, X shows as unavailable and makes no requests. No client
  secret is needed or used, because the phone is a public client and signs in with PKCE.
- **API credits.** The X API is
  [pay-per-use](https://docs.x.com/x-api/getting-started/pricing): every request is billed to the developer app's
  credits, which you buy in the Developer Console. When they run out, X answers 402 and Reins says so.

## Signing in

Reins uses [OAuth 2.0 Authorization Code with PKCE](https://docs.x.com/resources/fundamentals/authentication/oauth-2-0/authorization-code):

1. The phone opens `https://x.com/i/oauth2/authorize` with the client id, the callback URI, the scopes below, a random
   `state` and an S256 `code_challenge`.
2. The user signs in to X and allows Reins. X sends the browser back to `com.reins2fa.app://x-oauth?state=...&code=...`.
3. The phone checks the `state` and trades the code and the PKCE verifier for tokens at
   `POST https://api.x.com/2/oauth2/token`.
4. It asks `GET /2/users/me` who signed in, and keeps the tokens under that @username.

Access tokens last two hours. `offline.access` gives a refresh token, and Reins refreshes the access token shortly
before it runs out, or once if X refuses it. If the refresh is refused, the account is marked as needing a new
sign-in. Removing the account deletes its tokens from the phone. Revoke the app in X's settings to cancel it at X too.

Scopes asked for: `tweet.read`, `tweet.write`, `users.read`, `follows.read`, `follows.write`, `like.read`,
`like.write`, `mute.read`, `mute.write`, `block.read`, `block.write`, `bookmark.read`, `bookmark.write`, `dm.read`,
`dm.write`, `media.write` and `offline.access`.
[Scopes reference](https://docs.x.com/resources/fundamentals/authentication/oauth-2-0/authorization-code).

## Actions

Each tool lists the scopes it needs, and each write has a risk level. Risk is shown with the request; approvals work
as for every other connector. The prices are X's pay-per-use prices per request or per resource from the
[pricing page](https://docs.x.com/x-api/getting-started/pricing), and they may change.

### Reads

| Tool | Endpoint | Price |
| --- | --- | --- |
| `x_me` | [`GET /2/users/me`](https://docs.x.com/x-api/users/get-my-user) | Owned read, $0.001 |
| `x_user` | [`GET /2/users/by/username/:username`](https://docs.x.com/x-api/users/get-user-by-username), [`GET /2/users/:id`](https://docs.x.com/x-api/users/get-user-by-id) | User read, $0.010 |
| `x_post_get` | [`GET /2/tweets/:id`](https://docs.x.com/x-api/posts/get-post-by-id) | Post read, $0.005 |
| `x_home_timeline` | [`GET /2/users/:id/timelines/reverse_chronological`](https://docs.x.com/x-api/users/get-timeline) | $0.005 per post |
| `x_mentions` | [`GET /2/users/:id/mentions`](https://docs.x.com/x-api/users/get-mentions) | $0.005 per post |
| `x_search` | [`GET /2/tweets/search/recent`](https://docs.x.com/x-api/posts/search-recent-posts), last 7 days | $0.005 per post |
| `x_bookmarks` | [`GET /2/users/:id/bookmarks`](https://docs.x.com/x-api/users/get-bookmarks) | $0.005 per post |
| `x_dm_list` | [`GET /2/dm_events`](https://docs.x.com/x-api/direct-messages/get-dm-events), [`GET /2/dm_conversations/with/:id/dm_events`](https://docs.x.com/x-api/direct-messages/get-dm-events-for-a-dm-conversation) | DM event read, $0.010 |

Post reads are capped at 3 million a month on pay-per-use. Search needs at least 10 results a page and mentions at
least 5, so Reins asks for that many and hands over only as many as the AI asked for.

### Writes

| Tool | What the approval shows | Risk | Endpoint | Plan |
| --- | --- | --- | --- | --- |
| `x_media_upload` | The kind and size of the image, and whether it is for a post or a DM | Medium | [`POST /2/media/upload`](https://docs.x.com/x-api/media/upload-media) | Pay-per-use |
| `x_post` | The account, the exact text, the media ids, the quoted post | High | [`POST /2/tweets`](https://docs.x.com/x-api/posts/create-post) | Pay-per-use. Quoting (`quote`) needs **Enterprise**. |
| `x_reply` | The account, the post replied to (author and text), the exact text, the media | High | `POST /2/tweets` with `reply.in_reply_to_tweet_id` | Pay-per-use |
| `x_thread` | The account and every post of the thread, numbered | High | `POST /2/tweets`, one per post, each replying to the one before | Pay-per-use |
| `x_post_delete` | The account's post and its text. Asked every time. | High | [`DELETE /2/tweets/:id`](https://docs.x.com/x-api/posts/delete-post) | Pay-per-use |
| `x_like`, `x_unlike` | The post (author and text) | Medium | [`POST /2/users/:id/likes`](https://docs.x.com/x-api/users/like-post), [`DELETE .../likes/:tweet_id`](https://docs.x.com/x-api/users/unlike-post) | Pay-per-use |
| `x_repost`, `x_unrepost` | The post | Medium | [`POST /2/users/:id/retweets`](https://docs.x.com/x-api/users/repost-post), [`DELETE .../retweets/:tweet_id`](https://docs.x.com/x-api/users/unrepost-post) | Pay-per-use |
| `x_bookmark_add`, `x_bookmark_remove` | The post | Medium | [`POST /2/users/:id/bookmarks`](https://docs.x.com/x-api/users/create-bookmark), [`DELETE .../bookmarks/:tweet_id`](https://docs.x.com/x-api/users/delete-bookmark) | Pay-per-use |
| `x_follow`, `x_unfollow` | The account (@username and name) | Medium | [`POST /2/users/:id/following`](https://docs.x.com/x-api/users/follow-user), [`DELETE .../following/:target`](https://docs.x.com/x-api/users/unfollow-user) | Pay-per-use |
| `x_mute`, `x_unmute` | The account | Medium | [`POST /2/users/:id/muting`](https://docs.x.com/x-api/users/mute-user), [`DELETE .../muting/:target`](https://docs.x.com/x-api/users/unmute-user) | Pay-per-use |
| `x_block`, `x_unblock` | The account. Blocking is asked every time. | High / Medium | [`POST /2/users/:id/blocking`](https://docs.x.com/x-api/users/block-user), [`DELETE .../blocking/:target`](https://docs.x.com/x-api/users/unblock-user) | **Enterprise only** ([blocks](https://docs.x.com/x-api/users/blocks/introduction)) |
| `x_dm_send` | The account, the recipient (@username and name), the exact text, the media | High | [`POST /2/dm_conversations/with/:id/messages`](https://docs.x.com/x-api/direct-messages/create-dm-message-by-participant-id) | Pay-per-use |

Pay-per-use prices: creating a post costs $0.015, or $0.200 if it contains a URL. A DM costs $0.015, a like, repost,
follow or mute $0.015, undoing one $0.010, and a bookmark $0.005.

A preview looks up the post or account it is about, so the approval can name it. That lookup is a read and is billed
too.

Media: photos (JPEG, PNG, WebP) and GIFs, uploaded as base64 with `x_media_upload` and then attached by id. A post
takes up to 4 photos or 1 GIF. A DM takes 1. Reins checks the file's first bytes against the type it was given.
Video needs X's chunked upload, which Reins does not do.

Text over 280 characters is sent as it is, and the approval notes that X takes it only from accounts with X Premium.

## Rate limits and errors

X limits each endpoint per 15 minutes, per user, and some writes per 24 hours too. Examples: 100 posts per 15
minutes, 50 likes per 15 minutes and 1,000 per 24 hours, 15 DMs per 15 minutes and 1,440 per 24 hours. See
[Rate limits](https://docs.x.com/x-api/fundamentals/rate-limits). Reins reads the `x-rate-limit-limit`,
`x-rate-limit-remaining` and `x-rate-limit-reset` headers, and the `x-user-limit-24hour-*` headers. On a 429 it tells
the AI which limit is used up and when it resets, and does not retry by itself.

Other errors are explained without the token. See
[Response codes and errors](https://docs.x.com/x-api/fundamentals/response-codes-and-errors):

- **401:** the token is refreshed once. If that fails, the account needs a new sign-in.
- **402:** the app has no credits left.
- **403:** X refused this, with X's reason, such as an Enterprise-only endpoint or a protected account.
- **404:** there is no such post or account.
- **400 and 422:** the request was rejected, with X's reason.

## What is not done

- Video upload (chunked media upload).
- Polls, scheduled posts, Spaces, Lists and communities.
- Revoking the token at X when the account is removed. Do it in X's settings.
- The Integrations screen on the phones does not list X yet. The connector and its tools are in the core, and the
  apps' sign-in screen for it is a follow-up.
