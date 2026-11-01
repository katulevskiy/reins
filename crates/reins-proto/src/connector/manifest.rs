//! What each connector is, as data: its service id and name, how an account is added, the access it asks the service
//! for, its kinds of change and its tools.
//!
//! A connector is one [`ConnectorManifest`] in [`manifests`] plus its implementation of the core's `Connector` trait.
//! Everything that used to list the connectors by hand (the tool list, display names, classes, the Integrations
//! screen) reads them from here, in this order. The server builds `tools/list` from this table without the core, so
//! this table, not the core's registry, is the one list of connectors.
//!
//! Nothing here does IO, holds a credential or decides an approval: credentials stay on the phone, and every tool goes
//! through the approval flow its [`ToolSpec`] facts (effect, class, [`ToolSpec::once_only`]) already decide.
//!
//! `docs/connectors/adding-a-connector.md` walks through adding one.

use super::{
    BITBUCKET, CODEBERG, ClassInfo, DESKTOP, DEVICE_CALENDAR, DEVICE_CONTACTS, GCALENDAR, GCONTACTS, GITHUB, GITLAB,
    GMAIL, PAYMENTS, SMS, TASKS, TELEGRAM, ToolSpec, VAULT, desktop, github, gmail, payments, tasks, vault,
};

/// How an account of a connector is added on the phone. [`AuthType::kind`] is the string the apps switch on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthType {
    /// Google sign-in on the phone; the phone holds the tokens and hands the core short-lived ones.
    Google,
    /// A phone number and the login code sent to it, then a password when the account has one (Telegram).
    PhoneLogin,
    /// An access token the user pastes; the core checks it and keeps it in the phone's secret store.
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

/// One connector.
#[derive(Clone, Copy, Debug)]
pub struct ConnectorManifest {
    /// The service id tools, accounts, grants and the core's registry carry (`gmail`, `github`).
    pub service: &'static str,
    /// The name the user knows it by.
    pub name: &'static str,
    pub auth: AuthType,
    /// The access the sign-in asks the service for (OAuth scopes), when it asks for any. A tool's own
    /// [`ToolSpec::scopes`] must be among these.
    pub scopes: &'static [&'static str],
    /// The kinds of change its writes fall into, for permissions (see [`ToolSpec::class`]).
    pub classes: &'static [ClassInfo],
    /// Where it sits on the Integrations screen; `None` when it is not on it (part of Reins itself).
    pub screen: Option<u16>,
    /// Builds its tools, in order. Every one carries this connector's `service`.
    pub tools: fn() -> Vec<ToolSpec>,
}

// The same lists as the apps' Google sign-in (`GoogleScopes.of` on iOS, `GoogleAuthorizer.scopesOf` on Android),
// which is what asks Google for them.
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

/// Every connector, in tool order: `tools/list` is their tools one connector after the other, in this order. A new
/// connector goes at the end, so that clients see the tools they knew in the same places.
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::connector::specs;

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
    fn the_tool_list_is_the_connectors_tools_in_order() {
        let from_manifests: Vec<&str> = MANIFESTS.iter().flat_map(|m| (m.tools)()).map(|s| s.tool).collect();
        let listed: Vec<&str> = specs().iter().map(|s| s.tool).collect();
        assert_eq!(from_manifests, listed);
    }

    #[test]
    fn a_tool_needs_only_scopes_its_sign_in_asks_for() {
        for m in MANIFESTS {
            for spec in (m.tools)() {
                for scope in spec.scopes {
                    assert!(
                        m.scopes.contains(scope),
                        "{} needs {scope}, which {} never asks for",
                        spec.tool,
                        m.service
                    );
                }
            }
        }
        assert_eq!(manifest(GMAIL).unwrap().scopes, GOOGLE_GMAIL_SCOPES);
    }
}
