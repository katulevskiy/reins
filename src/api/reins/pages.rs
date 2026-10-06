//! Server-rendered HTML for the OAuth authorize flow (Decision 18): no JavaScript, every
//! dynamic value HTML-escaped, relative form/refresh URLs so the pages work under any
//! `DOMAIN` path prefix.

const STYLE: &str = "body{font-family:system-ui,-apple-system,Segoe UI,Roboto,sans-serif;margin:0;min-height:100vh;\
display:flex;align-items:center;justify-content:center;background:#f4f5f7;color:#16181d}\
main{background:#fff;max-width:26rem;width:100%;margin:1rem;padding:2rem;border-radius:1rem;\
box-shadow:0 4px 24px rgba(0,0,0,.08)}\
h1{font-size:1.3rem;margin:.2rem 0 1rem}p{line-height:1.45;margin:.6rem 0}\
.muted{color:#5d6675;font-size:.9rem}.code{font-size:3.5rem;font-weight:700;letter-spacing:.2rem;\
text-align:center;margin:1.2rem 0;font-variant-numeric:tabular-nums}\
.key{font-size:1.15rem;letter-spacing:.08rem;font-variant-numeric:tabular-nums}\
input[type=email]{width:100%;box-sizing:border-box;padding:.75rem;font-size:1rem;border:1px solid #c5cbd6;\
border-radius:.5rem;margin:.5rem 0 1rem}\
button{width:100%;padding:.8rem;font-size:1rem;font-weight:600;border:0;border-radius:.5rem;\
background:#175ddc;color:#fff;cursor:pointer}\
.error{background:#fdecea;color:#8a1f17;padding:.7rem .9rem;border-radius:.5rem}\
a.button{display:block;box-sizing:border-box;text-align:center;text-decoration:none;padding:.8rem;\
font-weight:600;border-radius:.5rem;background:#175ddc;color:#fff;margin:1rem 0}\
.pcode{font-size:2rem;font-weight:700;letter-spacing:.15rem;text-align:center;margin:1rem 0;\
font-family:ui-monospace,SFMono-Regular,Menlo,monospace}\
@media (prefers-color-scheme:dark){body{background:#12141a;color:#e8eaf0}\
main{background:#1c1f27;box-shadow:none}.muted{color:#9aa3b5}a{color:#8db4ff}\
input[type=email]{background:#12141a;color:#e8eaf0;border-color:#3a3f4c}.error{background:#3a1613;color:#f5b5ae}}";

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

fn layout(title: &str, head_extra: &str, body: &str) -> String {
    format!(
        "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
<meta name=\"robots\" content=\"noindex,nofollow\">{head_extra}<title>{title}</title>\
<style>{STYLE}</style></head><body><main>{body}</main></body></html>",
        title = escape_html(title),
    )
}

