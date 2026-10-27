//! A work profile's organization calls (O1): which org the signed-in account belongs to, and enrolling this device
//! to it (06 §6). They run on the work profile's own core (`profiles`): nothing here can read another profile's store,
//! so what an org receives is built only from the work account's session and what the app passes in about the device.

use reins_proto::managed::{
    ReviewAnswer, ReviewDecision, ReviewItem, ReviewSubject, SignedMemberPolicy, change_digest, request_digest,
};
use reins_proto::org::{Attestation, AttestationKind, EnrollmentRequest, KEY_P256, OrgMe, Posture};

use crate::{CoreError, engine::Engine, phone_api::ApiFailure, session::api_call};

/// The work account's organization, as the app shows it.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct OrgView {
    pub org_id: String,
    pub name: String,
    /// `team`, `business`, `enterprise`.
    pub tier: String,
    /// The roles' labels (`Security admin`, …).
    pub roles: Vec<String>,
    /// This device is enrolled (and not removed).
    pub enrolled: bool,
    pub github_orgs: Vec<String>,
}

impl From<OrgMe> for OrgView {
    fn from(me: OrgMe) -> Self {
        Self {
            org_id: me.org_id,
            name: me.name,
            tier: me.tier.as_str().to_owned(),
            roles: me.roles.iter().map(|r| r.label().to_owned()).collect(),
            enrolled: me.enrolled,
            github_orgs: me.routes.github_orgs,
        }
    }
}

/// What the platform measured about the device (06 §6).
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PostureInput {
    pub screen_lock: bool,
    pub os_name: String,
    pub os_version: String,
    /// `None`: the check could not run.
    pub rooted: Option<bool>,
    pub app_version: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum AttestationKindInput {
    AppAttest,
    AndroidKeyAttestation,
    PlayIntegrity,
    None,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct AttestationInput {
    pub kind: AttestationKindInput,
    /// As the platform gave it, base64.
    pub data: String,
}

/// The device key the app made in secure hardware for this org, its signature over
/// [`crate::profiles::org_enrollment_message`] of the challenge, and what vouches for it.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct EnrollInput {
    /// SubjectPublicKeyInfo DER, base64 (P-256).
    pub device_key: String,
    /// ECDSA P-256 SHA-256, DER, base64.
    pub signature: String,
    pub hardware: bool,
    pub attestations: Vec<AttestationInput>,
    pub posture: PostureInput,
}

fn posture(p: PostureInput, now: i64) -> Posture {
    Posture {
        screen_lock: p.screen_lock,
        os_name: p.os_name,
        os_version: p.os_version,
        rooted: p.rooted,
        app_version: p.app_version,
        collected_at: now,
    }
}

/// The request sent to the org: the challenge and the app's inputs, nothing else.
#[must_use]
pub fn enrollment_request(challenge: String, input: EnrollInput, now: i64) -> EnrollmentRequest {
    EnrollmentRequest {
        challenge,
        device_key: input.device_key,
        key_algorithm: KEY_P256.to_owned(),
        hardware: input.hardware,
        attestations: input
            .attestations
            .into_iter()
            .map(|a| Attestation {
                kind: match a.kind {
                    AttestationKindInput::AppAttest => AttestationKind::AppAttest,
                    AttestationKindInput::AndroidKeyAttestation => AttestationKind::AndroidKeyAttestation,
                    AttestationKindInput::PlayIntegrity => AttestationKind::PlayIntegrity,
                    AttestationKindInput::None => AttestationKind::None,
                },
                data: a.data,
            })
            .collect(),
        posture: posture(input.posture, now),
        signature: input.signature,
    }
}

/// What an org review is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum OrgReviewKind {
    /// An agent's request.
    Request,
    /// A pending admin change.
    Change,
}

