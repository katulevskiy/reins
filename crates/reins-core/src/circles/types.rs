//! What the apps see of Circles (UniFFI).

/// A person's role in an arrangement (04 §1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum CircleRole {
    /// Decides instead of the owner, within the scope (either is enough).
    Delegate,
    /// Must approve too (both).
    Cosign,
    /// Must approve (cooperative in Personal mode).
    Guardian,
    /// Is told; no buttons.
    Watcher,
}

impl From<reins_proto::circle::CircleKind> for CircleRole {
    fn from(k: reins_proto::circle::CircleKind) -> Self {
        use reins_proto::circle::CircleKind;
        match k {
            CircleKind::Delegate => Self::Delegate,
            CircleKind::Cosign => Self::Cosign,
            CircleKind::Guardian => Self::Guardian,
            CircleKind::Watcher => Self::Watcher,
        }
    }
}

impl From<CircleRole> for reins_proto::circle::CircleKind {
    fn from(r: CircleRole) -> Self {
        match r {
            CircleRole::Delegate => Self::Delegate,
            CircleRole::Cosign => Self::Cosign,
            CircleRole::Guardian => Self::Guardian,
            CircleRole::Watcher => Self::Watcher,
        }
    }
}

/// What an arrangement covers, as the Add screen offers it.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum CircleScopePreset {
    /// Everything the owner's agents do.
    Everything,
    /// Spending money.
    Money,
    /// Messages to Work or Outside people.
    MessagesOutside,
    /// What can't be undone (L3).
    Risky,
    /// Routine requests only (L0–L1): what a delegate usually handles.
    Routine,
    /// Everything one agent (connection) does.
    Agent {
        connection_id: String,
        label: String,
    },
}

/// What the owner sets up (the Add screen).
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CircleDraft {
    pub role: CircleRole,
    pub preset: CircleScopePreset,
    /// Deferred (the usual) or Instant (the helper is at hand).
    pub deferred: bool,
    /// `None`: the class's default (2 min Instant, 4 h Deferred).
    pub window_secs: Option<u64>,
    /// A delegate's window that runs out leaves the request to the owner (else: not approved).
    pub fall_back_to_me: bool,
    /// The helper sees the card as the owner does (else a summary). Never for a watcher.
    pub full_cards: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CircleInviteStart {
    pub id: String,
    /// What the helper opens (a link or a QR code): `reins-circle:<server>/<secret>`.
    pub link: String,
    pub expires_at: i64,
}

/// Where the owner's invitation stands.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum CircleInviteProgress {
    /// Nobody opened the link yet.
    Waiting,
    /// Someone answered: both phones show `code`; waiting for them to accept.
    Compare {
        code: String,
        helper: String,
    },
    /// They accepted: compare `code` once more, then confirm (`circle_invite_confirm`).
    Accepted {
        code: String,
        helper: String,
    },
    Active {
        link_id: String,
    },
    Declined,
    Expired,
    Cancelled,
}

/// Where the helper's side of an invitation stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Enum)]
pub enum CircleJoinStage {
    /// Answered; waiting for the inviter's phone.
    Opened,
    /// The terms and the code are here: compare, then accept or decline.
    Compare,
    /// Accepted; waiting for the inviter to confirm.
    Accepted,
    Active,
    Declined,
    Expired,
}

/// An invitation as the helper sees it.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CircleInviteView {
    pub id: String,
    /// The inviting account's email (the code, not this label, proves who it is).
    pub inviter: String,
    pub role: CircleRole,
    /// Empty until the inviter's phone sent the terms.
    pub scope_line: String,
    pub deferred: bool,
    pub window_secs: u64,
    pub full_cards: bool,
    pub code: Option<String>,
    pub stage: CircleJoinStage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum CircleLinkState {
    Active,
    /// The owner paused it: nothing goes to the helper meanwhile.
    Paused,
    Ended,
    /// The other side's owner key changed (a recovery, a reset): nothing counts until they're invited again.
    Suspended,
}

/// One arrangement, from this account's side.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CirclePerson {
    pub link_id: String,
    /// The other account's email.
    pub peer: String,
    pub role: CircleRole,
    /// This account is the owner (the other person helps it); else this account helps them.
    pub mine: bool,
    pub scope_line: String,
    pub deferred: bool,
    pub window_secs: u64,
    pub state: CircleLinkState,
    pub since: i64,
}

/// On the owner's card: who else decides, and how far it got ("Needs Alex too", "Alex approved, waiting for you").
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CircleProgress {
    /// People still to answer.
    pub needs: Vec<String>,
    /// People who approved.
    pub approved: Vec<String>,
    /// The owner's own tap is still needed.
    pub waiting_for_you: bool,
    /// The owner approved; it runs when the others have.
    pub you_approved: bool,
    pub deferred: bool,
    pub deadline: i64,
    pub line: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Enum)]
pub enum CircleReviewState {
    Waiting,
    /// This phone approved; the owner's phone runs it.
    Approved,
    Denied,
    Expired,
    /// The owner's side settled it (or ended or paused the arrangement) first.
    Withdrawn,
}

/// A request of another account waiting for this one (For others).
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CircleReviewView {
    /// The packet's id: what `circle_answer` takes.
    pub id: String,
    pub link_id: String,
    /// The owner's email.
    pub person: String,
    pub role: CircleRole,
    pub title: String,
    pub subtitle: String,
    pub headline: String,
    pub service: String,
    /// The owner's agent ("ChatGPT").
    pub agent: String,
    /// What it does ("Ask · Outside · Reaches people").
    pub line: String,
    /// Only when the arrangement shows full cards.
    pub detail: Option<String>,
    pub scope_line: String,
    pub created_at: i64,
    pub deadline: i64,
    pub state: CircleReviewState,
}

/// Something a watcher (or a helper) is told; no buttons.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CircleNoticeView {
    pub id: String,
    pub person: String,
    pub text: String,
    pub outcome: Option<String>,
    pub at: i64,
}

/// What the apps hear from the core about Circles ([`crate::Notifier::circle_changed`]).
#[allow(clippy::large_enum_variant, reason = "UniFFI enums carry records by value; the event is short-lived")]
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum CircleEvent {
    /// A request of another account waits for this phone: the one Circle notification ("Sam: purchase $42 waiting").
    ReviewWaiting {
        review: CircleReviewView,
    },
    /// Anything else changed (a review settled, an invitation moved, a person, the owner's progress): refresh quietly.
    Changed,
}