/// The desktop app's key fingerprint, which the user compares with the terminal and the phone before approving.
fn key_line(fingerprint: Option<&str>) -> String {
    fingerprint
        .map(|f| {
            format!(
                "<p>Desktop app key: <strong class=\"key\">{}</strong></p>\
<p class=\"muted\">Your computer and your phone show the same key. Approve only if they match.</p>",
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
        "<h1>Connect {name} to Reins</h1>\
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
    layout("Connect to Reins", "", &body)
}

/// Step 2: show the code to pick on the phone; reloads itself until the phone has answered.
pub fn wait_page(client_name: &str, code: u8, key_fingerprint: Option<&str>) -> String {
    let body = format!(
        "<h1>Approve on your phone</h1>\
<p>Open the Reins app and tap this number to connect <strong>{name}</strong>:</p>\
<div class=\"code\" aria-label=\"code\">{code}</div>{key_html}\
<p class=\"muted\">This page continues automatically. If the number is not offered on your phone, \
deny the request there.</p>",
        name = escape_html(client_name),
        key_html = key_line(key_fingerprint),
    );
    layout("Approve on your phone", "<meta http-equiv=\"refresh\" content=\"2\">", &body)
}

/// A terminal error the user cannot recover from on this page.
pub fn error_page(title: &str, message: &str) -> String {
    let body = format!("<h1>{}</h1><p>{}</p>", escape_html(title), escape_html(message));
    layout(title, "", &body)
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
        "<p class=\"muted\">No Reins app on this phone yet? \
<a href=\"{phone}\">Get the phone app</a>, sign in, then scan the code again. \
The download page shows availability for your phone.</p>",
        phone = escape_html(MOBILE_APP_URL),
    );
    let body = match code {
        Some(code) => format!(
            "<h1>Connect your computer</h1>\
<p>Your computer shows this code. Open it in the Reins app on your phone to connect the computer to your account:</p>\
<div class=\"pcode\" aria-label=\"pairing code\">{code}</div>\
<a class=\"button\" href=\"{open}\">Open in Reins</a>{install}\
<p class=\"muted\">On a computer? Scan the QR code with your phone's camera instead.</p>",
            code = escape_html(code),
            open = escape_html(&open_in_app_link(code, platform)),
        ),
        None => format!(
            "<h1>Connect your computer</h1>\
<p>Scan the QR code your computer shows with your phone's camera, or in the Reins app: \
<strong>Settings</strong>, <strong>Connect a computer</strong>.</p>{install}"
        ),
    };
    layout("Connect your computer to Reins", "", &body)
}

/// `/.well-known/apple-app-site-association`: pairing links under `{domain_path}/pair` open the iOS app of `team_id`
/// (none when it is empty). `bitwarden_credentials` keeps what the web vault serves there (the Bitwarden apps'
/// shared web credentials).
pub fn apple_app_site_association(team_id: &str, domain_path: &str, bitwarden_credentials: bool) -> serde_json::Value {
    let team = team_id.trim();
    let mut doc = serde_json::json!({});
    if !team.is_empty() {
        let path = format!("{}/pair", domain_path.trim_end_matches('/'));
        doc["applinks"] = serde_json::json!({
            "details": [{
                "appIDs": [format!("{team}.{APP_ID}")],
                "components": [{"/": path, "comment": "A computer's pairing code (QR code)"}]
            }]
        });
    }
    if bitwarden_credentials {
        doc["webcredentials"] =
            serde_json::json!({"apps": ["LTZ2PFU5D6.com.8bit.bitwarden", "LTZ2PFU5D6.com.8bit.bitwarden.beta"]});
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

/// `/.well-known/assetlinks.json`: pairing links open the Android app signed with one of `fingerprints` (an empty
/// list when there are none: links then open the page).
pub fn asset_links(fingerprints: &[String]) -> serde_json::Value {
    if fingerprints.is_empty() {
        return serde_json::json!([]);
    }
    serde_json::json!([{
        "relation": ["delegate_permission/common.handle_all_urls"],
        "target": {"namespace": "android_app", "package_name": APP_ID, "sha256_cert_fingerprints": fingerprints}
    }])
}

/// Routes mounted at `{domain_path}/`.
pub fn routes() -> Vec<rocket::Route> {
    routes![pair, signed_out]
}

/// AuthKit's logout landing page never initiates a new sign-in. The explicit link also works in Firefox,
/// which may require a user gesture to hand a custom scheme back to the native app.
#[get("/reins/signed-out")]
fn signed_out() -> rocket::response::content::RawHtml<String> {
    rocket::response::content::RawHtml(layout(
        "Signed out",
        "",
        "<h1>Signed out</h1><p>Your browser sign-in session has ended.</p>\
<a class=\"button\" href=\"com.reins2fa.app://signed-out\">Return to Reins</a>\
<p class=\"muted\">You can also close this page and return to the app.</p>",
    ))
}

/// Routes mounted at the server root `/`, where the phones look for them.
pub fn app_link_routes() -> Vec<rocket::Route> {
    let (_, bad) = android_cert_fingerprints(&crate::CONFIG.reins_android_cert_sha256());
    for item in bad {
        warn!("`REINS_ANDROID_CERT_SHA256`: `{item}` is not a SHA-256 fingerprint (32 bytes of hex); ignored");
    }
    routes![apple_app_site_association_doc, asset_links_doc]
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
            error_page(EVIL, EVIL),
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
            error_page("t", "m"),
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
        assert!(aasa.get("webcredentials").is_none());
        let under = apple_app_site_association(" T ", "/vw/", true);
        assert_eq!(under["applinks"]["details"][0]["components"][0]["/"], "/vw/pair");
        assert!(under["webcredentials"]["apps"][0].as_str().unwrap().ends_with("com.8bit.bitwarden"));
        assert!(apple_app_site_association("", "", false).get("applinks").is_none(), "no team id, no app links");

        let hex = "146de983c5730650d8eeb9952f34fc6416a08342e61dbea88a0496b23fcf44e5";
        let colons = "14:6D:E9:83:C5:73:06:50:D8:EE:B9:95:2F:34:FC:64:16:A0:83:42:E6:1D:BE:A8:8A:04:96:B2:3F:CF:44:E5";
        let (good, bad) = android_cert_fingerprints(&format!(" {hex} , {colons},nope,"));
        assert_eq!(good, [colons, colons]);
        assert_eq!(bad, ["nope"]);
        let links = asset_links(&good[..1]);
        assert_eq!(links[0]["target"]["package_name"], "com.reins2fa.app");
        assert_eq!(links[0]["target"]["sha256_cert_fingerprints"], serde_json::json!([colons]));
        assert_eq!(links[0]["relation"], serde_json::json!(["delegate_permission/common.handle_all_urls"]));
        assert_eq!(asset_links(&[]), serde_json::json!([]));
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
}
