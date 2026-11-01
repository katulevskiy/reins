//! X (formerly Twitter), through the official X API v2 only (<https://docs.x.com/x-api/introduction>): no scraping,
//! every request goes to `api.x.com` with the account's OAuth 2.0 token ([`oauth`]).
//!
//! The tokens stay in the phone's store, sealed per account (the @username, lowercase). Reads hand the AI posts and
//! profiles; direct messages are sensitive, so the user ticks each one before it is shared. Every write is shown on
//! the phone exactly as it would be sent (the account, the text, the post replied to or quoted, the recipient, the
//! media) and approved there. Rate limits are read from X's `x-rate-limit-*` headers
//! (<https://docs.x.com/x-api/fundamentals/rate-limits>).

mod oauth;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::{Arc, Mutex};

use data_encoding::{BASE64, HEXLOWER};
use reins_proto::connector::x::{MAX_IMAGE_BYTES, MAX_INLINE_IMAGE, MAX_POST_CHARS, MAX_THREAD};
use reins_proto::connector::{ConnectorCall, X};
use reqwest::header::HeaderMap;
use reqwest::{Method, StatusCode};
use serde_json::{Map, Value, json};

pub use self::oauth::{REDIRECT_URI, Tokens};
use super::{Connector, Item, Preview, PreviewImage};
use crate::blob::size_text;
use crate::store::{Store, unix_now};
use crate::types::GmailStatus;
use crate::{CoreError, text};

/// The X API (<https://docs.x.com/x-api/introduction>).
pub const X_BASE: &str = "https://api.x.com";
/// X's OAuth 2.0 sign-in page (<https://docs.x.com/resources/fundamentals/authentication/oauth-2-0/authorization-code>).
pub const X_AUTHORIZE_URL: &str = "https://x.com/i/oauth2/authorize";

/// Why X cannot be added in a build without the X developer app's client id (`docs/connectors/x.md`).
const NO_CLIENT_ID: &str = "This build has no X client id.";

const POST_FIELDS: &str =
    "author_id,conversation_id,created_at,in_reply_to_user_id,lang,public_metrics,referenced_tweets";
const USER_FIELDS: &str = "created_at,description,location,name,protected,public_metrics,url,username,verified";
const DM_FIELDS: &str = "id,text,created_at,sender_id,dm_conversation_id,attachments";

/// The X connector.
pub struct XApi {
    http: reqwest::Client,
    base: String,
    authorize_url: String,
    client_id: Option<String>,
    store: Arc<Store>,
    pending: Mutex<Option<oauth::Pending>>,
    refreshing: tokio::sync::Mutex<()>,
}

impl XApi {
    /// `base` is the API (`https://api.x.com`), `authorize_url` X's sign-in page (`https://x.com/i/oauth2/authorize`);
    /// tests point both at a fake server. Without a `client_id` the connector is unavailable.
    pub fn new(
        http: reqwest::Client,
        base: &str,
        authorize_url: &str,
        client_id: Option<String>,
        store: Arc<Store>,
    ) -> Self {
        Self {
            http,
            base: base.trim_end_matches('/').to_owned(),
            authorize_url: authorize_url.to_owned(),
            client_id,
            store,
            pending: Mutex::new(None),
            refreshing: tokio::sync::Mutex::new(()),
        }
    }

    /// One request with `token`, as it is: X's answer, or the error it means.
    async fn send(
        &self,
        token: &str,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<&Value>,
    ) -> Result<Value, CoreError> {
        let (status, limits, json) = self.exchange(token, method, path, query, body).await?;
        answer(status, &limits, json)
    }

    async fn exchange(
        &self,
        token: &str,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<&Value>,
    ) -> Result<(StatusCode, RateLimit, Value), CoreError> {
        let mut request = self.http.request(method.clone(), format!("{}{path}", self.base)).bearer_auth(token);
        if !query.is_empty() {
            request = request.query(query);
        }
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request
            .send()
            .await
            .map_err(|e| CoreError::service(format!("X could not be reached: {}", e.without_url())))?;
        let status = response.status();
        let limits = RateLimit::of(response.headers());
        log::debug!("x: {method} {path} answered {status}{limits}");
        let body = response.text().await.unwrap_or_default();
        let json = serde_json::from_str(&body).unwrap_or(Value::Null);
        Ok((status, limits, json))
    }

    /// A request in the account's name: its token is refreshed when it runs out, or once when X says it is no good.
    async fn call(
        &self,
        account: &str,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<&Value>,
    ) -> Result<Value, CoreError> {
        let mut tokens = self.tokens(account).await?;
        let (mut status, mut limits, mut json) =
            self.exchange(&tokens.access_token, method.clone(), path, query, body).await?;
        if status == StatusCode::UNAUTHORIZED && tokens.refresh_token.is_some() {
            tokens = self.refresh(account, &tokens).await?;
            (status, limits, json) = self.exchange(&tokens.access_token, method, path, query, body).await?;
        }
        answer(status, &limits, json)
    }

