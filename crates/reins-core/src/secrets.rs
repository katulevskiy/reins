//! Reins Secrets in the engine (docs/secrets.md): what the approval flow does besides the vault's own previews.
//!
//! - Standing permissions for an environment ([`build_secrets_grant`]): bound to the project, the environment, the
//!   runner, the project's folder on the computer, the command and the variables. The command is the exact one, or a
//!   prefix the user picked among those the phone offered; nothing the computer proposes is used.
//! - `integration_token_store`: a git host's token from the computer, in a box only this phone opens, added to the
//!   phone's integration of that host.
//! - "Revoke on all computers": the environment's permissions end here, and every paired computer is told to drop the
//!   values it holds (a [`LeaseRevocation`] sealed to its key, picked up by its daemon).

use reins_policy::{Grant, Scope, SecretsScope};
use reins_proto::connector::{ConnectorCall, TOKEN_HOSTS};
use reins_proto::desktop::{
    CommandScope, LeaseRevocation, PHONE_KEY_CHANGED, RevocationPost, SEALED_FIELD, SECRET_STORE_TTL_SECS,
    SealedRevocation, SecretsStoreAck, TO_DESKTOP, TO_PHONE, TokenStoreAck, TokenStored, TokenToStore, value_check,
};
use reins_proto::ids::{ConnectionId, GrantId};
use serde_json::{Value, json};

use crate::connector::sealed::{box_from_phone, client_key, nonce_arg, open_from, seal};
use crate::connector::vault::{ReleaseAsk, Vault};
use crate::connector::{Preview, SecretsPreview};
use crate::engine::Engine;
use crate::phone_api::ApiFailure;
use crate::session::api_call;
use crate::store::unix_now;
use crate::types::{SecretsRevoked, StandingGrant, TokenStoreView};

/// Revocations not yet delivered to the server (sealed, so no secret), sent again on each sync.
const REVOCATION_OUTBOX: &str = "secrets.revocations.outbox";

/// A revocation waiting in the outbox, and when it was made (kept a day, as the server would).
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct Outgoing {
    item: SealedRevocation,
    made_at: i64,
}
use crate::{CoreError, text};

/// A box made this far ahead of the phone's clock is refused too.
const CLOCK_SKEW_SECS: i64 = 5 * 60;

/// The standing permission the user chose alongside approving a release. `standing.scope.secrets` names one of the
/// commands the approval offered (`exact` when it names none) and whether every variable is covered.
pub(crate) fn build_secrets_grant(
    standing: &StandingGrant,
    ask: &ReleaseAsk,
    connection: &ConnectionId,
    account: &str,
    now: i64,
) -> Result<Grant, CoreError> {
    let expires_at = match standing.duration_secs {
        None => None,
        Some(0) => return Err(CoreError::invalid("the grant duration must be longer than zero")),
        Some(secs) => Some(now.saturating_add(i64::try_from(secs).unwrap_or(i64::MAX))),
    };
    let choice = standing.scope.secrets.as_ref();
    // Only what the phone offered: a command that cannot be remembered is offered nothing.
    let wanted = choice.map_or("exact", |c| c.command.as_str());
    let command =
        ask.command_options().into_iter().find(|(id, _)| id == wanted).map(|(_, scope)| scope).ok_or_else(|| {
            CoreError::invalid("this command is approved one run at a time; choose one of those offered")
        })?;
    let fields = match (&ask.fields, choice.is_some_and(|c| c.all_variables)) {
        (Some(asked), false) => asked.clone(),
        _ => Vec::new(),
    };
    Grant::new(
        GrantId(uuid::Uuid::new_v4().to_string()),
        connection.clone(),
        Scope::Secrets(SecretsScope {
            project: ask.project.clone(),
            env: ask.env.clone(),
            runner: ask.runner.clone(),
            root: ask.root.clone(),
            command,
            fields,
        }),
        now,
        expires_at,
        standing.max_uses,
    )
    .map(|g| g.for_account(Some(account.to_owned())))
    .map_err(|e| CoreError::invalid(e.to_string()))
}

/// The host a token for `service` must be for.
fn host_of(service: &str) -> Option<&'static str> {
    TOKEN_HOSTS.iter().find(|(s, _)| *s == service).map(|(_, h)| *h)
}

impl Engine {
    /// The vault, for the flow's own calls into it.
    fn vault(&self) -> Vault {
        Vault::new(self.vault_session(), std::sync::Arc::clone(&self.store))
    }

    /// Releases an environment, bound to `command` (the exact command, or the prefix of the standing permission).
    pub(crate) async fn release_env(
        &self,
        account: &str,
        call: &ConnectorCall,
        command: Option<(CommandScope, Option<i64>)>,
    ) -> Result<Value, CoreError> {
        let (command, ends) = match command {
            Some(c) => c,
            None => (crate::connector::vault::release_ask(call)?.exact(), None),
        };
        crate::connector::vault::perform_release(&self.vault(), account, call, command, ends).await
    }