/// One review waiting for this member, as the phone shows and signs it. `lines` are built on the phone from what
/// the digest covers, never taken from the server.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct OrgReviewView {
    pub id: String,
    pub kind: OrgReviewKind,
    pub org_name: String,
    /// The requester's or proposer's email.
    pub requester: String,
    /// The agent connection's label (requests).
    pub connection: String,
    /// The server's short title.
    pub title: String,
    pub lines: Vec<String>,
    /// Who must approve ("Security", "You").
    pub requires: String,
    pub level: String,
    pub zone: String,
    /// The request or change digest the phone recomputed.
    pub digest: String,
    /// The policy snapshot hash (requests); empty for changes.
    pub snapshot: String,
    pub expires_at: i64,
    /// Your own: your request, or your change to confirm.
    pub own: bool,
}

/// The longest line, title or name a review shows, in characters.
const MAX_SHOWN: usize = 160;

fn shown(s: &str) -> String {
    crate::text::truncate_chars(&crate::text::one_line(s), MAX_SHOWN)
}

/// The decision a review's answer carries: your own change is confirmed with `Propose`, anything else approved or
/// denied.
#[must_use]
pub fn decision_for(review: &OrgReviewView, approve: bool) -> ReviewDecision {
    match (approve, review.kind, review.own) {
        (true, OrgReviewKind::Change, true) => ReviewDecision::Propose,
        (true, ..) => ReviewDecision::Approve,
        (false, ..) => ReviewDecision::Deny,
    }
}

/// What the enrolled device key signs to answer `review` ([`reins_proto::managed::review_message`]).
#[must_use]
pub fn review_message_for(org_id: &str, review: &OrgReviewView, approve: bool, at: i64) -> Vec<u8> {
    let subject = match review.kind {
        OrgReviewKind::Request => ReviewSubject::Request,
        OrgReviewKind::Change => ReviewSubject::Change,
    };
    reins_proto::managed::review_message(
        org_id,
        subject,
        &review.id,
        &review.digest,
        decision_for(review, approve),
        &review.snapshot,
        at,
    )
}

/// The reviews of `org_id` the phone can vouch for: the digest is recomputed from the request (or the change's
/// stored text) and must equal the one the server asks to sign; a request must be this org's. The rest are dropped,
/// with a log line. Lines are built here from the same data the digest covers.
#[must_use]
pub fn review_views(org_id: &str, items: Vec<ReviewItem>) -> Vec<OrgReviewView> {
    items
        .into_iter()
        .filter_map(|item| {
            let id = item.id.clone();
            let view = review_view(org_id, item);
            if view.is_none() {
                log::warn!("org review {id} dropped: it does not match its digest");
            }
            view
        })
        .collect()
}

fn review_view(org_id: &str, item: ReviewItem) -> Option<OrgReviewView> {
    let (kind, digest, lines) = match item.subject {
        ReviewSubject::Request => {
            let r = item.request.as_ref()?;
            if r.org != org_id {
                return None;
            }
            let digest = request_digest(&r.org, &r.member, &r.connection, &r.tool, &r.args);
            let mut lines = reins_policy::org::analyze_request(&r.tool, &r.args).lines();
            if lines.is_empty() {
                lines.push(r.tool.clone());
            }
            (OrgReviewKind::Request, digest, lines)
        }
        ReviewSubject::Change => {
            let text = item.action_json.as_deref()?;
            let digest = change_digest(org_id, &item.id, text);
            (OrgReviewKind::Change, digest, change_lines(text))
        }
    };
    if digest != item.digest {
        return None;
    }
    Some(OrgReviewView {
        id: item.id,
        kind,
        org_name: crate::profiles::profile_label(&item.org_name),
        requester: shown(&item.requester),
        connection: shown(&item.connection),
        title: shown(&item.title),
        lines: lines.iter().map(|l| shown(l)).collect(),
        requires: shown(&item.requires),
        level: shown(&item.level),
        zone: shown(&item.zone),
        digest,
        snapshot: item.snapshot,
        expires_at: item.expires_at,
        own: item.own,
    })
}