    async fn get(&self, account: &str, path: &str, query: &[(&str, String)]) -> Result<Value, CoreError> {
        self.call(account, Method::GET, path, query, None).await
    }

    /// The account's own numeric id.
    fn me(&self, account: &str) -> Result<String, CoreError> {
        Ok(self.load(account)?.user_id.clone())
    }

    /// An account, by @username or numeric id.
    async fn user(&self, account: &str, written: &str) -> Result<User, CoreError> {
        let path = match who(written)? {
            Who::Id(id) => format!("/2/users/{id}"),
            Who::Name(name) => format!("/2/users/by/username/{name}"),
        };
        let json = self.get(account, &path, &[("user.fields", USER_FIELDS.to_owned())]).await?;
        User::of(&json["data"]).ok_or_else(|| CoreError::invalid("X has no such account"))
    }

    /// A post, with its author.
    async fn post(&self, account: &str, id: &str) -> Result<Post, CoreError> {
        let json = self.get(account, &format!("/2/tweets/{id}"), &post_query()).await?;
        let authors = authors(&json);
        Post::of(&json["data"], &authors).ok_or_else(|| CoreError::invalid("X has no such post"))
    }

    async fn posts(
        &self,
        account: &str,
        path: &str,
        mut query: Vec<(&str, String)>,
        limit: usize,
    ) -> Result<Vec<Item>, CoreError> {
        query.extend(post_query());
        let json = self.get(account, path, &query).await?;
        let authors = authors(&json);
        Ok(json["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| Post::of(p, &authors))
            .take(limit)
            .map(Post::item)
            .collect())
    }

    async fn direct_messages(&self, account: &str, call: &ConnectorCall, limit: usize) -> Result<Vec<Item>, CoreError> {
        let path = match call.str_arg("user").map(str::trim).filter(|u| !u.is_empty()) {
            Some(user) => format!("/2/dm_conversations/with/{}/dm_events", self.user(account, user).await?.id),
            None => "/2/dm_events".to_owned(),
        };
        let json = self
            .get(
                account,
                &path,
                &[
                    ("dm_event.fields", DM_FIELDS.to_owned()),
                    ("event_types", "MessageCreate".to_owned()),
                    ("expansions", "sender_id".to_owned()),
                    ("user.fields", "name,username".to_owned()),
                    ("max_results", limit.to_string()),
                ],
            )
            .await?;
        let authors = authors(&json);
        Ok(json["data"].as_array().into_iter().flatten().filter_map(|e| dm_item(e, &authors)).take(limit).collect())
    }

    /// What a write would do, and what it is about, for the approval.
    async fn plan(&self, account: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
        let me = format!("@{account}");
        let own = |lines: Vec<String>| Preview {
            resource: account.to_owned(),
            resource_label: me.clone(),
            lines,
            ..Preview::default()
        };
        let about_post = |post: &Post, lines: Vec<String>| Preview {
            resource: post.author_id.clone(),
            resource_label: post.label(),
            lines,
            ..Preview::default()
        };
        let about_user = |user: &User, lines: Vec<String>| Preview {
            resource: user.id.clone(),
            resource_label: format!("@{}", user.username),
            lines,
            ..Preview::default()
        };
        Ok(match call.op.as_str() {
            "post" => {
                let (text, image) = post_body(call).await?;
                let mut lines = vec![format!("Post from {me} on X")];
                if let Some(quote) = opt_post_id(call, "quote")? {
                    let quoted = self.post(account, &quote).await?;
                    lines.push(format!("Quoting {}", quoted.describe()));
                }
                lines.extend(image.as_ref().map(Image::line));
                lines.extend(length_note(text));
                lines.push(text.to_owned());
                with_image(own(lines), image.as_ref())
            }
            "reply" => {
                let target = self.post(account, &post_id(call, "reply_to")?).await?;
                let (text, image) = post_body(call).await?;
                if text.trim().is_empty() {
                    return Err(CoreError::invalid("a reply needs text"));
                }
                let mut lines = vec![format!("Reply from {me} on X"), format!("In reply to {}", target.describe())];
                lines.extend(image.as_ref().map(Image::line));
                lines.extend(length_note(text));
                lines.push(text.to_owned());
                with_image(about_post(&target, lines), image.as_ref())
            }
            "thread" => {
                let posts = thread(call)?;
                let mut lines = vec![format!("Thread of {} posts from {me} on X", posts.len())];
                for (i, text) in posts.iter().enumerate() {
                    lines.push(format!("{}/{}: {text}", i + 1, posts.len()));
                }
                if posts.iter().any(|t| t.chars().count() > MAX_POST_CHARS) {
                    lines.push(format!(
                        "Some posts are longer than {MAX_POST_CHARS} characters: X takes them only from accounts with \
                         X Premium."
                    ));
                }
                own(lines)
            }
            "post_delete" => {
                let post = self.post(account, &post_id(call, "post")?).await?;
                if post.author_id != self.me(account)? {
                    return Err(CoreError::invalid(format!(
                        "{} is not {me}'s post: X deletes only your own posts",
                        post.id
                    )));
                }
                let mut preview = own(vec![format!("Delete {me}'s post {} on X for good", post.id), post.text]);
                preview.once_only = true;
                preview
            }
            op @ ("like" | "unlike" | "repost" | "unrepost" | "bookmark_add" | "bookmark_remove") => {
                let post = self.post(account, &post_id(call, "post")?).await?;
                let what = post.describe();
                let line = match op {
                    "like" => format!("Like {what}, as {me}"),
                    "unlike" => format!("Take back {me}'s like of {what}"),
                    "repost" => format!("Repost {what} to {me}'s followers"),
                    "unrepost" => format!("Undo {me}'s repost of {what}"),
                    "bookmark_add" => format!("Bookmark {what} (only {me} sees bookmarks)"),
                    _ => format!("Remove {what} from {me}'s bookmarks"),
                };
                about_post(&post, vec![line])
            }
            op @ ("follow" | "unfollow" | "mute" | "unmute" | "block" | "unblock") => {
                let user = self.user(account, required(call, "user")?).await?;
                let them = user.describe();
                let line = match op {
                    "follow" => format!("Follow {them}, as {me}"),
                    "unfollow" => format!("Unfollow {them}, as {me}"),
                    "mute" => format!("Mute {them} for {me}"),
                    "unmute" => format!("Unmute {them} for {me}"),
                    "block" => format!("Block {them} from {me}: they can no longer follow {me} or see {me}'s posts"),
                    _ => format!("Unblock {them} for {me}"),
                };
                let mut preview = about_user(&user, vec![line]);
                preview.once_only = op == "block";
                preview
            }
            "dm_send" => {
                let user = self.user(account, required(call, "user")?).await?;
                let text = call.str_arg("text").unwrap_or_default();
                let image = Image::of(call).await?;
                if text.trim().is_empty() && image.is_none() {
                    return Err(CoreError::invalid("a direct message needs text or an image"));
                }
                let mut lines = vec![format!("Direct message from {me} to {} on X", user.describe())];
                lines.extend(image.as_ref().map(Image::line));
                if !text.is_empty() {
                    lines.push(text.to_owned());
                }
                with_image(about_user(&user, lines), image.as_ref())
            }
            op => return Err(CoreError::invalid(format!("X has no write called {op}"))),
        })
    }

    async fn publish(&self, account: &str, body: &Value) -> Result<String, CoreError> {
        let json = self.call(account, Method::POST, "/2/tweets", &[], Some(body)).await?;
        json["data"]["id"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| CoreError::service("X did not say which post it made"))
    }

    /// The image of a post, a reply or a direct message, uploaded to X: its media id. Only the image the approval
    /// showed is sent (the same bytes).
    async fn upload(&self, account: &str, image: &Image, dm: bool, approved: &Preview) -> Result<String, CoreError> {
        if approved.image.as_ref().is_none_or(|shown| shown.sha256 != image.sha256()) {
            return Err(CoreError::invalid("the image is not the one that was approved"));
        }
        let body = json!({"media": BASE64.encode(&image.bytes), "media_category": image.category(dm)});
        let json = self.call(account, Method::POST, "/2/media/upload", &[], Some(&body)).await?;
        json["data"]["id"]
            .as_str()
            .filter(|id| is_numeric_id(id))
            .map(str::to_owned)
            .ok_or_else(|| CoreError::service("X did not say which media it kept"))
    }

    /// The write the user approved. An account is the one the approval showed (`approved.resource`, its numeric id),
    /// never looked up again by @username, which may have changed hands in between.
    async fn act(&self, account: &str, call: &ConnectorCall, approved: &Preview) -> Result<Value, CoreError> {
        let url = |id: &str| format!("https://x.com/{account}/status/{id}");
        let approved_user = || {
            Some(approved.resource.as_str())
                .filter(|id| is_numeric_id(id))
                .map(str::to_owned)
                .ok_or_else(|| CoreError::invalid("the approval did not name an X account"))
        };
        match call.op.as_str() {
            "post" | "reply" => {
                let (text, image) = post_body(call).await?;
                let mut body = Map::new();
                if !text.is_empty() {
                    body.insert("text".into(), text.into());
                }
                if let Some(image) = &image {
                    let media = self.upload(account, image, false, approved).await?;
                    body.insert("media".into(), json!({"media_ids": [media]}));
                }
                if call.op == "reply" {
                    body.insert("reply".into(), json!({"in_reply_to_tweet_id": post_id(call, "reply_to")?}));
                } else if let Some(quote) = opt_post_id(call, "quote")? {
                    body.insert("quote_tweet_id".into(), quote.into());
                }
                let id = self.publish(account, &Value::Object(body)).await?;
                if let Some(blob) = image.as_ref().and_then(|i| i.blob.as_deref()) {
                    crate::blob::used(blob).await;
                }
                Ok(json!({"id": id, "url": url(&id)}))
            }
            "thread" => {
                let mut ids: Vec<String> = Vec::new();
                for text in thread(call)? {
                    let mut body = json!({"text": text});
                    if let Some(last) = ids.last() {
                        body["reply"] = json!({"in_reply_to_tweet_id": last});
                    }
                    match self.publish(account, &body).await {
                        Ok(id) => ids.push(id),
                        Err(e) if ids.is_empty() => return Err(e),
                        Err(e) => {
                            return Err(CoreError::service(format!(
                                "{e}. The thread stopped after {} of its posts, which are published: {}",
                                ids.len(),
                                url(&ids[0])
                            )));
                        }
                    }
                }
                Ok(json!({"ids": ids, "url": url(&ids[0])}))
            }
            "post_delete" => {
                let id = post_id(call, "post")?;
                let json = self.call(account, Method::DELETE, &format!("/2/tweets/{id}"), &[], None).await?;
                Ok(json!({"deleted": json["data"]["deleted"]}))
            }
            op @ ("like" | "repost" | "bookmark_add") => {
                let id = post_id(call, "post")?;
                let path = format!("/2/users/{}/{}", self.me(account)?, collection(op));
                let json = self.call(account, Method::POST, &path, &[], Some(&json!({"tweet_id": id}))).await?;
                Ok(json["data"].clone())
            }
            op @ ("unlike" | "unrepost" | "bookmark_remove") => {
                let id = post_id(call, "post")?;
                let path = format!("/2/users/{}/{}/{id}", self.me(account)?, collection(op));
                let json = self.call(account, Method::DELETE, &path, &[], None).await?;
                Ok(json["data"].clone())
            }
            op @ ("follow" | "mute" | "block") => {
                let target = approved_user()?;
                let path = format!("/2/users/{}/{}", self.me(account)?, collection(op));
                let body = json!({"target_user_id": target});
                let json = self.call(account, Method::POST, &path, &[], Some(&body)).await?;
                Ok(json["data"].clone())
            }
            op @ ("unfollow" | "unmute" | "unblock") => {
                let target = approved_user()?;
                let path = format!("/2/users/{}/{}/{target}", self.me(account)?, collection(op));
                let json = self.call(account, Method::DELETE, &path, &[], None).await?;
                Ok(json["data"].clone())
            }
            "dm_send" => {
                let target = approved_user()?;
                let text = call.str_arg("text").unwrap_or_default();
                let image = Image::of(call).await?;
                let mut body = Map::new();
                if !text.is_empty() {
                    body.insert("text".into(), text.into());
                }
                if let Some(image) = &image {
                    let media = self.upload(account, image, true, approved).await?;
                    body.insert("attachments".into(), json!([{"media_id": media}]));
                }
                if body.is_empty() {
                    return Err(CoreError::invalid("a direct message needs text or an image"));
                }
                let path = format!("/2/dm_conversations/with/{target}/messages");
                let json = self.call(account, Method::POST, &path, &[], Some(&Value::Object(body))).await?;
                if let Some(blob) = image.as_ref().and_then(|i| i.blob.as_deref()) {
                    crate::blob::used(blob).await;
                }
                Ok(json!({
                    "dm_event_id": json["data"]["dm_event_id"],
                    "conversation_id": json["data"]["dm_conversation_id"],
                }))
            }
            op => Err(CoreError::invalid(format!("X has no write called {op}"))),
        }
    }
}

#[async_trait::async_trait]
impl Connector for XApi {
    fn service(&self) -> &'static str {
        X
    }

