//! The signed web approval's pages (O3): the card with Approve and Deny, and passkeys. The console's look; values are
//! escaped; the only script is the WebAuthn call (`navigator.credentials`), with every value it needs in data
//! attributes.

#![allow(clippy::format_push_string, reason = "small server-rendered pages")]

use super::console_pages::{icon, page, svg};
use crate::{api::reins::pages::escape_html as e, db::models::reins_org_routing::ReinsOrgWebPasskey};

/// What the card shows.
#[derive(Clone, Debug)]
pub struct Card {
    pub org_name: String,
    pub requester: String,
    pub connection: String,
    pub lines: Vec<String>,
    pub needs: Vec<String>,
    pub progress: String,
    pub level: String,
    pub reason: Option<String>,
    pub expires_at: i64,
    /// The time the challenges sign (the server's).
    pub at: i64,
    pub approve_challenge: String,
    pub deny_challenge: String,
    /// The member's confirmed passkeys (credential ids).
    pub passkeys: Vec<String>,
    pub rp_id: String,
}

const B64: &str = "const b64u=b=>btoa(String.fromCharCode(...new Uint8Array(b))).replace(/\\+/g,'-')\
.replace(/\\//g,'_').replace(/=+$/,'');const ub=s=>Uint8Array.from(atob(s.replace(/-/g,'+').replace(/_/g,'/')),\
c=>c.charCodeAt(0));";

fn when(t: i64) -> String {
    chrono::DateTime::from_timestamp(t, 0).map(|d| d.format("%H:%M UTC").to_string()).unwrap_or_default()
}

/// A page with one line.
#[must_use]
pub fn message(base: &str, text: &str) -> String {
    page(text, None, &format!("<section><h1>{}</h1></section>", e(text)), base)
}

/// After answering.
#[must_use]
pub fn done(base: &str, approved: bool) -> String {
    let (title, path) = if approved {
        ("Approved", icon::CHECK)
    } else {
        ("Denied", icon::WARNING)
    };
    page(
        title,
        None,
        &format!("<section><h1 class=\"row\" style=\"justify-content:flex-start\">{}{title}</h1></section>", svg(path)),
        base,
    )
}

/// The review card.
#[must_use]
pub fn card(base: &str, csrf: &str, token: &str, c: &Card) -> String {
    let lines: String = c.lines.iter().map(|l| format!("<div>{}</div>", e(l))).collect();
    let needs: String = c.needs.iter().map(|n| format!("<span class=\"pill v\">{}</span>", e(n))).collect();
    let progress = if c.progress.is_empty() {
        String::new()
    } else {
        format!("<span class=\"pill\">{}</span>", e(&c.progress))
    };
    let reason = c.reason.as_deref().map(|r| format!("<p class=\"meta\">{}</p>", e(r))).unwrap_or_default();
    let keys = serde_json::to_string(&c.passkeys).unwrap_or_default();
    let body = if c.passkeys.is_empty() {
        format!("<p><a class=\"button\" href=\"{}passkeys/new\">Add a passkey</a></p>", e(base))
    } else {
        format!(
            "<form id=\"f\" method=\"post\" action=\"{b}approve/{t}\" data-approve=\"{a}\" data-deny=\"{d}\" \
data-rp=\"{rp}\" data-keys=\"{k}\"><input type=\"hidden\" name=\"csrf\" value=\"{csrf}\">\
<input type=\"hidden\" name=\"at\" value=\"{at}\"><input type=\"hidden\" name=\"decision\">\
<input type=\"hidden\" name=\"credential_id\"><input type=\"hidden\" name=\"authenticator_data\">\
<input type=\"hidden\" name=\"client_data_json\"><input type=\"hidden\" name=\"signature\">\
<button type=\"button\" data-d=\"approve\">{check}Approve</button>\
<button type=\"button\" class=\"quiet\" data-d=\"deny\">Deny</button></form><p class=\"meta\" id=\"err\"></p>\
<script>{B64}const f=document.getElementById('f');for(const b of f.querySelectorAll('button'))b.onclick=async()=>{{\
try{{const d=b.dataset.d;const c=await navigator.credentials.get({{publicKey:{{challenge:ub(f.dataset[d]),\
rpId:f.dataset.rp,userVerification:'required',timeout:60000,allowCredentials:JSON.parse(f.dataset.keys)\
.map(id=>({{type:'public-key',id:ub(id)}}))}}}});f.decision.value=d;f.credential_id.value=b64u(c.rawId);\
f.authenticator_data.value=b64u(c.response.authenticatorData);f.client_data_json.value=b64u(c.response.clientDataJSON);\
f.signature.value=b64u(c.response.signature);f.submit();}}catch(x){{document.getElementById('err').textContent=\
'Not signed. Try again.';}}}};</script>",
            b = e(base),
            t = e(token),
            a = e(&c.approve_challenge),
            d = e(&c.deny_challenge),
            rp = e(&c.rp_id),
            k = e(&keys),
            csrf = e(csrf),
            at = c.at,
            check = svg(icon::CHECK),
        )
    };
    let html = format!(
        "<section><div class=\"meta\">{org} · {who}{via}</div><h1>{first}</h1><div class=\"record\">{lines}</div>\
{reason}<p>{needs}{progress}<span class=\"pill\">{level}</span><span class=\"meta\">Due {due}</span></p>{body}</section>",
        org = e(&c.org_name),
        who = e(&c.requester),
        via = if c.connection.is_empty() {
            String::new()
        } else {
            format!(" · {}", e(&c.connection))
        },
        first = e(c.lines.first().map_or("Review", String::as_str)),
        level = e(&c.level),
        due = when(c.expires_at),
    );
    page("Review", None, &html, base)
}

