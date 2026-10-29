//! What each connector is, as data: its service id and name, how an account is added, the access it asks the service
//! for, its kinds of change, and its actions with how risky each one is.
//!
//! A connector is a plugin: one [`ConnectorManifest`] in [`manifests`] and its tools. What used to list the connectors
//! by hand (the tool list, display names, classes, the Integrations screen) reads them from here, in this order.
//! Nothing here does IO, holds a credential or decides an approval: credentials stay on the phone, and every action
//! goes through the same approval flow whatever its risk. The risk says what an action is, once, for the approval
//! screen, the docs and reviews.
//!
//! `docs/connectors/adding-a-connector.md` walks through adding one.

use super::{
    BITBUCKET, CODEBERG, ClassInfo, DESKTOP, DEVICE_CALENDAR, DEVICE_CONTACTS, Effect, GCALENDAR, GCONTACTS, GITHUB,
    GITLAB, GMAIL, PAYMENTS, SMS, TASKS, TELEGRAM, ToolSpec, VAULT, desktop, github, gmail, payments, specs, tasks,
    vault,
};

/// How an account of a connector is added on the phone. [`AuthType::kind`] is the string the apps switch on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthType {
    /// Google sign-in on the phone; the phone holds the tokens and hands the core short-lived ones.
    Google,
    /// A phone number and the login code sent to it, then a password when the account has one (Telegram).
    PhoneLogin,
    /// An access token the user pastes; the core keeps it in the phone's secret store.
    Token,
    /// Permission the phone's own operating system gives (its calendar, contacts, messages).
    Device,
    /// The Reins password vault, unlocked on the phone.
    Vault,
    /// Payment methods set up on their own screen.
    Payments,
    /// Part of Reins itself (the desktop app, tasks): no account to add, never on the Integrations screen.
    Builtin,
}

impl AuthType {
    /// The `kind` of the Integrations screen's entry (`ServiceView::kind`), which the apps switch on.
    #[must_use]
    pub const fn kind(self) -> &'static str {
        match self {
            Self::Google => "google",
            Self::PhoneLogin => "telegram",
            Self::Token => "token",
            Self::Device => "device",
            Self::Vault => "vault",
            Self::Payments => "payments",
            Self::Builtin => "builtin",
        }
    }
}

/// How much harm an action can do if it should not have happened. Descriptive only: every action, whatever its
/// level, is approved the way it always was (a read by what the user shares, a write on the phone, a
/// [`ToolSpec::once_only`] write every time).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RiskLevel {
    /// Lists, reads and searches: nothing changes, and the user chooses what is shared.
    Low,
    /// A change that can be undone or only touches the user's own things.
    Medium,
    /// Deletes, overwrites, sends or publishes something for good.
    High,
    /// Asked for every time: too far-reaching for a standing permission.
    Critical,
}

impl RiskLevel {
    /// The risk of a tool: the one it names ([`ToolSpec::risk`]), else derived from the facts it already has (its
    /// effect, [`ToolSpec::once_only`] and its `destructiveHint`), so that no existing tool needed a new field.
    #[must_use]
    pub fn of(spec: &ToolSpec) -> Self {
        if let Some(risk) = spec.risk {
            return risk;
        }
        if spec.effect != Effect::Write {
            Self::Low
        } else if spec.once_only {
            Self::Critical
        } else if spec.annotations()["destructiveHint"] == true {
            Self::High
        } else {
            Self::Medium
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }
}

/// One connector.
#[derive(Clone, Copy, Debug)]
pub struct ConnectorManifest {
    /// The service id tools, accounts and grants carry (`gmail`, `github`).
    pub service: &'static str,
    /// The name the user knows it by.
    pub name: &'static str,
    pub auth: AuthType,
    /// The access the sign-in asks the service for (OAuth scopes), when it asks for any. Documentation: the phone's
    /// own sign-in code is what asks for them.
    pub scopes: &'static [&'static str],
    /// The kinds of change its writes fall into, for permissions (see [`ToolSpec::class`]).
    pub classes: &'static [ClassInfo],
    /// Where it sits on the Integrations screen; `None` when it is not on it (part of Reins itself).
    pub screen: Option<u16>,
    /// Its tools, in order.
    pub tools: fn() -> Vec<ToolSpec>,
}