    async fn fetch(&self, account: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
        let limit = usize::try_from(call.int_arg("limit").unwrap_or(20).clamp(1, 100)).unwrap_or(20);
        match call.op.as_str() {
            "me" => {
                let json = self.get(account, "/2/users/me", &[("user.fields", USER_FIELDS.to_owned())]).await?;
                Ok(User::of(&json["data"]).map(User::item).into_iter().collect())
            }
            "user" => Ok(vec![self.user(account, required(call, "user")?).await?.item()]),
            "post_get" => Ok(vec![self.post(account, &post_id(call, "post")?).await?.item()]),
            "home_timeline" => {
                let path = format!("/2/users/{}/timelines/reverse_chronological", self.me(account)?);
                self.posts(account, &path, vec![("max_results", limit.to_string())], limit).await
            }
            "mentions" => {
                // X answers at least 5 mentions a page.
                let path = format!("/2/users/{}/mentions", self.me(account)?);
                self.posts(account, &path, vec![("max_results", limit.max(5).to_string())], limit).await
            }
            "search" => {
                // Recent search answers at least 10 posts a page.
                let query =
                    vec![("query", required(call, "query")?.to_owned()), ("max_results", limit.max(10).to_string())];
                self.posts(account, "/2/tweets/search/recent", query, limit).await
            }
            "bookmarks" => {
                let path = format!("/2/users/{}/bookmarks", self.me(account)?);
                self.posts(account, &path, vec![("max_results", limit.to_string())], limit).await
            }
            "dm_list" => self.direct_messages(account, call, limit).await,
            op => Err(CoreError::invalid(format!("X has no read called {op}"))),
        }
    }