/// An admin change in a few words, read from the stored text the digest covers. Read as JSON, not as this build's
/// `AdminAction`, so a kind added later still shows (generically, when unknown).
fn change_lines(text: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
        return vec!["Admin change".to_owned()];
    };
    let s = |k: &str| v.get(k).and_then(serde_json::Value::as_str).unwrap_or_default();
    let short = |id: &str| id.chars().take(8).collect::<String>();
    let role = || {
        serde_json::from_value::<reins_proto::org::Role>(v.get("role").cloned().unwrap_or_default())
            .map_or("a role", reins_proto::org::Role::label)
    };
    let subject = || {
        let sub = v.get("subject");
        let id = sub.and_then(|x| x.get("id")).and_then(serde_json::Value::as_str).unwrap_or_default();
        match sub.and_then(|x| x.get("type")).and_then(serde_json::Value::as_str) {
            Some("group") => format!("group {}", short(id)),
            _ => format!("member {}", short(id)),
        }
    };
    let line = match s("kind") {
        "grant_role" => format!("Grant {} to {}", role(), subject()),
        "revoke_role" => format!("Remove {} from {}", role(), subject()),
        "set_identity" => {
            let kind = serde_json::from_value::<reins_proto::org::IdentityKind>(
                v.get("identity").cloned().unwrap_or_default(),
            )
            .map_or("another provider", reins_proto::org::IdentityKind::label);
            format!("Sign in with {kind}")
        }
        "set_tier" => {
            let tier = serde_json::from_value::<reins_proto::org::Tier>(v.get("tier").cloned().unwrap_or_default())
                .map_or("another plan", reins_proto::org::Tier::label);
            format!("Change the plan to {tier}")
        }
        "remove_domain" => format!("Remove domain {}", s("domain")),
        "remove_member" => format!("Remove member {}", short(s("member"))),
        "set_two_person" => {
            if v.get("on").and_then(serde_json::Value::as_bool) == Some(true) {
                "Turn on two-person approval".to_owned()
            } else {
                "Turn off two-person approval".to_owned()
            }
        }
        "confirm_directory_removals" => "Remove the members the directory held back".to_owned(),
        "activate_policy" => match v.get("revision").and_then(serde_json::Value::as_u64) {
            Some(n) => format!("Activate policy revision {n}"),
            None => "Activate a policy".to_owned(),
        },
        "set_retention" => match v.get("days").and_then(serde_json::Value::as_u64) {
            Some(0) => "Keep no request bodies".to_owned(),
            Some(365) => "Keep request bodies 1 year".to_owned(),
            Some(n) => format!("Keep request bodies {n} days"),
            None => "Change retention".to_owned(),
        },
        "register_executor" => format!("Register executor {} ({})", s("executor_id"), s("mode")),
        "remove_executor" => format!("Remove executor {}", s("executor_id")),
        _ => "Admin change".to_owned(),
    };
    vec![line]
}

/// The member policy as the org receives it, once `body` checks out ([`checked_member_policy`]) and `version` is a
/// version (from 1).
pub fn signed_member_policy(
    body: String,
    version: u64,
    at: i64,
    device: String,
    sig: String,
) -> Result<SignedMemberPolicy, CoreError> {
    checked_member_policy(&body)?;
    if version == 0 {
        return Err(CoreError::invalid("A policy version starts at 1"));
    }
    Ok(SignedMemberPolicy {
        body,
        version,
        at,
        device,
        sig,
    })
}

/// `body` if it is a member policy written as its canonical JSON (what the device signs is what the org stores).
pub fn checked_member_policy(body: &str) -> Result<reins_policy::org::MemberPolicy, CoreError> {
    let invalid = || CoreError::invalid("That policy can't be read");
    let value: serde_json::Value = serde_json::from_str(body).map_err(|_| invalid())?;
    if !value.is_object() || reins_proto::managed::canonical_json(&value) != body {
        return Err(invalid());
    }
    serde_json::from_value(value).map_err(|_| invalid())
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs().try_into().unwrap_or(i64::MAX))
}

