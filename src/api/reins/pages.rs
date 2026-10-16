//! Server-rendered HTML for the OAuth authorize flow (Decision 18), the pairing link and the sign-out landing page:
//! no JavaScript, every dynamic value HTML-escaped, relative form/refresh/font URLs so the pages work under any
//! `DOMAIN` path prefix. They wear Reins Signal (`design/signal`, `Desk-Extras.png` "On the web"): paper and ink in
//! light and dark (`prefers-color-scheme`), serif titles with one italic word, inverse pill buttons, the number to tap
//! as a 96 px serif and a pairing code in mono in a sunken well. The faces are served from here
//! ([`font`], `font-src 'self'`); without them the serif falls back to Georgia.

/// Signal's colours and type. Light first, then dark: paper flips, the rest follows.
const STYLE: &str = ":root{--paper:#f6f6f8;--raised:#fff;--sunken:#ececf0;--ink:#16151b;--ink2:#5f5e6b;\
--meta:#6b6a78;--rule:#e3e3ea;--rule2:#d3d3dc;--inv:#16151b;--oninv:#fff;--violet:#5b43e8;--bad:#b91c1c;\
--dwash:#fbe3e3;--serif:\"Instrument Serif\",Georgia,\"Times New Roman\",serif;\
--sans:\"Instrument Sans\",system-ui,-apple-system,\"Segoe UI\",Roboto,sans-serif;\
--mono:\"Geist Mono\",ui-monospace,SFMono-Regular,Menlo,monospace;color-scheme:light dark}\
@media (prefers-color-scheme:dark){:root{--paper:#121215;--raised:#1c1c21;--sunken:#26262c;--ink:#ededf0;\
--ink2:#a6a6b0;--meta:#8e8e99;--rule:#2a2a30;--rule2:#3a3a42;--inv:#ededf0;--oninv:#121215;--violet:#a196ff;\
--bad:#f87171;--dwash:rgba(248,113,113,.14)}}\
body{font-family:var(--sans);margin:0;min-height:100vh;display:flex;align-items:center;justify-content:center;\
background:var(--paper);color:var(--ink);-webkit-font-smoothing:antialiased}\
main{background:var(--raised);max-width:24rem;width:100%;margin:1rem;padding:28px 26px;border-radius:18px;\
box-shadow:0 0 0 1px var(--rule),0 30px 60px -30px rgba(20,18,30,.4);display:flex;flex-direction:column;gap:14px;\
box-sizing:border-box}\
.mark{width:36px;height:36px;border-radius:9px;display:block}\
h1{font-family:var(--serif);font-weight:400;font-size:32px;line-height:34px;letter-spacing:-.01em;margin:0}\
h1 em{font-style:italic}\
p{font-size:14px;line-height:20px;margin:0;color:var(--ink2)}p strong{color:var(--ink);font-weight:600}\
.muted{color:var(--meta);font-size:13px;line-height:18px}\
.code{font-family:var(--serif);font-weight:400;font-size:96px;line-height:90px;letter-spacing:.02em;\
text-align:center;padding:6px 0;font-variant-numeric:lining-nums tabular-nums}\
.keyline{display:flex;flex-wrap:wrap;align-items:baseline;gap:4px 10px;padding:12px 14px;border-radius:14px;\
background:var(--sunken);color:var(--ink2)}\
.key{font-family:var(--serif);font-weight:400;font-size:22px;letter-spacing:.06em;color:var(--ink)}\
label{font-size:12.5px;color:var(--meta)}\
form{display:flex;flex-direction:column;gap:6px;margin:0}\
input[type=email]{height:44px;box-sizing:border-box;padding:0 14px;font:inherit;font-size:15px;color:var(--ink);\
background:var(--paper);border:0;box-shadow:0 0 0 1px var(--rule);border-radius:12px;margin-bottom:8px}\
input[type=email]:focus{outline:2px solid var(--violet);outline-offset:3px}\
button,a.button{display:flex;align-items:center;justify-content:center;box-sizing:border-box;height:44px;\
padding:0 18px;border:0;border-radius:999px;background:var(--inv);color:var(--oninv);font:inherit;font-size:15px;\
font-weight:600;text-decoration:none;cursor:pointer}\
button:active,a.button:active{opacity:.55}\
button:focus-visible,a.button:focus-visible{outline:2px solid var(--violet);outline-offset:3px}\
.error{background:var(--dwash);color:var(--bad);padding:10px 14px;border-radius:12px}\
a:not(.button){color:var(--ink2);text-underline-offset:.15em;text-decoration-thickness:1px}\
.pcode{font-family:var(--mono);font-size:26px;letter-spacing:.1em;text-align:center;padding:14px;\
border-radius:14px;background:var(--sunken);color:var(--ink)}\
.wait{display:flex;align-items:center;gap:8px;color:var(--meta);font-size:13px}\
.dot{width:8px;height:8px;border-radius:50%;background:var(--violet);flex:none;\
animation:breathe 1.8s ease-in-out infinite}\
@keyframes breathe{0%,100%{opacity:.35}50%{opacity:1}}\
@media (prefers-reduced-motion:reduce){.dot{animation:none}}";