    async fn preview(&self, account: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
        self.plan(account, call).await
    }

    /// Used on its own (without an approval to hand over): previews first, then acts on what the preview found.
    async fn perform(&self, account: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
        let approved = self.plan(account, call).await?;
        self.act(account, call, &approved).await
    }

    async fn perform_approved(
        &self,
        account: &str,
        call: &ConnectorCall,
        approved: &Preview,
    ) -> Result<Value, CoreError> {
        self.act(account, call, approved).await
    }

    async fn status(&self, account: &str) -> GmailStatus {
        if let Some(message) = self.unavailable() {
            return GmailStatus::Unavailable {
                message,
            };
        }
        // From the store only: every X API call is billed, so checking an account costs nothing.
        match self.load(account) {
            Ok(_) => GmailStatus::Ready,
            Err(_) => GmailStatus::NeedsConsent,
        }
    }

    fn unavailable(&self) -> Option<String> {
        self.client_id.as_deref().is_none_or(str::is_empty).then(|| NO_CLIENT_ID.to_owned())
    }

    async fn forget(&self, account: &str) {
        self.store.secret_delete(X, account).ok();
    }

    fn oauth_begin(&self) -> Result<String, CoreError> {
        self.begin()
    }

    async fn oauth_finish(&self, callback: &str) -> Result<String, CoreError> {
        self.finish(callback).await
    }
}