fn org_error(e: ApiFailure) -> CoreError {
    match e {
        ApiFailure::Status {
            status: 403,
            message,
            ..
        } => CoreError::invalid(message),
        other => other.into_core(),
    }
}

impl Engine {
    /// The signed-in account's organization; `None` for a Personal account (or a server without organizations).
    pub async fn org_membership(&self) -> Result<Option<OrgView>, CoreError> {
        let session = self.session()?;
        let me = api_call!(&session, |api| api.org_me()).map_err(ApiFailure::into_core)?;
        Ok(me.map(OrgView::from))
    }

    /// A fresh enrollment challenge for this device.
    pub async fn org_challenge(&self) -> Result<String, CoreError> {
        let session = self.session()?;
        let c = api_call!(&session, |api| api.org_challenge()).map_err(org_error)?;
        Ok(c.challenge)
    }

    /// Enrolls this device with the key and signature the app made over `challenge`.
    pub async fn org_enroll(&self, challenge: String, input: EnrollInput) -> Result<(), CoreError> {
        let request = enrollment_request(challenge, input, now());
        request.validate().map_err(CoreError::invalid)?;
        let session = self.session()?;
        api_call!(&session, |api| api.org_enroll(&request)).map_err(org_error)
    }

    /// Reports the enrolled device's posture now.
    pub async fn org_report_posture(&self, input: PostureInput) -> Result<(), CoreError> {
        let p = posture(input, now());
        p.validate().map_err(CoreError::invalid)?;
        let session = self.session()?;
        api_call!(&session, |api| api.org_posture(&p)).map_err(org_error)
    }

    /// What waits for this member's review (O2), each checked and described on the phone ([`review_views`]); none
    /// for a Personal account.
    pub async fn org_reviews(&self) -> Result<Vec<OrgReviewView>, CoreError> {
        let session = self.session()?;
        let Some(me) = api_call!(&session, |api| api.org_me()).map_err(ApiFailure::into_core)? else {
            return Ok(Vec::new());
        };
        let items = api_call!(&session, |api| api.org_reviews()).map_err(org_error)?;
        Ok(review_views(&me.org_id, items))
    }

    /// Sends this device's answer to review `id`: `signature` is the enrolled key's over [`review_message_for`] with
    /// the same `approve` and `at`. The review is read again first, so the decision sent is the one signed (`Propose`
    /// for your own change) and a review that no longer checks out is refused.
    pub async fn org_decide(&self, id: &str, approve: bool, at: i64, signature: String) -> Result<(), CoreError> {
        crate::phone_api::check_id(id)?;
        let review = self.org_reviews().await?.into_iter().find(|r| r.id == id).ok_or(CoreError::NotFound)?;
        let answer = ReviewAnswer {
            decision: decision_for(&review, approve),
            at,
            signature,
        };
        let session = self.session()?;
        api_call!(&session, |api| api.org_answer(id, &answer)).map_err(org_error)
    }

    /// Sets the member's own stricter policy: `body` is its canonical JSON, `version` greater than the one it replaces,
    /// `at` when it was signed, `signature` the enrolled key's over [`reins_proto::managed::member_policy_message`].
    pub async fn org_set_member_policy(
        &self,
        body: String,
        version: u64,
        at: i64,
        signature: String,
    ) -> Result<(), CoreError> {
        let policy = signed_member_policy(body, version, at, self.store.device_id()?, signature)?;
        let session = self.session()?;
        api_call!(&session, |api| api.org_member_policy(&policy)).map_err(org_error)
    }