/// The faces, as `@font-face` rules: `up` leads from the page back to the domain path's root (the fonts are at
/// `reins/fonts/` there), so the URLs stay relative.
fn font_faces(up: &str) -> String {
    let face = |family: &str, file: &str, weight: u16, style: &str| {
        format!(
            "@font-face{{font-family:\"{family}\";src:url(\"{up}reins/fonts/{file}.woff2\") format(\"woff2\");\
font-weight:{weight};font-style:{style};font-display:swap}}"
        )
    };
    [
        face("Instrument Serif", "InstrumentSerif-Regular", 400, "normal"),
        face("Instrument Serif", "InstrumentSerif-Italic", 400, "italic"),
        face("Instrument Sans", "InstrumentSans-Regular", 400, "normal"),
        face("Instrument Sans", "InstrumentSans-SemiBold", 600, "normal"),
        face("Geist Mono", "GeistMono", 400, "normal"),
    ]
    .concat()
}

/// The fonts the pages use, by file name.
const FONTS: [(&str, &[u8]); 5] = [
    ("InstrumentSerif-Regular.woff2", include_bytes!("../../static/fonts/InstrumentSerif-Regular.woff2")),
    ("InstrumentSerif-Italic.woff2", include_bytes!("../../static/fonts/InstrumentSerif-Italic.woff2")),
    ("InstrumentSans-Regular.woff2", include_bytes!("../../static/fonts/InstrumentSans-Regular.woff2")),
    ("InstrumentSans-SemiBold.woff2", include_bytes!("../../static/fonts/InstrumentSans-SemiBold.woff2")),
    ("GeistMono.woff2", include_bytes!("../../static/fonts/GeistMono.woff2")),
];

/// The Reins mark, inline (an `<img>` would need its own route): the shield and its check on the dark plate.
const MARK: &str = "<svg class=\"mark\" viewBox=\"0 0 108 108\" aria-hidden=\"true\">\
<rect width=\"108\" height=\"108\" rx=\"24\" fill=\"#170B33\"/>\
<g transform=\"translate(27 27) scale(2.25)\" fill=\"none\" stroke-linecap=\"round\" stroke-linejoin=\"round\">\
<path d=\"M12,3l7.5,3v5.5c0,4.6 -3.1,8.4 -7.5,9.5 -4.4,-1.1 -7.5,-4.9 -7.5,-9.5V6z\" stroke=\"#C9C1FB\" \
stroke-width=\"1.5\"/><path d=\"M8.7,12l2.4,2.4 4.2,-4.6\" stroke=\"#FFFFFF\" stroke-width=\"1.7\"/></g></svg>";

/// Where the pages are served, under `{domain_path}`: each page's relative URLs are worked out from its own path
/// ([`up_from`]), so they resolve under any `DOMAIN` path prefix.
pub mod at {
    /// The email form, and the errors of the authorize flow.
    pub const AUTHORIZE: &str = "/reins/oauth/authorize";
    /// The number to tap, and its errors.
    pub const WAIT: &str = "/reins/oauth/authorize/wait";
    /// The apps' browser sign-in (`identity::authorize`), on a server without one.
    pub const IDENTITY_AUTHORIZE: &str = "/identity/connect/authorize";
    /// A computer's pairing link.
    pub const PAIR: &str = "/pair";
    /// AuthKit's logout landing page.
    pub const SIGNED_OUT: &str = "/reins/signed-out";
}

/// What leads from a page served at `path` (under the domain path) back to the domain path's root: one `../` for
/// each folder the path is in (`/pair`: none, `/reins/oauth/authorize/wait`: three).
#[must_use]
pub fn up_from(path: &str) -> String {
    let folders = path.trim_matches('/').split('/').filter(|s| !s.is_empty()).count().saturating_sub(1);
    "../".repeat(folders)
}