    /// Opens a token boxed to this phone and checks it answers this very call.
    fn open_token(&self, call: &ConnectorCall, now: i64) -> Result<TokenToStore, CoreError> {
        let bad = |m: &str| CoreError::service(m);
        let desktop = client_key(call)?;
        let nonce = nonce_arg(call)?;
        let unreadable = || {
            CoreError::service(format!("{PHONE_KEY_CHANGED} If this is a new phone, run reins vault list --new-phone."))
        };
        let plain = open_from(desktop, &*self.store.inbox_key()?, call.str_arg("sealed").unwrap_or_default())
            .ok_or_else(unreadable)?;
        let opened: TokenToStore = serde_json::from_slice(&plain).map_err(|_| unreadable())?;
        if opened.dir != TO_PHONE || opened.nonce != nonce {
            return Err(bad("The token was sealed for another request; refused."));
        }
        if opened.created_at < now - SECRET_STORE_TTL_SECS || opened.created_at > now + CLOCK_SKEW_SECS {
            return Err(bad("The token was sealed too long ago (or the computer's clock is wrong); refused."));
        }
        let service = call.str_arg("service").unwrap_or_default();
        let host = call.str_arg("host").unwrap_or_default().to_ascii_lowercase();
        if opened.service != service || opened.host.to_ascii_lowercase() != host {
            return Err(bad("The token was sealed for another host; refused."));
        }
        if host_of(service) != Some(host.as_str()) {
            return Err(bad("Only a token for github.com, gitlab.com, codeberg.org or bitbucket.org can be added."));
        }
        if opened.token.trim().is_empty() || opened.token.len() > 4_000 {
            return Err(bad("The token is empty or too long."));
        }
        if self.connectors.get(service).is_none() {
            return Err(bad("That integration is not available on this phone."));
        }
        Ok(opened)
    }

    /// What adding a git host's token from the computer would do.
    pub(crate) fn token_store_preview(&self, call: &ConnectorCall) -> Result<Preview, CoreError> {
        let opened = self.open_token(call, unix_now())?;
        let name = crate::views::service_name(&opened.service);
        let scopes: Vec<String> =
            opened.scopes.iter().map(|s| text::one_line(s)).filter(|s| !s.is_empty()).take(30).collect();
        let check = value_check(&opened.token);
        let mut lines = vec![format!("Add a {name} account from the computer")];
        lines.push(format!("Host: {}", opened.host));
        if !scopes.is_empty() {
            lines.push(format!("Scopes (as the computer says): {}", scopes.join(", ")));
        }
        lines.push(format!("Check: {check}. The terminal shows the same four digits; deny if it does not."));
        Ok(Preview {
            resource: format!("{}_accounts", opened.service),
            resource_label: format!("{name} accounts"),
            lines,
            once_only: true,
            secrets: Some(SecretsPreview::Token(TokenStoreView {
                service: opened.service.clone(),
                host: opened.host.clone(),
                scopes,
                check,
            })),
            ..Preview::default()
        })
    }

    /// Adds the token to the phone's integration; the acknowledgement boxed from this phone's key.
    pub(crate) async fn token_store_perform(&self, call: &ConnectorCall) -> Result<Value, CoreError> {
        let opened = self.open_token(call, unix_now())?;
        let added = self.add_token_account(&opened.service, &opened.token).await?;
        let ack = TokenStoreAck {
            v: 1,
            dir: TO_DESKTOP.to_owned(),
            nonce: opened.nonce.clone(),
            service: opened.service.clone(),
            account: added.account,
        };
        Ok(json!({ SEALED_FIELD: box_from_phone(client_key(call)?, &*self.store.inbox_key()?, &ack)? }))
    }

    /// Every token of a store goes to an integration this phone has.
    pub(crate) fn check_store_tokens(&self, preview: &Preview) -> Result<(), CoreError> {
        if let Some(SecretsPreview::Store(v)) = &preview.secrets {
            for t in &v.tokens {
                if t.service.is_empty() || self.connectors.get(&t.service).is_none() {
                    return Err(CoreError::service(format!("This phone cannot take a token for {}.", t.host)));
                }
            }
        }
        Ok(())
    }