/// One action of a connector: the facts of its tool that say what it may do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActionManifest {
    pub tool: &'static str,
    pub service: &'static str,
    pub op: &'static str,
    /// List, read, search or write: whether it changes anything.
    pub effect: Effect,
    pub risk: RiskLevel,
    /// The scopes the action needs, when the connector names them per action; else see the connector's.
    pub scopes: &'static [&'static str],
    /// Its kind of change, for permissions; empty when it has none.
    pub class: &'static str,
    pub once_only: bool,
    pub desktop_only: bool,
}

impl ActionManifest {
    #[must_use]
    pub fn of(spec: &'static ToolSpec) -> Self {
        Self {
            tool: spec.tool,
            service: spec.service,
            op: spec.op,
            effect: spec.effect,
            risk: RiskLevel::of(spec),
            scopes: spec.scopes,
            class: spec.class,
            once_only: spec.once_only,
            desktop_only: spec.desktop_only,
        }
    }

    /// Whether it changes or sends anything.
    #[must_use]
    pub fn writes(&self) -> bool {
        self.effect == Effect::Write
    }
}

impl ConnectorManifest {
    /// Its actions, in tool order.
    pub fn actions(&self) -> impl Iterator<Item = ActionManifest> + '_ {
        specs().iter().filter(|s| s.service == self.service).map(ActionManifest::of)
    }
}

// The same lists as the apps' Google sign-in (`GoogleScopes.of` on iOS, `GoogleAuthorizer.scopesOf` on Android).
const GOOGLE_GMAIL_SCOPES: &[&str] =
    &["https://www.googleapis.com/auth/gmail.modify", "https://www.googleapis.com/auth/gmail.settings.basic"];
const GOOGLE_CALENDAR_SCOPES: &[&str] =
    &["https://www.googleapis.com/auth/calendar.events", "https://www.googleapis.com/auth/calendar.readonly"];
const GOOGLE_CONTACTS_SCOPES: &[&str] = &["https://www.googleapis.com/auth/contacts.readonly"];

const fn plain(
    service: &'static str,
    name: &'static str,
    auth: AuthType,
    screen: Option<u16>,
    tools: fn() -> Vec<ToolSpec>,
) -> ConnectorManifest {
    ConnectorManifest {
        service,
        name,
        auth,
        scopes: &[],
        classes: &[],
        screen,
        tools,
    }
}

const fn with(
    mut m: ConnectorManifest,
    scopes: &'static [&'static str],
    classes: &'static [ClassInfo],
) -> ConnectorManifest {
    m.scopes = scopes;
    m.classes = classes;
    m
}

/// Every connector, in tool order: `tools/list` is their tools one connector after the other, in this order.
const MANIFESTS: &[ConnectorManifest] = &[
    plain(TELEGRAM, "Telegram", AuthType::PhoneLogin, Some(3), super::telegram_tools),
    with(
        plain(GCALENDAR, "Google Calendar", AuthType::Google, Some(1), super::gcalendar_tools),
        GOOGLE_CALENDAR_SCOPES,
        &[],
    ),
    with(
        plain(GCONTACTS, "Google Contacts", AuthType::Google, Some(2), super::gcontacts_tools),
        GOOGLE_CONTACTS_SCOPES,
        &[],
    ),
    plain(DEVICE_CALENDAR, "Phone calendar", AuthType::Device, Some(8), super::device_calendar_tools),
    plain(DEVICE_CONTACTS, "Phone contacts", AuthType::Device, Some(9), super::device_contacts_tools),
    plain(SMS, "Text messages", AuthType::Device, Some(10), super::sms_tools),
    with(plain(GMAIL, "Gmail", AuthType::Google, Some(0), gmail::tools), GOOGLE_GMAIL_SCOPES, gmail::CLASSES),
    with(plain(GITHUB, "GitHub", AuthType::Token, Some(4), github::tools), &[], github::CLASSES),
    with(plain(VAULT, "Password vault", AuthType::Vault, Some(11), vault::tools), &[], vault::CLASSES),
    plain(DESKTOP, "Desktop app", AuthType::Builtin, None, desktop::tools),
    with(plain(GITLAB, "GitLab", AuthType::Token, Some(5), desktop::gitlab_tools), &[], desktop::HOST_CLASSES),
    with(plain(CODEBERG, "Codeberg", AuthType::Token, Some(6), desktop::codeberg_tools), &[], desktop::HOST_CLASSES),
    with(plain(BITBUCKET, "Bitbucket", AuthType::Token, Some(7), desktop::bitbucket_tools), &[], desktop::HOST_CLASSES),
    plain(PAYMENTS, "Payments", AuthType::Payments, Some(12), payments::tools),
    // Tasks never had a display name: the id is what the user has always seen.
    plain(TASKS, TASKS, AuthType::Builtin, None, tasks::tools),
];

