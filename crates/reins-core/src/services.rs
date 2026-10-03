//! Adding, checking and removing the accounts of every integration (the "Integrations" screen), whichever way an
//! account is added: Google sign-in, a phone number and code, a pasted token, or permission on this phone.

use std::sync::Arc;

use crate::CoreError;
use crate::connector::device::PHONE_ACCOUNT;
use crate::connector::{Connector, LoginProgress};
use crate::engine::Engine;
use crate::store::unix_now;
use crate::types::{AccountView, GmailStatus, PendingKind, ServiceView};
use crate::views::{self, ParkedRequest, service_name};

/// Every integration, in the order the screen lists them, and how an account is added to it.
const CATALOGUE: &[(&str, &str)] = &[
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
];

impl Engine {
    fn connector(&self, service: &str) -> Result<Arc<dyn Connector>, CoreError> {
        self.connectors.get(service).ok_or_else(|| CoreError::invalid("that integration is not available here"))
    }

    /// The integrations with their accounts, and whether each can be used in this build.
    pub fn services(&self) -> Result<Vec<ServiceView>, CoreError> {
        let accounts = self.accounts()?;
        Ok(CATALOGUE
            .iter()
            .map(|&(service, kind)| {
                let (available, note) = if service == views::SERVICE_GMAIL {
                    (true, None)
                } else if let Some(connector) = self.connectors.get(service) {
                    let note = connector.unavailable();
                    (note.is_none(), note)
                } else {
                    (false, Some("Not available in this build.".to_owned()))
                };
                ServiceView {
                    service: service.to_owned(),
                    name: service_name(service).to_owned(),
                    kind: kind.to_owned(),
                    available,
                    note,
                    accounts: accounts.iter().filter(|a| a.service == service).cloned().collect(),
                }
            })
            .collect())
    }

    /// Adds an account that needs no secret from the user: a Google account that was just authorized on this phone
    /// (`hint` is its address), or this phone's own calendar, contacts or messages once Android allowed them.
    pub async fn add_service_account(&self, service: &str, hint: &str) -> Result<AccountView, CoreError> {
        if service == views::SERVICE_GMAIL {
            return self.add_account(hint).await;
        }
        let connector = self.connector(service)?;
        if let Some(note) = connector.unavailable() {
            return Err(CoreError::invalid(note));
        }
        let kind = CATALOGUE.iter().find(|(s, _)| *s == service).map(|(_, k)| *k);
        let account = match kind {
            Some("google") => connector.identify(hint).await?,
            Some("device") => {
                if connector.status(PHONE_ACCOUNT).await != GmailStatus::Ready {
                    return Err(CoreError::needs_attention("Allow Rewarden to use this on the phone first"));
                }
                PHONE_ACCOUNT.to_owned()
            }
            _ => return Err(CoreError::invalid("this integration is not added that way")),
        };
        self.register_account(service, &account)
    }

    /// Adds an account from a pasted access token (GitHub, GitLab, Codeberg; Bitbucket: `email:token`).
    pub async fn add_token_account(&self, service: &str, token: &str) -> Result<AccountView, CoreError> {
        let connector = self.connector(service)?;
        let account = connector.sign_in_token(token.trim()).await?;
        self.register_account(service, &account)
    }

    /// Step 1 of a phone-number sign-in (Telegram): the login code is sent to the phone.
    pub async fn login_begin(&self, service: &str, phone: &str) -> Result<(), CoreError> {
        let connector = self.connector(service)?;
        if let Some(note) = connector.unavailable() {
            return Err(CoreError::invalid(note));
        }
        connector.login_begin(phone).await
    }

    /// Step 2: the code. The account is added once the sign-in is complete.
    pub async fn login_code(&self, service: &str, code: &str) -> Result<LoginProgress, CoreError> {
        let progress = self.connector(service)?.login_code(code.trim()).await?;
        if let LoginProgress::Done {
            account,
        } = &progress
        {
            self.register_account(service, account)?;
        }
        Ok(progress)
    }

    /// Step 3, for accounts with two-step verification.
    pub async fn login_password(&self, service: &str, password: &str) -> Result<AccountView, CoreError> {
        let account = self.connector(service)?.login_password(password).await?;
        self.register_account(service, &account)
    }

    /// Disconnects an account of any integration: it is signed out and its secrets and grants deleted, and requests
    /// waiting for it are answered with an error.
    pub async fn remove_service_account(&self, service: &str, account: &str) -> Result<(), CoreError> {
        let account = account.trim().to_lowercase();
        if !self.store.remove_account(service, &account)? {
            return Err(CoreError::NotFound);
        }
        if let Some(connector) = self.connectors.get(service) {
            connector.forget(&account).await;
        }
        self.store.secret_delete(service, &account).ok();
        let session = self.session().ok();
        for row in self.store.pending_rows(unix_now())? {
            if row.kind != PendingKind::Request {
                continue;
            }
            let Ok(parked) = serde_json::from_slice::<ParkedRequest>(&row.payload) else {
                continue;
            };
            let about = parked.account.as_deref().or(parked.request.account.as_deref());
            if parked.service() != service || about != Some(account.as_str()) || !self.store.remove_pending(&row.id)? {
                continue;
            }
            self.notifier.item_resolved(row.id.clone());
            if let Some(session) = &session {
                self.respond(
                    session,
                    &row.id,
                    rewarden_proto::relay::RelayOutcome::Error {
                        message: format!("The user disconnected {account} from {} in Rewarden.", service_name(service)),
                    },
                )
                .await
                .ok();
            }
        }
        Ok(())
    }

    /// Whether one account of an integration can be used right now.
    pub async fn service_account_status(&self, service: &str, account: &str) -> GmailStatus {
        if service == views::SERVICE_GMAIL {
            return self.account_status(account).await;
        }
        match self.connectors.get(service) {
            Some(connector) => connector.status(account).await,
            None => GmailStatus::Unavailable {
                message: "Not available in this build.".to_owned(),
            },
        }
    }
}