/// The member's passkeys, and adding one.
#[must_use]
pub fn passkeys(
    base: &str,
    csrf: &str,
    email: &str,
    user: &str,
    challenge: &str,
    rp_id: &str,
    keys: &[ReinsOrgWebPasskey],
) -> String {
    let rows: String = keys
        .iter()
        .map(|k| {
            let state = if k.confirmed_at.is_some() {
                "<span class=\"pill ok\">Confirmed</span>"
            } else {
                "<span class=\"pill\">Confirm on your phone</span>"
            };
            format!(
                "<tr><td class=\"mono\">{id}</td><td>{state}</td><td><form method=\"post\" action=\"{b}passkeys/{id}/revoke\">\
<input type=\"hidden\" name=\"csrf\" value=\"{csrf}\"><button class=\"danger\" type=\"submit\">Remove</button></form></td></tr>",
                id = e(k.credential_id.get(..16).unwrap_or(&k.credential_id)),
                b = e(base),
                csrf = e(csrf),
            )
        })
        .collect();
    let exclude = serde_json::to_string(&keys.iter().map(|k| &k.credential_id).collect::<Vec<_>>()).unwrap_or_default();
    let user_id = data_encoding::BASE64URL_NOPAD.encode(user.as_bytes());
    let html = format!(
        "<section><h1>Passkeys</h1><p class=\"meta\">Device-bound only.</p><table>{rows}</table>\
<form id=\"p\" method=\"post\" action=\"{b}passkeys/new\" data-ch=\"{ch}\" data-rp=\"{rp}\" data-user=\"{uid}\" \
data-name=\"{email}\" data-ex=\"{ex}\"><input type=\"hidden\" name=\"csrf\" value=\"{csrf}\">\
<input type=\"hidden\" name=\"credential_id\"><input type=\"hidden\" name=\"client_data_json\">\
<input type=\"hidden\" name=\"attestation_object\"><button type=\"button\" id=\"add\">Add passkey</button></form>\
<p class=\"meta\" id=\"err\"></p><script>{B64}const p=document.getElementById('p');document.getElementById('add')\
.onclick=async()=>{{try{{const c=await navigator.credentials.create({{publicKey:{{challenge:ub(p.dataset.ch),\
rp:{{id:p.dataset.rp,name:'Reins'}},user:{{id:ub(p.dataset.user),name:p.dataset.name,displayName:p.dataset.name}},\
pubKeyCredParams:[{{type:'public-key',alg:-7}}],authenticatorSelection:{{userVerification:'required',\
residentKey:'discouraged'}},attestation:'none',timeout:60000,excludeCredentials:JSON.parse(p.dataset.ex)\
.map(id=>({{type:'public-key',id:ub(id)}}))}}}});p.credential_id.value=b64u(c.rawId);\
p.client_data_json.value=b64u(c.response.clientDataJSON);p.attestation_object.value=b64u(c.response.attestationObject);\
p.submit();}}catch(x){{document.getElementById('err').textContent='Not added. Try again.';}}}};</script></section>",
        b = e(base),
        ch = e(challenge),
        rp = e(rp_id),
        uid = e(&user_id),
        email = e(email),
        ex = e(&exclude),
        csrf = e(csrf),
    );
    page("Passkeys", None, &html, base)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card_of(lines: Vec<String>) -> Card {
        Card {
            org_name: "<Acme>".into(),
            requester: "lee@acme.com".into(),
            connection: "Claude".into(),
            lines,
            needs: vec!["Group \"sec\"".into()],
            progress: String::new(),
            level: "L2".into(),
            reason: Some("<script>alert(1)</script>".into()),
            expires_at: 0,
            at: 5,
            approve_challenge: "a\"b".into(),
            deny_challenge: "d".into(),
            passkeys: vec!["k1".into()],
            rp_id: "reins.example".into(),
        }
    }

    #[test]
    fn values_are_escaped() {
        let html = card("/reins/org/", "c\"s", "tok", &card_of(vec!["Create an issue in <b>acme/api</b>".into()]));
        assert!(!html.contains("<b>acme") && html.contains("&lt;b&gt;acme"));
        assert!(!html.contains("<script>alert") && !html.contains("<Acme>"));
        assert!(html.contains("a&quot;b") && html.contains("c&quot;s"));
        // The only script is the WebAuthn one.
        assert_eq!(html.matches("<script>").count(), 1);
        assert!(html.contains("navigator.credentials.get"));
    }

    #[test]
    fn no_passkey_offers_adding_one() {
        let mut c = card_of(vec!["x".into()]);
        c.passkeys.clear();
        let html = card("/reins/org/", "c", "tok", &c);
        assert!(html.contains("passkeys/new") && !html.contains("<script>"));
    }
}