    /// Carries out an approved store: the environments and SSH keys in the vault, the tokens to the integrations; the
    /// acknowledgement boxed from this phone's key, one result per part. Computers holding the earlier values of the
    /// environments changed drop them.
    pub(crate) async fn secrets_store_perform(&self, account: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
        let (opened, envs, ssh) = crate::connector::vault::perform_store_parts(&self.vault(), account, call).await?;
        let mut tokens = Vec::new();
        for t in &opened.tokens {
            let host = t.host.to_ascii_lowercase();
            let Some(service) = t.service() else {
                tokens.push(TokenStored {
                    host,
                    error: Some("not a git host the phone serves".to_owned()),
                    ..TokenStored::default()
                });
                continue;
            };
            // Bitbucket signs in with the user and an app password together.
            let token = match (&t.user, service) {
                (Some(user), "bitbucket") => zeroize::Zeroizing::new(format!("{user}:{}", t.token)),
                _ => zeroize::Zeroizing::new(t.token.clone()),
            };
            tokens.push(match self.add_token_account(service, &token).await {
                Ok(added) => TokenStored {
                    host,
                    service: service.to_owned(),
                    account: added.account,
                    error: None,
                },
                Err(e) => TokenStored {
                    host,
                    service: service.to_owned(),
                    account: String::new(),
                    error: Some(e.to_string()),
                },
            });
        }
        for e in envs.iter().filter(|e| e.error.is_none()) {
            if let Err(err) = self.tell_computers(Some(&e.project), Some(&e.env)).await {
                log::warn!("could not tell the computers that {}/{} changed: {err}", e.project, e.env);
            }
        }
        let ack = SecretsStoreAck {
            v: 1,
            dir: TO_DESKTOP.to_owned(),
            nonce: opened.nonce.clone(),
            envs,
            ssh,
            tokens,
        };
        Ok(json!({ SEALED_FIELD: box_from_phone(client_key(call)?, &*self.store.inbox_key()?, &ack)? }))
    }

    /// Whether the phone asks every time for `project/env` (`Some`), or leaves it to `reins.toml` and the environment's
    /// name (`None`: production and `confirm = "always"` are asked every time).
    pub fn set_secrets_always_ask(&self, project: &str, env: &str, always: Option<bool>) -> Result<(), CoreError> {
        crate::connector::vault::set_always_ask(&self.store, project, env, always)
    }

    pub fn secrets_always_ask(&self, project: &str, env: &str) -> Result<Option<bool>, CoreError> {
        crate::connector::vault::always_ask_choice(&self.store, project, env)
    }

    /// "Revoke on all computers": ends the permissions for secrets of `project` (and `env`; neither: every one) on this
    /// phone first, so nothing it signs afterwards relies on them, then tells every paired computer to drop the values
    /// it holds. A computer not told yet is named in the answer, and told on a later sync.
    pub async fn revoke_secrets_everywhere(
        &self,
        project: Option<&str>,
        env: Option<&str>,
    ) -> Result<SecretsRevoked, CoreError> {
        let matches = |s: &SecretsScope| project.is_none_or(|p| p == s.project) && env.is_none_or(|e| e == s.env);
        let mut ended = 0;
        for g in self.store.grants()? {
            if let Scope::Secrets(s) = &g.grant.scope
                && matches(s)
                && !g.grant.revoked
            {
                self.store.revoke_grant(&g.grant.id)?;
                ended += 1;
            }
        }
        let (told, pending) = self.tell_computers(project, env).await?;
        let not_delivered = if pending.is_empty() {
            Vec::new()
        } else {
            let known = self.connections().await.unwrap_or_default();
            pending
                .iter()
                .map(|id| {
                    known.iter().find(|c| &c.id == id).map_or_else(|| "a computer".to_owned(), |c| c.label.clone())
                })
                .collect()
        };
        Ok(SecretsRevoked {
            ended,
            told,
            not_delivered,
        })
    }

    /// Leaves for every paired computer a note, sealed to its key, to drop the values it holds of `project` (and `env`;
    /// neither: all), with any note still waiting from before. Returns how many computers were told, and the
    /// connections of those not told yet (their notes wait in the outbox; one whose key cannot seal a note is listed
    /// too).
    pub(crate) async fn tell_computers(
        &self,
        project: Option<&str>,
        env: Option<&str>,
    ) -> Result<(u32, Vec<String>), CoreError> {
        let revocation = LeaseRevocation {
            v: 1,
            dir: TO_DESKTOP.to_owned(),
            id: uuid::Uuid::new_v4().to_string(),
            issued_at: unix_now(),
            project: project.map(str::to_owned),
            env: env.map(str::to_owned),
        };
        let now = unix_now();
        let mut fresh = Vec::new();
        let mut unsealed = Vec::new();
        for connection_id in self.store.desktop_connections()? {
            let sealed = self
                .store
                .desktop_key(&connection_id)?
                .as_deref()
                .and_then(reins_proto::desktop::decode_key)
                .and_then(|key| seal(key, &revocation).ok());
            match sealed {
                Some(sealed) => fresh.push(Outgoing {
                    item: SealedRevocation {
                        connection_id,
                        sealed,
                        all: revocation.project.is_none(),
                    },
                    made_at: now,
                }),
                None => unsealed.push(connection_id),
            }
        }
        let told = u32::try_from(fresh.len()).unwrap_or(u32::MAX);
        let mut pending = self.flush_revocations(fresh).await?;
        pending.extend(unsealed);
        pending.sort();
        pending.dedup();
        Ok((
            if pending.is_empty() {
                told
            } else {
                told.saturating_sub(u32::try_from(pending.len()).unwrap_or(0))
            },
            pending,
        ))
    }