/// Escapes text for HTML element content and quoted attribute values.
pub fn escape_html(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            other => out.push(other),
        }
    }
    out
}

/// A page served at `path` (one of [`at`]): its font URLs lead from there to the font route.
fn layout(title: &str, head_extra: &str, body: &str, path: &str) -> String {
    format!(
        "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
<meta name=\"robots\" content=\"noindex,nofollow\">{head_extra}<title>{title}</title>\
<style>{faces}{STYLE}</style></head><body><main>{body}</main></body></html>",
        title = escape_html(title),
        faces = font_faces(&up_from(path)),
    )
}

/// The desktop app's key fingerprint, which the user compares with the terminal and the phone before approving.
fn key_line(fingerprint: Option<&str>) -> String {
    fingerprint
        .map(|f| {
            format!(
                "<p class=\"keyline\">Desktop app key: <strong class=\"key\">{}</strong>must match your phone</p>",
                escape_html(f)
            )
        })
        .unwrap_or_default()
}

/// Step 1: ask for the account email. `session` is an opaque server-side session id.
pub fn email_form_page(
    client_name: &str,
    client_host: &str,
    session: &str,
    error: Option<&str>,
    key_fingerprint: Option<&str>,
) -> String {
    let error_html =
        error.map(|e| format!("<p class=\"error\" role=\"alert\">{}</p>", escape_html(e))).unwrap_or_default();
    let key_html = key_line(key_fingerprint);
    let body = format!(
        "{MARK}<h1>Connect <em>{name}</em> to Reins</h1>\
<p><strong>{name}</strong> ({host}) wants to use the tools you connect in Reins. \
Nothing is shared until you approve it on your phone.</p>{key_html}{error_html}\
<form method=\"post\" action=\"authorize\">\
<input type=\"hidden\" name=\"session\" value=\"{session}\">\
<label for=\"email\">Your account email</label>\
<input id=\"email\" name=\"email\" type=\"email\" autocomplete=\"email\" required autofocus>\
<button type=\"submit\">Continue</button></form>\
<p class=\"muted\">You will get a request in the Reins app on your phone.</p>",
        name = escape_html(client_name),
        host = escape_html(client_host),
        session = escape_html(session),
    );
    layout("Connect to Reins", "", &body, at::AUTHORIZE)
}

/// Step 2: show the code to pick on the phone; reloads itself until the phone has answered.
pub fn wait_page(client_name: &str, code: u8, key_fingerprint: Option<&str>) -> String {
    let body = format!(
        "<h1>Approve on <em>your phone</em></h1>\
<p>Open Reins and tap this number to connect <strong>{name}</strong>:</p>\
<div class=\"code\" aria-label=\"code\">{code}</div>{key_html}\
<p class=\"wait\"><span class=\"dot\"></span>This page continues by itself.</p>\
<p class=\"muted\">If the number is not offered on your phone, deny the request there.</p>",
        name = escape_html(client_name),
        key_html = key_line(key_fingerprint),
    );
    layout("Approve on your phone", "<meta http-equiv=\"refresh\" content=\"2\">", &body, at::WAIT)
}

/// A terminal error the user cannot recover from, shown at `path` (one of [`at`]).
pub fn error_page(path: &str, title: &str, message: &str) -> String {
    let body = format!("{MARK}<h1>{}</h1><p>{}</p>", escape_html(title), escape_html(message));
    layout(title, "", &body, path)
}

// ---------------------------------------------------------------------------------------
// Pairing links (`/pair?code=`) and the phone apps' link associations
// ---------------------------------------------------------------------------------------

/// The id of both phone apps: the iOS bundle id and the Android application id.
pub const APP_ID: &str = "com.reins2fa.app";
/// The custom scheme both apps register (`reins://pair?code=`), for when a link does not open the app by itself.
pub const APP_SCHEME: &str = "reins";
/// The phone download page handles platform-specific store links and release availability.
/// Keep pairing links usable before the App Store and Play Store listings exist.
pub const MOBILE_APP_URL: &str = concat!(reins_proto::default_server!(), "/get");

/// Which link opens the app on the phone that opened the page, from its `User-Agent`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    /// An `intent://` link: Chrome opens the app, or the download page when it is not installed.
    Android,
    /// iPhone, iPad (which says "Macintosh"), a computer: `reins://`.
    Other,
}