/// X's answer as a result: its JSON, or the error it means
/// (<https://docs.x.com/x-api/fundamentals/response-codes-and-errors>).
fn answer(status: StatusCode, limits: &RateLimit, json: Value) -> Result<Value, CoreError> {
    if status.is_success() {
        // A 200 can still carry only errors (a post that is gone, an account that is suspended).
        if json.get("data").is_none()
            && let Some(problem) = problem(&json)
        {
            return Err(CoreError::invalid(format!("X refused: {problem}")));
        }
        return Ok(json);
    }
    let detail = problem(&json).unwrap_or_else(|| status.canonical_reason().unwrap_or("error").to_owned());
    Err(match status.as_u16() {
        429 => CoreError::service(limits.used_up(unix_now())),
        401 => CoreError::needs_attention("X needs the account signed in again"),
        402 => CoreError::service(
            "X says the developer app has no API credits left (the X API is pay-per-use: add credits at console.x.com)",
        ),
        403 => CoreError::service(format!("X refused this (403): {detail}")),
        404 => CoreError::invalid(format!("X has no such post or account: {detail}")),
        400 | 422 => CoreError::invalid(format!("X rejected the request: {detail}")),
        _ => CoreError::service(format!("X answered {status}: {detail}")),
    })
}

/// What X said went wrong, made safe to show.
fn problem(json: &Value) -> Option<String> {
    let said = json["detail"]
        .as_str()
        .or_else(|| json["errors"][0]["detail"].as_str())
        .or_else(|| json["errors"][0]["message"].as_str())
        .or_else(|| json["title"].as_str())?;
    Some(text::truncate_chars(&text::one_line(&text::neutralize(said)), 300))
}

/// X's rate-limit headers (<https://docs.x.com/x-api/fundamentals/rate-limits>).
#[derive(Debug, Default, PartialEq, Eq)]
struct RateLimit {
    limit: Option<u64>,
    remaining: Option<u64>,
    /// Unix seconds.
    reset: Option<i64>,
    /// When the 24-hour limit some writes also have (likes, direct messages) is the one used up: when it resets.
    day_reset: Option<i64>,
}