    /// Whether `server_url` offers organizations (its admin console answers): only then does Settings offer "Add work
    /// account". A server that can't be reached offers none.
    pub async fn orgs_offered(&self, server_url: &str) -> Result<bool, CoreError> {
        let server = crate::http::ServerUrl::parse(server_url)?;
        let response =
            self.http.get(server.join("/reins/org")).timeout(std::time::Duration::from_secs(10)).send().await;
        Ok(response.is_ok_and(|r| r.status().is_success()))
    }

    /// The tag the server puts on this account's pushes ([`reins_proto::pairing::push_profile_tag`]); `None` signed
    /// out. A phone with a work profile routes a push to the profile whose tag it carries.
    pub async fn push_tag(&self) -> Result<Option<String>, CoreError> {
        let Ok(session) = self.session() else {
            return Ok(None);
        };
        Ok(Some(reins_proto::pairing::push_profile_tag(&session.account_user_id().await?)))
    }

    /// Starts an "Add work account" sign-in: like `sso_begin`, sent to the org that verified `domain`.
    pub fn sso_begin_work(&self, server_url: &str, domain: &str) -> Result<crate::types::SsoStart, CoreError> {
        let server = crate::http::ServerUrl::parse(server_url)?;
        let start = crate::sso::begin_work(&server, domain)?;
        Ok(crate::types::SsoStart {
            url: start.url,
            callback_scheme: crate::sso::CALLBACK_SCHEME.to_owned(),
            state: start.state,
            verifier: start.verifier.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> EnrollInput {
        EnrollInput {
            device_key: "MFkw".to_owned(),
            signature: "MEUC".to_owned(),
            hardware: true,
            attestations: vec![AttestationInput {
                kind: AttestationKindInput::AndroidKeyAttestation,
                data: "MII".to_owned(),
            }],
            posture: PostureInput {
                screen_lock: true,
                os_name: "Android".to_owned(),
                os_version: "16".to_owned(),
                rooted: Some(false),
                app_version: "1.4.0".to_owned(),
            },
        }
    }

    use reins_proto::managed::{
        RequestBody, ReviewAnswer, ReviewDecision, ReviewItem, ReviewSubject, change_digest, request_digest,
        review_message,
    };
    use serde_json::json;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::http::{ServerUrl, client};
    use crate::phone_api::PhoneApi;

    const ORG: &str = "o1";

    fn request_item(id: &str, args: serde_json::Value, digest_args: &serde_json::Value) -> ReviewItem {
        let request = RequestBody {
            org: ORG.into(),
            member: "u-dana".into(),
            connection: "c1".into(),
            tool: "gmail_send".into(),
            args,
        };
        ReviewItem {
            id: id.into(),
            subject: ReviewSubject::Request,
            org_name: "Acme".into(),
            requester: "dana@acme.com".into(),
            connection: "Claude".into(),
            title: "Send an email".into(),
            digest: request_digest(ORG, "u-dana", "c1", "gmail_send", digest_args),
            request: Some(request),
            action: None,
            action_json: None,
            snapshot: "snap1".into(),
            requires: "Security".into(),
            level: "L2".into(),
            zone: "external".into(),
            expires_at: 100,
            own: false,
            needs: Vec::new(),
            progress: String::new(),
            step: None,
            urgent: false,
            reason: None,
        }
    }

    fn change_item(id: &str, action_json: &str, digest_json: &str, own: bool) -> ReviewItem {
        ReviewItem {
            id: id.into(),
            subject: ReviewSubject::Change,
            org_name: "Acme".into(),
            requester: "lee@acme.com".into(),
            connection: String::new(),
            title: "Policy".into(),
            request: None,
            action: None,
            action_json: Some(action_json.into()),
            digest: change_digest(ORG, id, digest_json),
            snapshot: String::new(),
            requires: "Security".into(),
            level: String::new(),
            zone: String::new(),
            expires_at: 100,
            own,
            needs: Vec::new(),
            progress: String::new(),
            step: None,
            urgent: false,
            reason: None,
        }
    }

    async fn served(items: &[ReviewItem]) -> Vec<ReviewItem> {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/reins/api/org/reviews"))
            .respond_with(ResponseTemplate::new(200).set_body_json(items))
            .mount(&server)
            .await;
        let (http, url) = (client().unwrap(), ServerUrl::parse(&server.uri()).unwrap());
        PhoneApi::new(&http, &url, "TOKEN").org_reviews().await.unwrap()
    }

    #[tokio::test]
    async fn reviews_with_a_mismatched_digest_are_dropped() {
        let args = json!({"to": ["ext@other.com"], "subject": "Q3", "body": "numbers"});
        let shown = json!({"to": ["ext@other.com"], "subject": "Q3", "body": "lunch?"});
        let activate = r#"{"kind":"activate_policy","revision":3,"hash":"ab12"}"#;
        let items = served(&[
            request_item("or_ok", args.clone(), &args),
            // The server shows one body and asks to sign the digest of another.
            request_item("or_swapped", shown, &args),
            change_item("ch_ok", activate, activate, false),
            change_item("ch_swapped", r#"{"kind":"activate_policy","revision":2,"hash":"ab12"}"#, activate, false),
        ])
        .await;
        let views = review_views(ORG, items);
        let ids: Vec<&str> = views.iter().map(|v| v.id.as_str()).collect();
        assert_eq!(ids, ["or_ok", "ch_ok"]);
        // Another org's request, a request without its body and a change without its text are dropped too.
        let mut other = request_item("or_other", args.clone(), &args);
        other.request.as_mut().unwrap().org = "o2".into();
        other.digest = request_digest("o2", "u-dana", "c1", "gmail_send", &args);
        let mut bare = request_item("or_bare", args.clone(), &args);
        bare.request = None;
        let mut textless = change_item("ch_bare", activate, activate, false);
        textless.action_json = None;
        assert!(review_views(ORG, vec![other, bare, textless]).is_empty());
    }

    #[tokio::test]
    async fn lines_are_built_on_the_phone() {
        let args = json!({"to": ["ext@other.com"], "subject": "Q3"});
        let items = served(&[
            request_item("or_1", args.clone(), &args),
            change_item(
                "ch_1",
                r#"{"kind":"activate_policy","revision":3,"hash":"ab12"}"#,
                r#"{"kind":"activate_policy","revision":3,"hash":"ab12"}"#,
                true,
            ),
            change_item(
                "ch_2",
                r#"{"kind":"grant_role","subject":{"type":"member","id":"m-12345678abcd"},"role":"security_admin"}"#,
                r#"{"kind":"grant_role","subject":{"type":"member","id":"m-12345678abcd"},"role":"security_admin"}"#,
                false,
            ),
            change_item(
                "ch_3",
                r#"{"kind":"register_executor","executor_id":"e1","mode":"cloud","public_key":"KEY"}"#,
                r#"{"kind":"register_executor","executor_id":"e1","mode":"cloud","public_key":"KEY"}"#,
                false,
            ),
            change_item("ch_4", r#"{"kind":"something_new","x":1}"#, r#"{"kind":"something_new","x":1}"#, false),
        ])
        .await;
        let views = review_views(ORG, items);
        let expected = reins_policy::org::analyze_request("gmail_send", &args).lines();
        assert!(!expected.is_empty());
        assert_eq!(views[0].lines, expected);
        assert_eq!(views[0].kind, OrgReviewKind::Request);
        assert_eq!(views[0].requires, "Security");
        assert_eq!(views[1].kind, OrgReviewKind::Change);
        assert_eq!(views[1].lines, ["Activate policy revision 3"]);
        assert!(views[1].own);
        assert_eq!(views[2].lines, ["Grant Security admin to member m-123456"]);
        assert_eq!(views[3].lines, ["Register executor e1 (cloud)"]);
        assert_eq!(views[4].lines, ["Admin change"]);
    }

    #[tokio::test]
    async fn the_message_matches_the_servers() {
        let args = json!({"to": ["ext@other.com"]});
        let activate = r#"{"kind":"activate_policy","revision":3,"hash":"ab12"}"#;
        let views = review_views(
            ORG,
            vec![
                request_item("or_1", args.clone(), &args),
                change_item("ch_own", activate, activate, true),
                change_item("ch_other", activate, activate, false),
            ],
        );
        let request = &views[0];
        let digest = request_digest(ORG, "u-dana", "c1", "gmail_send", &args);
        for (approve, decision) in [(true, ReviewDecision::Approve), (false, ReviewDecision::Deny)] {
            assert_eq!(
                review_message_for(ORG, request, approve, 77),
                review_message(ORG, ReviewSubject::Request, "or_1", &digest, decision, "snap1", 77)
            );
        }
        // Your own change is confirmed with `Propose`; someone else's is approved.
        let own_digest = change_digest(ORG, "ch_own", activate);
        assert_eq!(
            review_message_for(ORG, &views[1], true, 5),
            review_message(ORG, ReviewSubject::Change, "ch_own", &own_digest, ReviewDecision::Propose, "", 5)
        );
        assert_eq!(
            review_message_for(ORG, &views[2], true, 5),
            review_message(
                ORG,
                ReviewSubject::Change,
                "ch_other",
                &change_digest(ORG, "ch_other", activate),
                ReviewDecision::Approve,
                "",
                5
            )
        );
        assert_eq!(decision_for(&views[1], false), ReviewDecision::Deny);
        // The answer goes out as the server reads it.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/reins/api/org/reviews/or_1"))
            .and(body_json(json!({"decision": "approve", "at": 77, "signature": "MEUC"})))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        let (http, url) = (client().unwrap(), ServerUrl::parse(&server.uri()).unwrap());
        let answer = ReviewAnswer {
            decision: decision_for(request, true),
            at: 77,
            signature: "MEUC".into(),
        };
        PhoneApi::new(&http, &url, "TOKEN").org_answer("or_1", &answer).await.unwrap();
    }

    #[tokio::test]
    async fn a_member_policy_goes_out_versioned_and_timed() {
        let body = r#"{"matrix":null,"rules":[]}"#;
        let policy = signed_member_policy(body.into(), 4, 1_700_000_000, "dev-1".into(), "MEUC".into()).unwrap();
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/reins/api/org/member-policy"))
            .and(body_json(json!({"body": body, "version": 4, "at": 1_700_000_000, "device": "dev-1", "sig": "MEUC"})))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        let (http, url) = (client().unwrap(), ServerUrl::parse(&server.uri()).unwrap());
        PhoneApi::new(&http, &url, "TOKEN").org_member_policy(&policy).await.unwrap();
        assert!(signed_member_policy("[]".into(), 1, 1, "d".into(), "s".into()).is_err());
        assert!(signed_member_policy(body.into(), 0, 1, "d".into(), "s".into()).is_err(), "versions start at 1");
    }

    #[test]
    fn a_member_policy_must_be_its_canonical_text() {
        assert!(checked_member_policy(r#"{"matrix":null,"rules":[]}"#).is_ok());
        assert!(checked_member_policy(r#"{"rules":[],"matrix":null}"#).is_err(), "not canonical");
        assert!(checked_member_policy("[]").is_err());
    }

    #[test]
    fn the_request_is_only_the_challenge_and_the_inputs() {
        let r = enrollment_request("ch".to_owned(), input(), 7);
        assert_eq!(r.validate(), Ok(()));
        let json = serde_json::to_value(&r).unwrap();
        let keys: Vec<&str> = json.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            ["attestations", "challenge", "device_key", "hardware", "key_algorithm", "posture", "signature"]
        );
        let posture: Vec<&str> = json["posture"].as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(posture, ["app_version", "collected_at", "os_name", "os_version", "rooted", "screen_lock"]);
        assert_eq!(json["attestations"][0]["kind"], "android_key_attestation");
    }
}