impl Platform {
    pub fn from_user_agent(user_agent: Option<&str>) -> Self {
        if user_agent.is_some_and(|ua| ua.contains("Android")) {
            Self::Android
        } else {
            Self::Other
        }
    }
}

#[rocket::async_trait]
impl<'r> rocket::request::FromRequest<'r> for Platform {
    type Error = std::convert::Infallible;

    async fn from_request(request: &'r rocket::Request<'_>) -> rocket::request::Outcome<Self, Self::Error> {
        rocket::request::Outcome::Success(Self::from_user_agent(request.headers().get_one("User-Agent")))
    }
}

/// The link that opens `code` in the app on `platform`. `code` is a normalized user code (letters and a dash).
pub fn open_in_app_link(code: &str, platform: Platform) -> String {
    let query: String = url::form_urlencoded::Serializer::new(String::new()).append_pair("code", code).finish();
    match platform {
        Platform::Android => {
            let fallback: String = url::form_urlencoded::byte_serialize(MOBILE_APP_URL.as_bytes()).collect();
            format!(
                "intent://pair?{query}#Intent;scheme={APP_SCHEME};package={APP_ID};S.browser_fallback_url={fallback};end"
            )
        }
        Platform::Other => format!("{APP_SCHEME}://pair?{query}"),
    }
}

/// `{DOMAIN}/pair?code=`: what a computer's pairing QR code links to. With the app installed (and the app links set up)
/// the phone opens the link in the app and never shows this page; otherwise it offers to open the app, or to install
/// it. `code` is `None` when the link has no valid code.
pub fn pair_page(code: Option<&str>, platform: Platform) -> String {
    let install = format!(
        "<p class=\"muted\">No app yet? <a href=\"{phone}\">Get it</a>, sign in, then scan again.</p>",
        phone = escape_html(MOBILE_APP_URL),
    );
    let body = match code {
        Some(code) => format!(
            "<h1>Connect your <em>computer</em></h1>\
<p>Your computer shows this code. Open it in Reins on your phone:</p>\
<div class=\"pcode\" aria-label=\"pairing code\">{code}</div>\
<a class=\"button\" href=\"{open}\">Open in Reins</a>{install}\
<p class=\"muted\">On a computer? Scan the QR code with your phone's camera instead.</p>",
            code = escape_html(code),
            open = escape_html(&open_in_app_link(code, platform)),
        ),
        None => format!(
            "<h1>Connect your <em>computer</em></h1>\
<p>Scan the QR code your computer shows with your phone's camera, or in the Reins app: \
<strong>Settings</strong>, <strong>Connect a computer</strong>.</p>{install}"
        ),
    };
    layout("Connect your computer to Reins", "", &body, at::PAIR)
}

/// `/.well-known/apple-app-site-association`: pairing links under `{domain_path}/pair` open the iOS app of `team_id`
/// (none when it is empty), and the app may make and use passkeys for this domain (the vault's passkeys,
/// `reins_proto::vault_passkey`). `bitwarden_credentials` keeps what the web vault serves there (the Bitwarden apps'
/// shared web credentials).
pub fn apple_app_site_association(team_id: &str, domain_path: &str, bitwarden_credentials: bool) -> serde_json::Value {
    let team = team_id.trim();
    let mut doc = serde_json::json!({});
    let mut credential_apps = Vec::new();
    if !team.is_empty() {
        credential_apps.push(format!("{team}.{APP_ID}"));
        let path = format!("{}/pair", domain_path.trim_end_matches('/'));
        doc["applinks"] = serde_json::json!({
            "details": [{
                "appIDs": [format!("{team}.{APP_ID}")],
                "components": [{"/": path, "comment": "A computer's pairing code (QR code)"}]
            }]
        });
    }
    if bitwarden_credentials {
        credential_apps
            .extend(["LTZ2PFU5D6.com.8bit.bitwarden".to_owned(), "LTZ2PFU5D6.com.8bit.bitwarden.beta".to_owned()]);
    }
    if !credential_apps.is_empty() {
        doc["webcredentials"] = serde_json::json!({"apps": credential_apps});
    }
    doc
}