fn header<T: std::str::FromStr>(headers: &HeaderMap, name: &str) -> Option<T> {
    headers.get(name)?.to_str().ok()?.trim().parse().ok()
}

impl RateLimit {
    fn of(headers: &HeaderMap) -> Self {
        let day_used_up = header::<u64>(headers, "x-user-limit-24hour-remaining") == Some(0);
        Self {
            limit: header(headers, "x-rate-limit-limit"),
            remaining: header(headers, "x-rate-limit-remaining"),
            reset: header(headers, "x-rate-limit-reset"),
            day_reset: if day_used_up {
                header(headers, "x-user-limit-24hour-reset")
            } else {
                None
            },
        }
    }

    fn used_up(&self, now: i64) -> String {
        let (which, reset) = match (self.day_reset, self.reset) {
            (Some(day), _) if day > now => ("daily limit", Some(day)),
            (_, reset) => ("rate limit", reset),
        };
        match reset.filter(|r| *r > now) {
            Some(reset) => {
                let minutes = (reset - now + 59) / 60;
                let plural = if minutes == 1 {
                    ""
                } else {
                    "s"
                };
                format!(
                    "X's {which} for this is used up: it resets at {} (in {minutes} minute{plural})",
                    text::iso_utc(reset)
                )
            }
            None => format!("X's {which} for this is used up: try again in a few minutes"),
        }
    }
}

impl std::fmt::Display for RateLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let (Some(remaining), Some(limit)) = (self.remaining, self.limit) {
            write!(f, " ({remaining}/{limit} left)")?;
        }
        Ok(())
    }
}

fn collection(op: &str) -> &'static str {
    match op {
        "like" | "unlike" => "likes",
        "repost" | "unrepost" => "retweets",
        "bookmark_add" | "bookmark_remove" => "bookmarks",
        "follow" | "unfollow" => "following",
        "mute" | "unmute" => "muting",
        _ => "blocking",
    }
}

fn post_query() -> Vec<(&'static str, String)> {
    vec![
        ("tweet.fields", POST_FIELDS.to_owned()),
        ("expansions", "author_id".to_owned()),
        ("user.fields", "name,username".to_owned()),
    ]
}

fn required<'a>(call: &'a ConnectorCall, name: &str) -> Result<&'a str, CoreError> {
    call.str_arg(name)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CoreError::invalid(format!("{name} is required")))
}

/// A post id, or an x.com link to the post.
fn parse_post_id(written: &str) -> Option<String> {
    let written = written.trim();
    let id = written
        .split_once("/status/")
        .map_or(written, |(_, rest)| rest.split(['/', '?', '#']).next().unwrap_or_default());
    (!id.is_empty() && id.len() <= 20 && id.bytes().all(|b| b.is_ascii_digit())).then(|| id.to_owned())
}

fn post_id(call: &ConnectorCall, name: &str) -> Result<String, CoreError> {
    parse_post_id(required(call, name)?)
        .ok_or_else(|| CoreError::invalid(format!("{name} must be a post id or its x.com link")))
}

fn opt_post_id(call: &ConnectorCall, name: &str) -> Result<Option<String>, CoreError> {
    match call.str_arg(name).map(str::trim).filter(|s| !s.is_empty()) {
        Some(_) => post_id(call, name).map(Some),
        None => Ok(None),
    }
}

fn is_username(name: &str) -> bool {
    (1..=15).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

enum Who {
    Id(String),
    Name(String),
}

fn who(written: &str) -> Result<Who, CoreError> {
    let written = written.trim();
    let name = written.strip_prefix('@').unwrap_or(written);
    if !written.starts_with('@') && !name.is_empty() && name.len() <= 20 && name.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(Who::Id(name.to_owned()));
    }
    if is_username(name) {
        return Ok(Who::Name(name.to_lowercase()));
    }
    Err(CoreError::invalid("the account must be an @username or a numeric id"))
}

/// The text and the image of a post or a reply: at least one of them.
async fn post_body(call: &ConnectorCall) -> Result<(&str, Option<Image>), CoreError> {
    let text = call.str_arg("text").unwrap_or_default();
    let image = Image::of(call).await?;
    if text.trim().is_empty() && image.is_none() {
        return Err(CoreError::invalid("a post needs text or an image"));
    }
    Ok((text, image))
}

fn thread(call: &ConnectorCall) -> Result<Vec<&str>, CoreError> {
    let posts: Vec<&str> = call.list_arg("posts");
    if posts.len() < 2 || posts.len() > MAX_THREAD {
        return Err(CoreError::invalid(format!("a thread has 2 to {MAX_THREAD} posts")));
    }
    if posts.iter().any(|p| p.trim().is_empty()) {
        return Err(CoreError::invalid("every post of a thread needs text"));
    }
    Ok(posts)
}

