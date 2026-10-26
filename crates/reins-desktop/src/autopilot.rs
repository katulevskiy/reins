//! The phone's Autopilot from this computer: its modes as the server last heard them (`GET /reins/desktop/phone`), a
//! change of mode (`desktop_autopilot_mode`) and "This was wrong" (`desktop_autopilot_correct`).
//!
//! This computer never makes Reins more permissive on its own say: it only asks. The phone applies at once what is
//! stricter (Manual from Auto, Lockdown, ending a bypass) and asks its owner, held to approve, for anything looser
//! (Auto, a bypass, leaving Lockdown), the same as any other request. [`is_stricter`] is only how the app words the
//! button; the phone decides with its own state.

use reins_proto::autopilot::{DesktopModes, Mode};
use serde_json::{Value, json};

use crate::auth::Refusal;
use crate::config::{Config, Paths};
use crate::journal::{Entry, Kind};
use crate::phone::awaiting;
use crate::server::LinkError;
use crate::server::client::DesktopClient;

pub const MODE_TOOL: &str = "desktop_autopilot_mode";
pub const CORRECT_TOOL: &str = "desktop_autopilot_correct";

/// A change of the phone's global mode, as this computer may ask it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeChoice {
    Manual,
    Auto,
    Lockdown,
    /// Everything approved for this many minutes (1 to a day).
    Bypass(u16),
    /// Ends the running bypass or Overnight.
    Stop,
}

impl ModeChoice {
    fn arguments(self) -> Value {
        match self {
            Self::Manual => json!({"mode": "manual"}),
            Self::Auto => json!({"mode": "auto"}),
            Self::Lockdown => json!({"mode": "lockdown"}),
            Self::Bypass(minutes) => json!({"mode": "bypass", "minutes": minutes}),
            Self::Stop => json!({"mode": "stop"}),
        }
    }

    /// One line for the activity log.
    #[must_use]
    pub fn what(self) -> String {
        match self {
            Self::Manual => "Autopilot: Manual".to_owned(),
            Self::Auto => "Autopilot: Auto".to_owned(),
            Self::Lockdown => "Autopilot: Lockdown".to_owned(),
            Self::Bypass(minutes) => {
                format!("Autopilot: Bypass for {}", crate::work_session::span(u64::from(minutes) * 60))
            }
            Self::Stop => "Autopilot: end the bypass".to_owned(),
        }
    }
}

/// Whether the phone will apply `choice` at once (it only makes Reins stricter) rather than ask its owner, judging by
/// the modes it last reported. Unknown modes: asked.
#[must_use]
pub fn is_stricter(choice: ModeChoice, modes: Option<&DesktopModes>) -> bool {
    match choice {
        ModeChoice::Lockdown | ModeChoice::Stop => true,
        ModeChoice::Bypass(_) => false,
        ModeChoice::Manual => modes.is_some_and(|m| m.base.openness() >= Mode::Manual.openness()),
        ModeChoice::Auto => modes.is_some_and(|m| m.base.openness() >= Mode::Auto.openness()),
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
    }
}

/// Asks the phone for `tool` with `arguments`, logged as `entry`; `Ok` once the phone applied it.
async fn ask(paths: &Paths, config: &Config, tool: &str, arguments: Value, entry: Entry) -> Result<(), String> {
    let link = crate::work_session::phone_link(paths, config)?;
    let phone = link.phone().map_err(|r| refusal_text(&r))?;
    let answer = awaiting(
        link.journal(),
        entry,
        None,
        link.timeout(),
        phone.ask(tool, None, |_| arguments.clone(), "No answer on your phone in time."),
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
/// approval on the phone (up to the approval wait).
pub async fn change_mode(paths: &Paths, config: &Config, choice: ModeChoice) -> Result<(), String> {
    let entry = Entry::new(Kind::Ask, &choice.what()).source(Some("Reins")).service(Some("Autopilot"));
    ask(paths, config, MODE_TOOL, choice.arguments(), entry).await
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
    ask(paths, config, CORRECT_TOOL, json!({"request_id": request_id, "should_have": should}), entry).await
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
        assert!(!is_stricter(ModeChoice::Bypass(30), Some(&modes(Mode::Auto))));
        assert!(is_stricter(ModeChoice::Manual, Some(&modes(Mode::Auto))));
        assert!(!is_stricter(ModeChoice::Manual, Some(&modes(Mode::Lockdown))), "leaving Lockdown is asked");
        assert!(!is_stricter(ModeChoice::Auto, Some(&modes(Mode::Manual))));
        assert!(!is_stricter(ModeChoice::Manual, None), "not known: asked");
        assert_eq!(ModeChoice::Bypass(90).arguments(), json!({"mode": "bypass", "minutes": 90}));
        let spec = reins_proto::connector::spec_for_tool(MODE_TOOL).expect("the phone's tool");
        for choice in
            [ModeChoice::Manual, ModeChoice::Auto, ModeChoice::Lockdown, ModeChoice::Bypass(5), ModeChoice::Stop]
        {
            let mut args = choice.arguments();
            args["client_key"] = json!(crate::identity::Identity::generate().public_key());
            args["nonce"] = json!("n1");
            spec.parse(&args).unwrap_or_else(|e| panic!("{choice:?}: {e}"));
        }
        assert!(reins_proto::connector::spec_for_tool(CORRECT_TOOL).is_some_and(|s| s.desktop_only));
    }
}