/// The certificate fingerprints of `REINS_ANDROID_CERT_SHA256`: comma-separated, with or without colons, as
/// `AB:CD:...` (upper case). Anything that is not 32 bytes of hex is dropped (and named in the log at launch).
pub fn android_cert_fingerprints(raw: &str) -> (Vec<String>, Vec<String>) {
    let (mut good, mut bad) = (Vec::new(), Vec::new());
    for item in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let hex: String = item.chars().filter(|c| *c != ':').collect();
        if hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            let upper = hex.to_ascii_uppercase();
            let pairs: Vec<&str> = (0..32).map(|i| &upper[2 * i..2 * i + 2]).collect();
            good.push(pairs.join(":"));
        } else {
            bad.push(item.to_owned());
        }
    }
    (good, bad)
}

/// `/.well-known/assetlinks.json`: pairing links open the Android app signed with one of `fingerprints`, and that app
/// may make and use passkeys for this domain (the vault's passkeys); an empty list when there are none (links then
/// open the page).
pub fn asset_links(fingerprints: &[String]) -> serde_json::Value {
    if fingerprints.is_empty() {
        return serde_json::json!([]);
    }
    serde_json::json!([{
        "relation": ["delegate_permission/common.handle_all_urls", "delegate_permission/common.get_login_creds"],
        "target": {"namespace": "android_app", "package_name": APP_ID, "sha256_cert_fingerprints": fingerprints}
    }])
}

/// Routes mounted at `{domain_path}/`.
pub fn routes() -> Vec<rocket::Route> {
    routes![pair, signed_out, font]
}

/// AuthKit's logout landing page never initiates a new sign-in. The explicit link also works in Firefox,
/// which may require a user gesture to hand a custom scheme back to the native app.
#[get("/reins/signed-out")]
fn signed_out() -> rocket::response::content::RawHtml<String> {
    rocket::response::content::RawHtml(layout(
        "Signed out",
        "",
        &format!(
            "{MARK}<h1>Signed <em>out</em></h1><p>Your browser sign-in session has ended.</p>\
<a class=\"button\" href=\"com.reins2fa.app://signed-out\">Return to Reins</a>\
<p class=\"muted\">You can also close this page and return to the app.</p>"
        ),
        at::SIGNED_OUT,
    ))
}

/// The pages' faces (WOFF2, a year in caches: they never change under a name).
#[get("/reins/fonts/<name>")]
fn font(name: &str) -> Option<crate::util::Cached<(rocket::http::ContentType, &'static [u8])>> {
    let (_, bytes) = FONTS.iter().find(|(file, _)| *file == name)?;
    Some(crate::util::Cached::long((rocket::http::ContentType::new("font", "woff2"), *bytes), true))
}

/// Routes mounted at the server root `/`, where the phones look for them.
pub fn app_link_routes() -> Vec<rocket::Route> {
    let (_, bad) = android_cert_fingerprints(&crate::CONFIG.reins_android_cert_sha256());
    for item in bad {
        warn!("`REINS_ANDROID_CERT_SHA256`: `{item}` is not a SHA-256 fingerprint (32 bytes of hex); ignored");
    }
    routes![apple_app_site_association_doc, asset_links_doc, openai_apps_challenge]
}

/// The token alone, as OpenAI's domain verification wants it; `None` (not found) when none is configured or it is not
/// one token-like word (letters, digits, `-`, `_`, `.`, `~`, `=`, `+`, `/`).
pub fn openai_challenge_body(configured: &str) -> Option<String> {
    let token = configured.trim();
    let token_char = |c: char| c.is_ascii_alphanumeric() || "-_.~=+/".contains(c);
    (!token.is_empty() && token.len() <= 512 && token.chars().all(token_char)).then(|| token.to_owned())
}

/// OpenAI's domain verification for a ChatGPT directory submission (`REINS_OPENAI_APPS_CHALLENGE`).
#[get("/.well-known/openai-apps-challenge")]
fn openai_apps_challenge() -> Option<(rocket::http::ContentType, String)> {
    openai_challenge_body(&crate::CONFIG.reins_openai_apps_challenge()).map(|t| (rocket::http::ContentType::Plain, t))
}

#[get("/pair?<code>")]
fn pair(code: Option<&str>, platform: Platform) -> rocket::response::content::RawHtml<String> {
    let code = code.and_then(reins_proto::pairing::normalize_user_code);
    rocket::response::content::RawHtml(pair_page(code.as_deref(), platform))
}