fn length_note(text: &str) -> Option<String> {
    (text.chars().count() > MAX_POST_CHARS)
        .then(|| format!("Longer than {MAX_POST_CHARS} characters: X takes it only from an account with X Premium."))
}

fn is_numeric_id(id: &str) -> bool {
    (1..=20).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit())
}

/// The preview, showing the image the write publishes.
fn with_image(mut preview: Preview, image: Option<&Image>) -> Preview {
    preview.image = image.map(Image::preview);
    preview
}

/// The image a post, a reply or a direct message carries (<https://docs.x.com/x-api/media/upload-media>): a photo or
/// a GIF, given inline (`content_base64`) or uploaded through the server (`blob`). The user sees it with the approval;
/// video needs X's chunked upload, which Reins does not do.
struct Image {
    bytes: Vec<u8>,
    mime: &'static str,
    name: String,
    /// The upload it came from, deleted once the write used it.
    blob: Option<String>,
}

impl Image {
    async fn of(call: &ConnectorCall) -> Result<Option<Self>, CoreError> {
        let given = |key: &str| call.str_arg(key).map(str::trim).filter(|v| !v.is_empty());
        let name =
            given("file_name").map_or_else(|| "image".to_owned(), |n| text::truncate_chars(&text::one_line(n), 200));
        let (bytes, blob) = match (call.str_arg("content_base64"), given("blob")) {
            (Some(_), Some(_)) => {
                return Err(CoreError::invalid("give the image either as content_base64 or as blob, not both"));
            }
            (Some(encoded), None) => {
                let compact: String = encoded.chars().filter(|c| !c.is_ascii_whitespace()).collect();
                let bytes = BASE64
                    .decode(compact.as_bytes())
                    .map_err(|_| CoreError::invalid("content_base64 must be base64"))?;
                if bytes.len() > MAX_INLINE_IMAGE {
                    return Err(CoreError::invalid(format!(
                        "an image over {} is uploaded instead: call again with file_name and without content_base64 \
                         for an upload link",
                        size_text(MAX_INLINE_IMAGE as u64)
                    )));
                }
                (bytes, None)
            }
            (None, Some(id)) => (crate::blob::read(id, MAX_IMAGE_BYTES).await?.to_vec(), Some(id.to_owned())),
            (None, None) if given("file_name").is_some() => {
                return Err(CoreError::invalid("the image has not been uploaded"));
            }
            (None, None) => return Ok(None),
        };
        if bytes.is_empty() {
            return Err(CoreError::invalid("the image is empty"));
        }
        if bytes.len() as u64 > MAX_IMAGE_BYTES {
            return Err(CoreError::invalid(format!("X takes images up to {}", size_text(MAX_IMAGE_BYTES))));
        }
        let mime = sniff(&bytes).ok_or_else(|| CoreError::invalid("the image is not a JPEG, PNG, WebP or GIF"))?;
        Ok(Some(Self {
            bytes,
            mime,
            name,
            blob,
        }))
    }

    fn category(&self, dm: bool) -> &'static str {
        match (dm, self.mime == "image/gif") {
            (false, false) => "tweet_image",
            (false, true) => "tweet_gif",
            (true, false) => "dm_image",
            (true, true) => "dm_gif",
        }
    }

    fn sha256(&self) -> String {
        HEXLOWER.encode(ring::digest::digest(&ring::digest::SHA256, &self.bytes).as_ref())
    }

    /// "With a PNG image (12.0 KB), shown below".
    fn line(&self) -> String {
        let kind = self.mime.trim_start_matches("image/").to_uppercase();
        format!("With a {kind} image ({}), shown below", size_text(self.bytes.len() as u64))
    }

    fn preview(&self) -> PreviewImage {
        PreviewImage {
            name: self.name.clone(),
            mime: self.mime.to_owned(),
            size: self.bytes.len() as u64,
            sha256: self.sha256(),
            data_base64: BASE64.encode(&self.bytes),
        }
    }
}

/// What the file's first bytes say it is.
fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// The accounts X included with an answer, by id: (name, username).
fn authors(json: &Value) -> HashMap<String, (String, String)> {
    json["includes"]["users"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|u| {
            Some((
                u["id"].as_str()?.to_owned(),
                (
                    u["name"].as_str().unwrap_or_default().to_owned(),
                    u["username"].as_str().unwrap_or_default().to_owned(),
                ),
            ))
        })
        .collect()
}

/// A post, as the AI and the approvals see it.
struct Post {
    id: String,
    author_id: String,
    name: String,
    username: String,
    text: String,
    created_at: i64,
    raw: Value,
}