    /// Posts what waits in the outbox with `fresh`; what the server did not take stays there (never dropped, never
    /// counted as told). The connections whose notes still wait.
    pub(crate) async fn flush_revocations(&self, fresh: Vec<Outgoing>) -> Result<Vec<String>, CoreError> {
        let now = unix_now();
        let mut out: Vec<Outgoing> =
            self.store.meta_get(REVOCATION_OUTBOX)?.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        out.retain(|o| o.made_at > now - reins_proto::desktop::REVOCATION_TTL_SECS);
        out.extend(fresh);
        if out.is_empty() {
            return Ok(Vec::new());
        }
        // Kept before it is sent: a failure, or the app ending, leaves it to the next sync.
        let kept = serde_json::to_string(&out).map_err(|e| CoreError::storage(e.to_string()))?;
        self.store.meta_set(REVOCATION_OUTBOX, &kept)?;
        let post = RevocationPost {
            items: out.iter().map(|o| o.item.clone()).collect(),
        };
        let sent = match self.session() {
            Ok(session) => api_call!(&session, |api| api.post_revocations(&post)).map_err(ApiFailure::into_core),
            Err(e) => Err(e),
        };
        match sent {
            Ok(()) => {
                self.store.meta_set(REVOCATION_OUTBOX, "[]")?;
                Ok(Vec::new())
            }
            Err(e) => {
                log::warn!("revocations not delivered yet, kept to send again: {e}");
                Ok(out.into_iter().map(|o| o.item.connection_id).collect())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{GrantScopeChoice, SecretsScopeChoice};

    fn ask(fields: Option<&[&str]>) -> ReleaseAsk {
        ReleaseAsk {
            project: "acme".into(),
            env: "dev".into(),
            fields: fields.map(|f| f.iter().map(|x| (*x).to_owned()).collect()),
            program: "/usr/bin/npm".into(),
            args: vec!["run".into(), "dev".into()],
            root: "/home/me/acme".into(),
            runner: "agent:claude-code".into(),
            lease: 3_600,
            purpose: None,
            confirm_always: false,
            print: false,
            typed: None,
            aliases: Vec::new(),
            path_check: None,
        }
    }

    fn standing(secrets: Option<SecretsScopeChoice>) -> StandingGrant {
        StandingGrant {
            duration_secs: Some(3_600),
            max_uses: None,
            scope: GrantScopeChoice {
                all_mail: false,
                selected_messages_only: false,
                sender_addresses: vec![],
                sender_domains: vec![],
                subject_pattern: None,
                recipient_addresses: vec![],
                recipient_domains: vec![],
                resources: vec![],
                classes: vec![],
                secrets,
            },
        }
    }

    fn scope_of(g: &Grant) -> &SecretsScope {
        let Scope::Secrets(s) = &g.scope else {
            panic!("a secrets scope expected")
        };
        s
    }

    #[test]
    fn a_grant_for_secrets_is_the_exact_command_unless_the_user_picked_an_offered_prefix() {
        let conn: ConnectionId = "c1".into();
        let exact = build_secrets_grant(&standing(None), &ask(Some(&["A"])), &conn, "me", 100).unwrap();
        let s = scope_of(&exact);
        assert_eq!((s.command.shown(), s.fields.clone()), ("npm run dev".to_owned(), vec!["A".to_owned()]));
        assert_eq!((s.runner.as_str(), s.root.as_str()), ("agent:claude-code", "/home/me/acme"));
        assert_eq!(exact.account.as_deref(), Some("me"));
        let picked = SecretsScopeChoice {
            command: "prefix:2".into(),
            all_variables: true,
        };
        let wide = build_secrets_grant(&standing(Some(picked)), &ask(Some(&["A"])), &conn, "me", 100).unwrap();
        assert_eq!(scope_of(&wide).command.shown(), "npm run dev *");
        assert!(scope_of(&wide).fields.is_empty(), "every variable, as the user chose");
        for bad in ["prefix:0", "prefix:1", "prefix:7", "anything", ""] {
            let c = SecretsScopeChoice {
                command: bad.into(),
                all_variables: false,
            };
            assert!(build_secrets_grant(&standing(Some(c)), &ask(None), &conn, "me", 100).is_err(), "{bad}");
        }
    }

    #[test]
    fn tokens_go_only_to_the_hosts_the_integrations_serve() {
        assert_eq!(host_of("github"), Some("github.com"));
        assert_eq!(host_of("npm"), None);
    }
}