#[get("/.well-known/apple-app-site-association")]
fn apple_app_site_association_doc() -> (rocket::http::ContentType, String) {
    let team = Some(crate::CONFIG.reins_apple_team_id())
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| crate::CONFIG.reins_apns_team_id());
    let doc = apple_app_site_association(&team, &crate::CONFIG.domain_path(), crate::CONFIG.web_vault_enabled());
    (rocket::http::ContentType::JSON, doc.to_string())
}

#[get("/.well-known/assetlinks.json")]
fn asset_links_doc() -> (rocket::http::ContentType, String) {
    let (fingerprints, _) = android_cert_fingerprints(&crate::CONFIG.reins_android_cert_sha256());
    (rocket::http::ContentType::JSON, asset_links(&fingerprints).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const EVIL: &str = "<script>alert(1)</script>\"'&";

    #[test]
    fn escapes_the_five_html_metacharacters() {
        assert_eq!(escape_html(EVIL), "&lt;script&gt;alert(1)&lt;/script&gt;&quot;&#x27;&amp;");
        assert_eq!(escape_html("plain text"), "plain text");
    }

    #[test]
    fn hostile_values_never_reach_the_markup_unescaped() {
        for page in [
            email_form_page(EVIL, EVIL, EVIL, Some(EVIL), Some(EVIL)),
            wait_page(EVIL, 47, Some(EVIL)),
            error_page(at::AUTHORIZE, EVIL, EVIL),
        ] {
            assert!(!page.contains("<script"), "{page}");
            assert!(!page.contains("alert(1)</script>"), "{page}");
            assert!(page.contains("&lt;script&gt;"), "{page}");
        }
    }

    #[test]
    fn pages_use_no_javascript_and_only_relative_urls() {
        for page in [
            email_form_page("Claude", "claude.ai", "sess", None, Some("4821 9930")),
            wait_page("Claude", 12, Some("4821 9930")),
            error_page(at::AUTHORIZE, "t", "m"),
        ] {
            assert!(!page.to_lowercase().contains("<script"), "{page}");
            assert!(!page.contains("http://") && !page.contains("https://"), "{page}");
            assert!(!page.to_lowercase().contains("javascript:"), "{page}");
            assert!(page.starts_with("<!DOCTYPE html>"));
        }
    }

    #[test]
    fn form_posts_the_session_and_email_to_a_relative_url() {
        let page = email_form_page("ChatGPT", "chatgpt.com", "abc-123", None, None);
        assert!(page.contains("method=\"post\" action=\"authorize\""));
        assert!(page.contains("name=\"session\" value=\"abc-123\""));
        assert!(page.contains("type=\"email\""));
        assert!(page.contains("ChatGPT") && page.contains("chatgpt.com"));
        assert!(!page.contains("role=\"alert\""), "no error box without an error");
        assert!(email_form_page("c", "h", "s", Some("Too many attempts"), None).contains("Too many attempts"));
    }

    #[test]
    fn wait_page_shows_the_code_and_refreshes_itself() {
        let page = wait_page("ChatGPT", 47, None);
        assert!(page.contains(">47<"));
        assert!(page.contains("<meta http-equiv=\"refresh\" content=\"2\">"));
        assert!(!email_form_page("c", "h", "s", None, None).contains("http-equiv"), "only the wait page refreshes");
    }

    #[test]
    fn the_pair_page_opens_the_app_or_offers_it() {
        let ios = pair_page(Some("BCDF-GHJK"), Platform::Other);
        assert!(ios.contains(">BCDF-GHJK<"), "{ios}");
        assert!(ios.contains("href=\"reins://pair?code=BCDF-GHJK\""), "{ios}");
        assert!(ios.contains(MOBILE_APP_URL));
        assert!(!ios.contains("id0000000000"), "never send a user to a fake App Store listing");
        let android = pair_page(Some("BCDF-GHJK"), Platform::Android);
        assert!(
            android.contains(
                "href=\"intent://pair?code=BCDF-GHJK#Intent;scheme=reins;package=com.reins2fa.app;\
S.browser_fallback_url=https%3A%2F%2Fapp.reins2fa.com%2Fget;end\""
            ),
            "{android}"
        );
        let bare = pair_page(None, Platform::Other);
        assert!(!bare.contains("reins://") && bare.contains("Connect a computer"), "{bare}");
        for page in [ios, android, bare] {
            assert!(!page.to_lowercase().contains("<script") && !page.contains("http-equiv"), "{page}");
        }
        assert_eq!(Platform::from_user_agent(Some("Mozilla/5.0 (Linux; Android 15; Pixel 9)")), Platform::Android);
        assert_eq!(Platform::from_user_agent(Some("Mozilla/5.0 (iPhone; CPU iPhone OS 26_0)")), Platform::Other);
        assert_eq!(Platform::from_user_agent(None), Platform::Other);
    }

    #[test]
    fn app_link_documents() {
        let aasa = apple_app_site_association("DEF123GHIJ", "", false);
        assert_eq!(aasa["applinks"]["details"][0]["appIDs"], serde_json::json!(["DEF123GHIJ.com.reins2fa.app"]));
        assert_eq!(aasa["applinks"]["details"][0]["components"][0]["/"], "/pair");
        assert_eq!(aasa["webcredentials"]["apps"], serde_json::json!(["DEF123GHIJ.com.reins2fa.app"]), "passkeys");
        let under = apple_app_site_association(" T ", "/vw/", true);
        assert_eq!(under["applinks"]["details"][0]["components"][0]["/"], "/vw/pair");
        assert_eq!(
            under["webcredentials"]["apps"],
            serde_json::json!([
                "T.com.reins2fa.app",
                "LTZ2PFU5D6.com.8bit.bitwarden",
                "LTZ2PFU5D6.com.8bit.bitwarden.beta"
            ])
        );
        let none = apple_app_site_association("", "", false);
        assert!(none.get("applinks").is_none(), "no team id, no app links");
        assert!(none.get("webcredentials").is_none(), "nor passkeys");

        let hex = "146de983c5730650d8eeb9952f34fc6416a08342e61dbea88a0496b23fcf44e5";
        let colons = "14:6D:E9:83:C5:73:06:50:D8:EE:B9:95:2F:34:FC:64:16:A0:83:42:E6:1D:BE:A8:8A:04:96:B2:3F:CF:44:E5";
        let (good, bad) = android_cert_fingerprints(&format!(" {hex} , {colons},nope,"));
        assert_eq!(good, [colons, colons]);
        assert_eq!(bad, ["nope"]);
        let links = asset_links(&good[..1]);
        assert_eq!(links[0]["target"]["package_name"], "com.reins2fa.app");
        assert_eq!(links[0]["target"]["sha256_cert_fingerprints"], serde_json::json!([colons]));
        assert_eq!(
            links[0]["relation"],
            serde_json::json!([
                "delegate_permission/common.handle_all_urls",
                "delegate_permission/common.get_login_creds"
            ])
        );
        assert_eq!(asset_links(&[]), serde_json::json!([]));
    }

    /// The font URLs of a page, as written in its `@font-face` rules.
    fn font_urls(page: &str) -> Vec<&str> {
        page.split("url(\"").skip(1).filter_map(|rest| rest.split_once("\")").map(|(url, _)| url)).collect()
    }

    /// Every page, at the path it is served from (under the domain path): each of its font URLs, relative, resolves
    /// to the font route there, under any `DOMAIN` path prefix, and names a file the route serves.
    #[test]
    fn every_font_url_resolves_to_the_font_route() {
        let route = routes![font].remove(0);
        let font_path = route.uri.path().strip_suffix("<name>").unwrap().to_owned();
        assert_eq!(font_path, "/reins/fonts/");
        let pages = [
            (at::AUTHORIZE, email_form_page("c", "h", "s", None, None)),
            (at::AUTHORIZE, error_page(at::AUTHORIZE, "t", "m")),
            (at::WAIT, wait_page("c", 1, None)),
            (at::WAIT, error_page(at::WAIT, "Link expired", "m")),
            (at::IDENTITY_AUTHORIZE, error_page(at::IDENTITY_AUTHORIZE, "t", "m")),
            (at::PAIR, pair_page(None, Platform::Other)),
            (at::PAIR, pair_page(Some("BCDF-GHJK"), Platform::Android)),
            (at::SIGNED_OUT, signed_out().0),
        ];
        for domain_path in ["", "/vw", "/a/b"] {
            for (served, page) in &pages {
                let base = url::Url::parse(&format!("https://reins.example{domain_path}{served}")).unwrap();
                let urls = font_urls(page);
                assert_eq!(urls.len(), FONTS.len(), "{served}");
                for u in urls {
                    assert!(!u.starts_with('/') && !u.contains("://"), "{served}: {u} is relative");
                    let resolved = base.join(u).unwrap();
                    let file = resolved
                        .path()
                        .strip_prefix(&format!("{domain_path}{font_path}"))
                        .unwrap_or_else(|| panic!("{served} under {domain_path:?}: {u} goes to {resolved}"));
                    assert!(font(file).is_some(), "{served}: {file} is served");
                }
            }
        }
        assert_eq!(up_from(at::PAIR), "");
        assert_eq!(up_from(at::WAIT), "../../../");
        assert!(pages.iter().all(|(_, p)| p.contains("Georgia")), "the serif falls back to Georgia");
    }

    #[test]
    fn the_font_files_are_small_woff2() {
        for (file, bytes) in FONTS {
            assert_eq!(&bytes[..4], b"wOF2", "{file}");
            assert!(bytes.len() < 40_000, "{file} is a Latin subset");
            assert!(font(file).is_some());
        }
        assert!(font("../Cargo.toml").is_none() && font("nope.woff2").is_none());
    }

    /// Titles are the serif with one italic word; the number to tap is the big serif, the pairing code mono.
    #[test]
    fn the_pages_speak_signal() {
        assert!(wait_page("c", 42, None).contains("<h1>Approve on <em>your phone</em></h1>"));
        assert!(email_form_page("Claude", "claude.ai", "s", None, None).contains("Connect <em>Claude</em> to Reins"));
        assert!(pair_page(Some("WDJB-MJHT"), Platform::Other).contains("Connect your <em>computer</em>"));
        assert!(STYLE.contains("prefers-color-scheme:dark") && STYLE.contains("font-size:96px"));
        assert!(!email_form_page(EVIL, "h", "s", None, None).contains("<em><script"));
    }

    /// `REINS_PAGES_OUT=<dir> cargo test --features sqlite pages_for_screenshots -- --ignored`: writes every page at the
    /// path it is served from (so its relative URLs resolve when the directory is served), for screenshots.
    #[test]
    #[ignore = "writes files, for screenshots"]
    fn pages_for_screenshots() {
        let Some(out) = std::env::var_os("REINS_PAGES_OUT").map(std::path::PathBuf::from) else {
            return;
        };
        let pages = [
            ("reins/oauth/authorize.html", email_form_page("Claude", "claude.ai", "s", None, None)),
            (
                "reins/oauth/authorize-key.html",
                email_form_page(
                    "Reins desktop",
                    "127.0.0.1",
                    "s",
                    Some("That email has no account"),
                    Some("7712 0458"),
                ),
            ),
            ("reins/oauth/authorize/wait.html", wait_page("Claude", 42, None)),
            ("pair.html", pair_page(Some("WDJB-MJHT"), Platform::Other)),
            ("pair-none.html", pair_page(None, Platform::Other)),
            (
                "identity/connect/authorize.html",
                error_page(
                    at::IDENTITY_AUTHORIZE,
                    "No browser sign-in on this server",
                    "This server signs in with an email and a master password. Close this page and, in the app, use Sign in or Create account.",
                ),
            ),
            ("reins/signed-out.html", signed_out().0),
        ];
        for (path, html) in pages {
            let file = out.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            // Without the meta refresh: a screenshot waits for nothing.
            std::fs::write(&file, html.replace("<meta http-equiv=\"refresh\" content=\"2\">", "")).unwrap();
        }
    }

    #[test]
    fn the_desktop_key_fingerprint_is_shown_only_when_there_is_one() {
        let line = "Desktop app key: <strong class=\"key\">4821 9930</strong>";
        assert!(email_form_page("Reins desktop", "127.0.0.1", "s", None, Some("4821 9930")).contains(line));
        assert!(wait_page("Reins desktop", 47, Some("4821 9930")).contains(line));
        for page in [email_form_page("Claude", "claude.ai", "s", None, None), wait_page("Claude", 47, None)] {
            assert!(!page.contains("Desktop app key"), "{page}");
        }
        let hostile = wait_page("x", 47, Some(EVIL));
        assert!(!hostile.contains("<script") && hostile.contains("&lt;script&gt;"), "{hostile}");
    }

    #[test]
    fn the_openai_challenge_is_the_bare_token_or_nothing() {
        assert_eq!(openai_challenge_body("  abc-123_XYZ \n").as_deref(), Some("abc-123_XYZ"));
        assert_eq!(openai_challenge_body(""), None);
        assert_eq!(openai_challenge_body("two words"), None);
        assert_eq!(openai_challenge_body(EVIL), None);
        assert_eq!(openai_challenge_body(&"x".repeat(513)), None);
    }
}
