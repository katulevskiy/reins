//! The phone's Autopilot from this computer: its modes as the server last heard them (`GET /reins/desktop/phone`), a
//! change of mode (`desktop_autopilot_mode`) and "This was wrong" (`desktop_autopilot_correct`).
//!
//! This computer never makes Reins more permissive on its own say: it only asks. The phone applies at once what is
//! stricter (Manual from Auto, Lockdown, ending a bypass or Overnight) and asks its owner, held to approve, for
//! anything looser (Auto, leaving Lockdown). A looser ask carries this app's own word for exactly that call (a
//! [`DesktopAsk`] boxed from its key to the phone key it pinned), so the server cannot make one up. A bypass and
//! Overnight start on the phone only. [`is_stricter`] is only how the app words the button; the phone decides with
//! its own state.

use reins_proto::autopilot::{ASK_DIR, DesktopAsk, DesktopModes, Mode};
use serde_json::{Value, json};

use crate::auth::Refusal;
use crate::config::{Config, Paths};
use crate::journal::{Entry, Kind};
use crate::phone::awaiting;
use crate::server::LinkError;
use crate::server::client::DesktopClient;

pub const MODE_TOOL: &str = "desktop_autopilot_mode";
pub const CORRECT_TOOL: &str = "desktop_autopilot_correct";
const MODE_OP: &str = "autopilot_mode";
const CORRECT_OP: &str = "autopilot_correct";

/// What a looser ask needs first: the phone's key on this computer.
pub const NEEDS_PHONE_KEY: &str = "This needs your phone's key on this computer first: run `reins vault list` once \
     and type the twelve digits your phone shows.";

/// A change of the phone's global mode, as this computer may ask it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeChoice {
    Manual,
    Auto,
    Lockdown,
    /// Ends the running bypass or Overnight.
    Stop,
}

impl ModeChoice {
    fn arguments(self) -> Value {
        match self {
            Self::Manual => json!({"mode": "manual"}),
            Self::Auto => json!({"mode": "auto"}),
            Self::Lockdown => json!({"mode": "lockdown"}),
            Self::Stop => json!({"mode": "stop"}),
        }
    }

    /// One line for the activity log.
    #[must_use]
    pub fn what(self) -> &'static str {
        match self {
            Self::Manual => "Autopilot: Manual",
            Self::Auto => "Autopilot: Auto",
            Self::Lockdown => "Autopilot: Lockdown",
            Self::Stop => "Autopilot: end the bypass or Overnight",
        }
    }
}

/// Whether the phone will apply `choice` at once (it only makes Reins stricter) rather than ask its owner, judging by
/// the modes it last reported. Unknown modes, and Auto over an Overnight (which may return to Manual): asked.
#[must_use]
pub fn is_stricter(choice: ModeChoice, modes: Option<&DesktopModes>) -> bool {
    match choice {
        ModeChoice::Lockdown | ModeChoice::Stop => true,
        ModeChoice::Manual => modes.is_some_and(|m| m.base != Mode::Lockdown && m.global != Mode::Lockdown),
        ModeChoice::Auto => {
            modes.is_some_and(|m| m.base.openness() >= Mode::Auto.openness() && m.global != Mode::Overnight)
        }
    }
}