/// Every connector, in tool order.
#[must_use]
pub fn manifests() -> &'static [ConnectorManifest] {
    MANIFESTS
}

/// The connector of a service.
#[must_use]
pub fn manifest(service: &str) -> Option<&'static ConnectorManifest> {
    MANIFESTS.iter().find(|m| m.service == service)
}

/// The connectors on the Integrations screen, in the screen's order.
#[must_use]
pub fn on_screen() -> Vec<&'static ConnectorManifest> {
    let mut listed: Vec<_> = MANIFESTS.iter().filter(|m| m.screen.is_some()).collect();
    listed.sort_by_key(|m| m.screen);
    listed
}

/// Every action of every connector, in tool order.
pub fn actions() -> impl Iterator<Item = ActionManifest> {
    specs().iter().map(ActionManifest::of)
}

/// The action of a tool.
#[must_use]
pub fn action(tool: &str) -> Option<ActionManifest> {
    super::spec_for_tool(tool).map(ActionManifest::of)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn services_are_unique_and_every_tool_belongs_to_its_connector() {
        let services: BTreeSet<_> = MANIFESTS.iter().map(|m| m.service).collect();
        assert_eq!(services.len(), MANIFESTS.len(), "a service has two manifests");
        for m in MANIFESTS {
            for spec in (m.tools)() {
                assert_eq!(spec.service, m.service, "{} is listed under {}", spec.tool, m.service);
            }
            assert_eq!(m.screen.is_none(), m.auth == AuthType::Builtin, "{}", m.service);
        }
        let screens: Vec<_> = MANIFESTS.iter().filter_map(|m| m.screen).collect();
        assert_eq!(screens.iter().collect::<BTreeSet<_>>().len(), screens.len(), "two connectors share a place");
    }

    #[test]
    fn the_screen_lists_what_it_always_did_in_its_order() {
        let screen: Vec<_> = on_screen().iter().map(|m| (m.service, m.auth.kind())).collect();
        assert_eq!(
            screen,
            [
                ("gmail", "google"),
                ("gcalendar", "google"),
                ("gcontacts", "google"),
                ("telegram", "telegram"),
                ("github", "token"),
                ("gitlab", "token"),
                ("codeberg", "token"),
                ("bitbucket", "token"),
                ("device_calendar", "device"),
                ("device_contacts", "device"),
                ("sms", "device"),
                ("vault", "vault"),
                ("payments", "payments"),
            ]
        );
    }

    #[test]
    fn risk_follows_what_the_tool_already_says() {
        for spec in specs() {
            let risk = RiskLevel::of(spec);
            assert_eq!(risk == RiskLevel::Low, spec.effect != Effect::Write, "{}", spec.tool);
            assert_eq!(risk == RiskLevel::Critical, spec.once_only, "{}", spec.tool);
        }
        let risk = |tool| action(tool).unwrap().risk;
        assert_eq!(risk("telegram_read"), RiskLevel::Low);
        assert_eq!(risk("calendar_create_event"), RiskLevel::Medium);
        assert_eq!(risk("telegram_send"), RiskLevel::High);
    }

    #[test]
    fn actions_are_the_tools() {
        assert_eq!(actions().count(), specs().len());
        let per_connector: usize = MANIFESTS.iter().map(|m| m.actions().count()).sum();
        assert_eq!(per_connector, specs().len());
        let gmail = manifest(GMAIL).unwrap();
        assert!(gmail.actions().all(|a| a.service == GMAIL));
        assert_eq!(gmail.scopes, GOOGLE_GMAIL_SCOPES);
    }
}
