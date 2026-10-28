//! A Family profile (Decisions v2 · P7, 08 R7): a work profile whose org is a family. The member joined by an
//! invitation link (`reins-family://<server host>/<secret>`) with a new account on this profile's own core, so nothing
//! of the Personal profile ever reaches the family (08 R6).

use reins_proto::org::{FamilyLeaveRequest, FamilyLeaveView, FamilyMe};

use crate::{CoreError, engine::Engine, phone_api::ApiFailure, session::api_call};

/// The link a guardian sends: `reins-family://<server host>/<secret>`.
pub const LINK_SCHEME: &str = "reins-family://";

/// What a family can see, as fixed words (the "What your family sees" screen).
pub const SEES: [&str; 3] =
    ["Requests from your agents in this profile", "What guardians decided", "Not your Personal profile"];

/// A request to leave, as the member sees theirs.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FamilyLeaveInfo {
    pub requested_at: i64,
    /// When they go; `None` while a minor's request waits for a guardian.
    pub effective_at: Option<i64>,
    pub needs_consent: bool,
}

/// A member's request to leave, as a guardian sees it (a minor's waits for their Allow).
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FamilyLeaveAsk {
    pub member: String,
    pub email: String,
    pub minor: bool,
    pub requested_at: i64,
    pub effective_at: Option<i64>,
}

/// The family, as this member's phone shows it.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FamilyView {
    pub org_id: String,
    pub name: String,
    pub minor: bool,
    pub guardian: bool,
    pub guardians: Vec<String>,
    /// An adult's notice before leaving: 0, 3 or 7 days.
    pub notice_days: u8,
    pub leave: Option<FamilyLeaveInfo>,
    /// A guardian asked to mark this account a minor's: it counts only if this member accepts.
    pub minor_proposed: bool,
    /// What the family sees ([`SEES`]).
    pub sees: Vec<String>,
    /// For a guardian: requests to leave.
    pub leaves: Vec<FamilyLeaveAsk>,
}

fn leave(l: &FamilyLeaveView) -> FamilyLeaveInfo {
    FamilyLeaveInfo {
        requested_at: l.requested_at,
        effective_at: l.effective_at,
        needs_consent: l.needs_consent,
    }
}

fn ask(l: FamilyLeaveRequest) -> FamilyLeaveAsk {
    FamilyLeaveAsk {
        member: l.member,
        email: l.email,
        minor: l.minor,
        requested_at: l.requested_at,
        effective_at: l.effective_at,
    }
}

impl From<FamilyMe> for FamilyView {
    fn from(me: FamilyMe) -> Self {
        Self {
            org_id: me.org_id,
            name: me.name,
            minor: me.minor,
            guardian: me.guardian,
            guardians: me.guardians,
            notice_days: me.notice_days,
            leave: me.leave.as_ref().map(leave),
            minor_proposed: me.minor_proposed,
            sees: SEES.iter().map(|s| (*s).to_owned()).collect(),
            leaves: me.leaves.into_iter().map(ask).collect(),
        }
    }
}

/// The secret of an invitation link (`reins-family://host/secret`, or the bare secret). The host, when given, must be
/// this profile's server.
pub fn token_of(link: &str, server: &str) -> Result<String, CoreError> {
    let link = link.trim();
    let rest = link.strip_prefix(LINK_SCHEME).unwrap_or(link);
    let (host, token) = match rest.rsplit_once('/') {
        Some((host, token)) => (Some(host), token),
        None => (None, rest),
    };
    if let Some(host) = host {
        let ours = server.trim_end_matches('/').rsplit("://").next().unwrap_or(server);
        if !host.eq_ignore_ascii_case(ours) {
            return Err(CoreError::invalid("This invitation is for another Reins server."));
        }
    }
    if token.len() != 43 || !token.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        return Err(CoreError::invalid("This isn't a family invitation."));
    }
    Ok(token.to_owned())
}

fn family_error(e: ApiFailure) -> CoreError {
    match e {
        // The server's own words for a refused invitation ("Use a new address for your family account.").
        ApiFailure::Status {
            status: 400,
            message,
            ..
        } if !message.is_empty() => CoreError::invalid(message),
        ApiFailure::Status {
            status: 404,
            ..
        } => CoreError::NotFound,
        other => other.into_core(),
    }
}

impl Engine {
    /// Joins the family whose invitation `link` is, with this (new) account.
    pub async fn family_join(&self, link: &str) -> Result<FamilyView, CoreError> {
        let session = self.session()?;
        let token = token_of(link, session.server.as_str())?;
        api_call!(&session, |api| api.family_join(&token)).map(FamilyView::from).map_err(family_error)
    }

    /// The family of this account; `None` when it isn't a family's member.
    pub async fn family_view(&self) -> Result<Option<FamilyView>, CoreError> {
        let session = self.session()?;
        Ok(api_call!(&session, |api| api.family_me()).map_err(family_error)?.map(FamilyView::from))
    }

    pub async fn family_request_leave(&self) -> Result<FamilyLeaveInfo, CoreError> {
        let session = self.session()?;
        api_call!(&session, |api| api.family_leave()).map(|l| leave(&l)).map_err(family_error)
    }

    pub async fn family_cancel_leave(&self) -> Result<(), CoreError> {
        let session = self.session()?;
        api_call!(&session, |api| api.family_stay()).map_err(family_error)
    }

    pub async fn family_consent_leave(&self, member: &str) -> Result<(), CoreError> {
        let session = self.session()?;
        api_call!(&session, |api| api.family_consent(member)).map_err(family_error)
    }

    pub async fn family_answer_minor(&self, accept: bool) -> Result<(), CoreError> {
        let session = self.session()?;
        api_call!(&session, |api| api.family_minor(accept)).map_err(family_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_carry_their_server_and_a_secret() {
        let token = "A".repeat(43);
        assert_eq!(token_of(&format!("reins-family://reins.site/{token}"), "https://reins.site").unwrap(), token);
        assert_eq!(token_of(&token, "https://reins.site").unwrap(), token);
        assert!(token_of(&format!("reins-family://evil.example/{token}"), "https://reins.site").is_err());
        assert!(token_of("reins-family://reins.site/short", "https://reins.site").is_err());
    }

    #[test]
    fn the_view_says_what_the_family_sees() {
        let v = FamilyView::from(FamilyMe {
            org_id: "f1".into(),
            name: "The Lees".into(),
            minor: true,
            guardian: false,
            guardians: vec!["mom@example.com".into()],
            notice_days: 3,
            leave: Some(FamilyLeaveView {
                requested_at: 5,
                effective_at: None,
                needs_consent: true,
            }),
            minor_proposed: false,
            leaves: Vec::new(),
        });
        assert_eq!(v.sees, SEES);
        assert!(v.leave.unwrap().needs_consent);
    }
}