/// The phone's modes as this computer is shown them; `None`: not reported yet, or a server too old to say.
pub async fn modes(paths: &Paths) -> Result<Option<DesktopModes>, String> {
    let client = DesktopClient::new(paths)?;
    match client.phone().await {
        Ok(seen) => Ok(seen.modes),
        Err(LinkError::NotFound) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

fn refusal_text(r: &Refusal) -> String {
    match r {
        Refusal::Denied(m) => format!("denied on your phone: {m}"),
        Refusal::Waiting(m) | Refusal::Unavailable(m) => m.clone(),
        Refusal::Parked(p) => p.message.clone(),
    }
}

/// This app's word for the call `op` with `args` and `nonce`, boxed to `phone_key` (see the module docs).
fn proof(identity: &crate::identity::Identity, phone_key: &str, op: &str, args: &Value, nonce: &str) -> Option<String> {
    let ask = DesktopAsk {
        v: 1,
        dir: ASK_DIR.to_owned(),
        op: op.to_owned(),
        args: DesktopAsk::covered(args.as_object()?),
        nonce: nonce.to_owned(),
        created_at: crate::now_unix(),
    };
    identity.box_to(phone_key, &serde_json::to_vec(&ask).ok()?)
}

/// Asks the phone for `op` with `arguments`, logged as `entry`; `Ok` once the phone applied it. `looser`: the phone
/// will ask its owner, so the call carries this app's proof (and needs the phone's key here).
async fn ask(
    paths: &Paths,
    config: &Config,
    op: &str,
    arguments: Value,
    looser: bool,
    entry: Entry,
) -> Result<(), String> {
    let link = crate::work_session::phone_link(paths, config)?;
    let phone = link.phone().map_err(|r| refusal_text(&r))?;
    let phone_key = crate::vault_cli::pinned(paths, phone.server());
    if looser && phone_key.is_none() {
        return Err(NEEDS_PHONE_KEY.to_owned());
    }
    let identity = phone.identity();
    let tool = if op == MODE_OP {
        MODE_TOOL
    } else {
        CORRECT_TOOL
    };
    let with_proof = |nonce: &str| {
        let mut args = arguments.clone();
        if let Some(key) = &phone_key
            && let Some(p) = proof(identity, key, op, &args, nonce)
        {
            args["proof"] = json!(p);
        }
        args
    };
    let answer = awaiting(
        link.journal(),
        entry,
        None,
        link.timeout(),
        phone.ask(tool, None, with_proof, "No answer on your phone in time."),
    )
    .await
    .map_err(|r| refusal_text(&r))?;
    if answer.data.get("applied").and_then(Value::as_bool) == Some(true) {
        Ok(())
    } else {
        Err("the phone's answer does not say it was applied".to_owned())
    }
}

/// Asks the phone to change its global mode. Stricter changes apply at once; looser ones wait for the owner's
/// approval on the phone (up to the approval wait). `modes`: the phone's modes as last shown here.
pub async fn change_mode(
    paths: &Paths,
    config: &Config,
    choice: ModeChoice,
    modes: Option<&DesktopModes>,
) -> Result<(), String> {
    let entry = Entry::new(Kind::Ask, choice.what()).source(Some("Reins")).service(Some("Autopilot"));
    ask(paths, config, MODE_OP, choice.arguments(), !is_stricter(choice, modes), entry).await
}

/// "This was wrong": tells the phone's Autopilot what it should have done with `request_id` (a request of this
/// computer it decided). Teaching it to deny applies at once; teaching it to approve is asked on the phone.
pub async fn correct(
    paths: &Paths,
    config: &Config,
    request_id: &str,
    should_have_approved: bool,
) -> Result<(), String> {
    let should = if should_have_approved {
        "approve"
    } else {
        "deny"
    };
    let entry = Entry::new(Kind::Ask, &format!("Autopilot: should have said {should}"))
        .source(Some("Reins"))
        .service(Some("Autopilot"));
    let args = json!({"request_id": request_id, "should_have": should});
    ask(paths, config, CORRECT_OP, args, should_have_approved, entry).await
}

#[cfg(test)]
mod tests {
    use reins_proto::autopilot::Span;

    use super::*;

    fn modes(base: Mode) -> DesktopModes {
        DesktopModes {
            mode: base,
            span: Span::default(),
            global: base,
            base,
            reported_at: 1,
        }
    }

    #[test]
    fn the_app_words_what_the_phone_applies_at_once_and_what_it_asks() {
        assert!(is_stricter(ModeChoice::Lockdown, None));
        assert!(is_stricter(ModeChoice::Stop, None));
        assert!(is_stricter(ModeChoice::Manual, Some(&modes(Mode::Auto))));
        assert!(!is_stricter(ModeChoice::Manual, Some(&modes(Mode::Lockdown))), "leaving Lockdown is asked");
        assert!(!is_stricter(ModeChoice::Auto, Some(&modes(Mode::Manual))));
        let mut night = modes(Mode::Auto);
        night.global = Mode::Overnight;
        assert!(!is_stricter(ModeChoice::Auto, Some(&night)), "an Overnight may return to Manual");
        assert!(!is_stricter(ModeChoice::Manual, None), "not known: asked");
        let spec = reins_proto::connector::spec_for_tool(MODE_TOOL).expect("the phone's tool");
        let app = crate::identity::Identity::generate();
        let phone = crate::identity::Identity::generate();
        for choice in [ModeChoice::Manual, ModeChoice::Auto, ModeChoice::Lockdown, ModeChoice::Stop] {
            let mut args = choice.arguments();
            args["client_key"] = json!(app.public_key());
            args["nonce"] = json!("n1");
            args["proof"] = json!(proof(&app, &phone.public_key(), MODE_OP, &args, "n1").unwrap());
            spec.parse(&args).unwrap_or_else(|e| panic!("{choice:?}: {e}"));
        }
        assert!(spec.parse(&json!({"mode": "bypass", "client_key": app.public_key(), "nonce": "n"})).is_err());
        // The proof opens with the phone's key from this app's key, for exactly these arguments.
        let args = json!({"mode": "auto", "client_key": app.public_key(), "nonce": "n2"});
        let boxed = proof(&app, &phone.public_key(), MODE_OP, &args, "n2").unwrap();
        let ask: DesktopAsk = serde_json::from_slice(&phone.open_box_from(&app.public_key(), &boxed).unwrap()).unwrap();
        assert_eq!((ask.dir.as_str(), ask.op.as_str(), ask.nonce.as_str()), (ASK_DIR, MODE_OP, "n2"));
        assert_eq!(Value::Object(ask.args), json!({"mode": "auto"}));
        assert!(reins_proto::connector::spec_for_tool(CORRECT_TOOL).is_some_and(|s| s.desktop_only));
    }
}