impl Post {
    fn of(json: &Value, authors: &HashMap<String, (String, String)>) -> Option<Self> {
        let id = json["id"].as_str()?.to_owned();
        let author_id = json["author_id"].as_str().unwrap_or_default().to_owned();
        let (name, username) = authors.get(&author_id).cloned().unwrap_or_default();
        Some(Self {
            id,
            author_id,
            name: text::one_line(&text::neutralize(&name)),
            username,
            text: text::neutralize(json["text"].as_str().unwrap_or_default()),
            created_at: json["created_at"].as_str().and_then(text::parse_when).unwrap_or(0),
            raw: json.clone(),
        })
    }

    fn label(&self) -> String {
        if self.username.is_empty() {
            format!("user {}", self.author_id)
        } else {
            format!("@{}", self.username)
        }
    }

    fn url(&self) -> String {
        if self.username.is_empty() {
            format!("https://x.com/i/web/status/{}", self.id)
        } else {
            format!("https://x.com/{}/status/{}", self.username, self.id)
        }
    }

    /// "@name's post 123: "the first words"", for approvals.
    fn describe(&self) -> String {
        format!("{}'s post {}: \"{}\"", self.label(), self.id, text::truncate_chars(&text::one_line(&self.text), 200))
    }

    fn item(self) -> Item {
        let mut extra = Map::new();
        extra.insert("url".into(), self.url().into());
        for key in ["conversation_id", "in_reply_to_user_id", "referenced_tweets", "public_metrics", "lang"] {
            if let Some(v) = self.raw.get(key) {
                extra.insert(key.into(), v.clone());
            }
        }
        let from = if self.name.is_empty() {
            self.label()
        } else {
            format!("{} ({})", self.name, self.label())
        };
        Item {
            title: format!("{}'s post", self.label()),
            snippet: text::truncate_chars(&text::one_line(&self.text), 200),
            id: self.id,
            resource: self.author_id,
            resource_label: from.clone(),
            from,
            date: self.created_at,
            body: Some(self.text),
            extra,
            ..Item::default()
        }
    }
}

/// An account, as the AI and the approvals see it.
struct User {
    id: String,
    name: String,
    username: String,
    raw: Value,
}

impl User {
    fn of(json: &Value) -> Option<Self> {
        Some(Self {
            id: json["id"].as_str()?.to_owned(),
            name: text::one_line(&text::neutralize(json["name"].as_str().unwrap_or_default())),
            username: json["username"].as_str()?.to_owned(),
            raw: json.clone(),
        })
    }

    /// "@name (Display Name)", for approvals.
    fn describe(&self) -> String {
        if self.name.is_empty() {
            format!("@{}", self.username)
        } else {
            format!("@{} ({})", self.username, self.name)
        }
    }

    fn item(self) -> Item {
        let mut extra = Map::new();
        extra.insert("url".into(), format!("https://x.com/{}", self.username).into());
        for key in ["location", "protected", "verified", "public_metrics", "url", "created_at"] {
            if let Some(v) = self.raw.get(key) {
                extra.insert(key.into(), v.clone());
            }
        }
        let bio = text::neutralize(self.raw["description"].as_str().unwrap_or_default());
        let mut title = self.describe();
        if self.raw["protected"].as_bool() == Some(true) {
            let _ = write!(title, ", protected");
        }
        Item {
            resource: self.id.clone(),
            resource_label: format!("@{}", self.username),
            from: format!("@{}", self.username),
            title,
            snippet: text::truncate_chars(&text::one_line(&bio), 200),
            body: (!bio.is_empty()).then_some(bio),
            id: self.id,
            extra,
            ..Item::default()
        }
    }
}

/// A direct message: sensitive, so the user ticks it before it is shared.
fn dm_item(json: &Value, authors: &HashMap<String, (String, String)>) -> Option<Item> {
    let id = json["id"].as_str()?.to_owned();
    let sender = json["sender_id"].as_str().unwrap_or_default();
    let from = authors.get(sender).map_or_else(|| format!("user {sender}"), |(_, u)| format!("@{u}"));
    let conversation = json["dm_conversation_id"].as_str().unwrap_or_default().to_owned();
    let body = text::neutralize(json["text"].as_str().unwrap_or_default());
    let mut extra = Map::new();
    extra.insert("sender_id".into(), sender.into());
    if let Some(a) = json.get("attachments") {
        extra.insert("attachments".into(), a.clone());
    }
    Some(Item {
        id,
        resource_label: format!("Direct messages with {from}"),
        resource: conversation,
        title: format!("Direct message from {from}"),
        snippet: text::truncate_chars(&text::one_line(&body), 200),
        from,
        date: json["created_at"].as_str().and_then(text::parse_when).unwrap_or(0),
        body: Some(body),
        sensitive: true,
        extra,
        ..Item::default()
    })
}
