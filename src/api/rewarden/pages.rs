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
@media (prefers-color-scheme:dark){body{background:#12141a;color:#e8eaf0}\
main{background:#1c1f27;box-shadow:none}.muted{color:#9aa3b5}\
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
        "<h1>Connect {name} to Rewarden</h1>\
<p><strong>{name}</strong> ({host}) wants to use the tools you connect in Rewarden. \
Nothing is shared until you approve it on your phone.</p>{key_html}{error_html}\
<form method=\"post\" action=\"authorize\">\
<input type=\"hidden\" name=\"session\" value=\"{session}\">\
<label for=\"email\">Your account email</label>\
<input id=\"email\" name=\"email\" type=\"email\" autocomplete=\"email\" required autofocus>\
<button type=\"submit\">Continue</button></form>\
<p class=\"muted\">You will get a request in the Rewarden app on your phone.</p>",
        name = escape_html(client_name),
        host = escape_html(client_host),
        session = escape_html(session),
    );
    layout("Connect to Rewarden", "", &body)
}

/// Step 2: show the code to pick on the phone; reloads itself until the phone has answered.
pub fn wait_page(client_name: &str, code: u8, key_fingerprint: Option<&str>) -> String {
    let body = format!(
        "<h1>Approve on your phone</h1>\
<p>Open the Rewarden app and tap this number to connect <strong>{name}</strong>:</p>\
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
    fn the_desktop_key_fingerprint_is_shown_only_when_there_is_one() {
        let line = "Desktop app key: <strong class=\"key\">4821 9930</strong>";
        assert!(email_form_page("Rewarden desktop", "127.0.0.1", "s", None, Some("4821 9930")).contains(line));
        assert!(wait_page("Rewarden desktop", 47, Some("4821 9930")).contains(line));
        for page in [email_form_page("Claude", "claude.ai", "s", None, None), wait_page("Claude", 47, None)] {
            assert!(!page.contains("Desktop app key"), "{page}");
        }
        let hostile = wait_page("x", 47, Some(EVIL));
        assert!(!hostile.contains("<script") && hostile.contains("&lt;script&gt;"), "{hostile}");
    }
}
