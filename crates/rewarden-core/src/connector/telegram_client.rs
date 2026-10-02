//! The real Telegram backend, on grammers (Telegram's own protocol, spoken directly from this phone). The session of a
//! signed-in account is kept sealed in the local store; nothing about it ever reaches the Rewarden server.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use grammers_client::client::{LoginToken, PasswordToken};
use grammers_client::message::InputMessage;
use grammers_client::peer::Peer;
use grammers_client::{Client, InvocationError, SenderPool, SignInError};
use grammers_session::types::{ChannelState, DcOption, PeerId, PeerInfo, UpdateState, UpdatesState};
use grammers_session::{BoxFuture, Session, SessionData};
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;

use super::LoginProgress;
use super::telegram::{Chat, Msg, TelegramBackend};
use crate::store::Store;
use crate::types::GmailStatus;
use crate::{CoreError, text};

const SERVICE: &str = "telegram";
/// No single call to Telegram may take longer than this.
const TIMEOUT: Duration = Duration::from_secs(30);

/// What is kept of a session between runs: the datacenters (with the authorization key), the peers seen, the update
/// state. Serialized to JSON and sealed with the data key.
#[derive(Serialize, Deserialize)]
struct Stored {
    home_dc: i32,
    dc_options: Vec<DcOption>,
    peers: Vec<PeerInfo>,
    updates: UpdatesState,
}

/// A session that lives in memory and can be written out (grammers has no such storage that avoids a database).
struct PersistedSession {
    data: Mutex<SessionData>,
}

#[derive(Debug)]
struct Poisoned;

impl std::fmt::Display for Poisoned {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("session lock is poisoned")
    }
}

impl std::error::Error for Poisoned {}

impl PersistedSession {
    fn new(data: SessionData) -> Self {
        Self {
            data: Mutex::new(data),
        }
    }

    fn lock(&self) -> MutexGuard<'_, SessionData> {
        self.data.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn export(&self) -> Result<Vec<u8>, CoreError> {
        let d = self.lock();
        let stored = Stored {
            home_dc: d.home_dc,
            dc_options: d.dc_options.values().cloned().collect(),
            peers: d.peer_infos.values().cloned().collect(),
            updates: d.updates_state.clone(),
        };
        serde_json::to_vec(&stored).map_err(|e| CoreError::storage(e.to_string()))
    }

    fn import(raw: &[u8]) -> Result<SessionData, CoreError> {
        let stored: Stored = serde_json::from_slice(raw)
            .map_err(|_| CoreError::needs_attention("the saved Telegram session is damaged"))?;
        let mut data = SessionData {
            home_dc: stored.home_dc,
            ..SessionData::default()
        };
        for option in stored.dc_options {
            data.dc_options.insert(option.id, option);
        }
        for peer in stored.peers {
            data.peer_infos.insert(peer.id(), peer);
        }
        data.updates_state = stored.updates;
        Ok(data)
    }
}

impl Session for PersistedSession {
    type Error = Poisoned;

    fn home_dc_id(&self) -> Result<i32, Poisoned> {
        Ok(self.lock().home_dc)
    }

    fn set_home_dc_id(&self, dc_id: i32) -> BoxFuture<'_, Result<(), Poisoned>> {
        Box::pin(async move {
            self.lock().home_dc = dc_id;
            Ok(())
        })
    }

    fn dc_option(&self, dc_id: i32) -> Result<Option<DcOption>, Poisoned> {
        Ok(self.lock().dc_options.get(&dc_id).cloned())
    }

    fn set_dc_option(&self, dc_option: &DcOption) -> BoxFuture<'_, Result<(), Poisoned>> {
        let dc_option = dc_option.clone();
        Box::pin(async move {
            self.lock().dc_options.insert(dc_option.id, dc_option);
            Ok(())
        })
    }

    fn peer(&self, peer: PeerId) -> BoxFuture<'_, Result<Option<PeerInfo>, Poisoned>> {
        Box::pin(async move { Ok(self.lock().peer_infos.get(&peer).cloned()) })
    }

    fn cache_peer(&self, peer: &PeerInfo) -> BoxFuture<'_, Result<(), Poisoned>> {
        let peer = peer.clone();
        Box::pin(async move {
            self.lock().peer_infos.entry(peer.id()).or_insert_with(|| peer.clone()).extend_info(&peer);
            Ok(())
        })
    }

    fn updates_state(&self) -> BoxFuture<'_, Result<UpdatesState, Poisoned>> {
        Box::pin(async move { Ok(self.lock().updates_state.clone()) })
    }

    fn set_update_state(&self, update: UpdateState) -> BoxFuture<'_, Result<(), Poisoned>> {
        Box::pin(async move {
            let mut data = self.lock();
            match update {
                UpdateState::All(state) => data.updates_state = state,
                UpdateState::Primary {
                    pts,
                    date,
                    seq,
                } => {
                    data.updates_state.pts = pts;
                    data.updates_state.date = date;
                    data.updates_state.seq = seq;
                }
                UpdateState::Secondary {
                    qts,
                } => data.updates_state.qts = qts,
                UpdateState::Channel {
                    id,
                    pts,
                } => {
                    data.updates_state.channels.retain(|c| c.id != id);
                    data.updates_state.channels.push(ChannelState {
                        id,
                        pts,
                    });
                }
            }
            Ok(())
        })
    }
}

/// One connection to Telegram for one account.
struct Live {
    client: Client,
    session: Arc<PersistedSession>,
    runner: JoinHandle<()>,
}

impl Drop for Live {
    fn drop(&mut self) {
        self.runner.abort();
    }
}

struct PendingLogin {
    live: Arc<Live>,
    phone: String,
    token: Option<LoginToken>,
    password: Option<PasswordToken>,
}

pub struct Grammers {
    api_id: i32,
    api_hash: String,
    store: Arc<Store>,
    live: Mutex<HashMap<String, Arc<Live>>>,
    login: tokio::sync::Mutex<Option<PendingLogin>>,
}

fn map_error(e: &InvocationError) -> CoreError {
    if let InvocationError::Rpc(rpc) = e {
        let wait = rpc.value.unwrap_or(0);
        return match rpc.name.as_str() {
            "AUTH_KEY_UNREGISTERED"
            | "SESSION_REVOKED"
            | "SESSION_EXPIRED"
            | "USER_DEACTIVATED"
            | "USER_DEACTIVATED_BAN"
            | "AUTH_KEY_DUPLICATED" => CoreError::needs_attention("Telegram signed this phone out"),
            "FLOOD_WAIT" | "SLOWMODE_WAIT" | "FLOOD_PREMIUM_WAIT" => {
                CoreError::service(format!("Telegram asks to wait {wait} seconds before trying again"))
            }
            "PEER_ID_INVALID" | "CHAT_ID_INVALID" | "CHANNEL_INVALID" | "CHANNEL_PRIVATE" | "USER_ID_INVALID" => {
                CoreError::service("Telegram does not know that chat, or this account cannot see it")
            }
            "CHAT_WRITE_FORBIDDEN" | "USER_BANNED_IN_CHANNEL" | "CHAT_SEND_PLAIN_FORBIDDEN" => {
                CoreError::service("Telegram does not let this account write in that chat")
            }
            "PHONE_NUMBER_INVALID" | "PHONE_NUMBER_BANNED" => {
                CoreError::invalid("Telegram does not accept that phone number")
            }
            "PHONE_CODE_INVALID" | "PHONE_CODE_EXPIRED" | "PHONE_CODE_EMPTY" => {
                CoreError::invalid("That code is wrong or has expired. Ask for a new one.")
            }
            "PASSWORD_HASH_INVALID" => CoreError::invalid("Wrong password"),
            "API_ID_INVALID" | "API_ID_PUBLISHED_FLOOD" => {
                CoreError::service("Telegram rejected this app's credentials")
            }
            other => CoreError::service(format!("Telegram answered {other}")),
        };
    }
    CoreError::Network {
        reason: "could not reach Telegram".to_owned(),
    }
}

fn phone_account(raw: &str) -> Result<String, CoreError> {
    let digits: String = raw.chars().filter(char::is_ascii_digit).collect();
    if !(7..=15).contains(&digits.len()) {
        return Err(CoreError::invalid("Enter the phone number with its country code, like +1 555 010 0100"));
    }
    Ok(format!("+{digits}"))
}

async fn timed<T>(future: impl Future<Output = Result<T, InvocationError>>) -> Result<T, CoreError> {
    match tokio::time::timeout(TIMEOUT, future).await {
        Ok(result) => result.map_err(|e| map_error(&e)),
        Err(_) => Err(CoreError::Network {
            reason: "Telegram took too long to answer".to_owned(),
        }),
    }
}

impl Grammers {
    pub fn new(api_id: i32, api_hash: &str, store: Arc<Store>) -> Self {
        Self {
            api_id,
            api_hash: api_hash.to_owned(),
            store,
            live: Mutex::new(HashMap::new()),
            login: tokio::sync::Mutex::new(None),
        }
    }

    /// Whether this build carries Telegram's application credentials.
    pub fn configured(&self) -> bool {
        self.api_id > 0 && !self.api_hash.is_empty()
    }

    fn connect(&self, data: SessionData) -> Arc<Live> {
        let session = Arc::new(PersistedSession::new(data));
        let SenderPool {
            runner,
            handle,
            ..
        } = SenderPool::new(Arc::clone(&session), self.api_id);
        let client = Client::new(handle);
        Arc::new(Live {
            client,
            session,
            runner: tokio::spawn(async move {
                runner.run().await;
            }),
        })
    }

    fn live_for(&self, account: &str) -> Result<Arc<Live>, CoreError> {
        let mut live = self.live.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(existing) = live.get(account) {
            return Ok(Arc::clone(existing));
        }
        if !self.configured() {
            return Err(CoreError::service("Telegram is not set up in this build of Rewarden"));
        }
        let raw = self
            .store
            .secret_get(SERVICE, account)?
            .ok_or_else(|| CoreError::needs_attention("Telegram is not signed in for that account"))?;
        let created = self.connect(PersistedSession::import(&raw)?);
        live.insert(account.to_owned(), Arc::clone(&created));
        Ok(created)
    }

    /// Writes the session out, so a restart does not need a new login (which Telegram limits strictly).
    fn flush(&self, account: &str, live: &Live) {
        match live.session.export() {
            Ok(raw) => {
                if let Err(e) = self.store.secret_put(SERVICE, account, &raw) {
                    log::warn!("could not save the Telegram session: {e}");
                }
            }
            Err(e) => log::warn!("could not export the Telegram session: {e}"),
        }
    }

    async fn peer_ref(&self, live: &Live, chat: &Chat) -> Result<grammers_session::types::PeerRef, CoreError> {
        let id = chat
            .id
            .parse::<i64>()
            .ok()
            .and_then(PeerId::from_bot_api_dialog_id)
            .ok_or_else(|| CoreError::service("That is not a Telegram chat id"))?;
        Session::peer_ref(&*live.session, id)
            .await
            .map_err(|_| CoreError::storage("Telegram session unavailable"))?
            .ok_or_else(|| {
                CoreError::service("Telegram has not shown this account that chat yet; list the chats first")
            })
    }

    fn chat_of(peer: &Peer, last: Option<&grammers_client::message::Message>) -> Option<Chat> {
        let id = peer.id().bot_api_dialog_id()?.to_string();
        let (kind, title) = match peer {
            Peer::User(u) => (
                if u.is_bot() {
                    "bot"
                } else {
                    "user"
                },
                u.full_name(),
            ),
            Peer::Group(_) => ("group", peer.name().unwrap_or_default().to_owned()),
            Peer::Channel(_) => ("channel", peer.name().unwrap_or_default().to_owned()),
        };
        let title = if title.trim().is_empty() {
            "(no name)".to_owned()
        } else {
            title
        };
        Some(Chat {
            id,
            title,
            kind: kind.to_owned(),
            username: peer.username().map(str::to_owned),
            last: last.map(|m| m.text().to_owned()).unwrap_or_default(),
            date: last.map_or(0, |m| m.date().timestamp()),
        })
    }

    fn msg_of(m: &grammers_client::message::Message, chat_id: &str, chat_title: &str) -> Msg {
        let (from, from_id) = if m.outgoing() {
            ("me".to_owned(), 0)
        } else {
            (
                m.sender().and_then(Peer::name).unwrap_or_default().to_owned(),
                m.sender_id().and_then(PeerId::bot_api_dialog_id).unwrap_or(0),
            )
        };
        let mut body = text::neutralize(m.text());
        if body.trim().is_empty() && m.media().is_some() {
            "[media]".clone_into(&mut body);
        }
        Msg {
            id: i64::from(m.id()),
            chat: chat_id.to_owned(),
            chat_title: chat_title.to_owned(),
            from: if from.is_empty() {
                chat_title.to_owned()
            } else {
                from
            },
            from_id,
            text: body,
            date: m.date().timestamp(),
        }
    }
}

#[async_trait::async_trait]
impl TelegramBackend for Grammers {
    async fn dialogs(&self, account: &str, limit: usize) -> Result<Vec<Chat>, CoreError> {
        let live = self.live_for(account)?;
        let mut iter = live.client.iter_dialogs();
        let mut chats = Vec::new();
        while chats.len() < limit {
            let Some(dialog) = timed(iter.next()).await? else {
                break;
            };
            if let Some(chat) = Self::chat_of(dialog.peer(), dialog.last_message.as_ref()) {
                chats.push(chat);
            }
        }
        self.flush(account, &live);
        Ok(chats)
    }

    async fn resolve_username(&self, account: &str, username: &str) -> Result<Option<Chat>, CoreError> {
        let live = self.live_for(account)?;
        let found = timed(live.client.resolve_username(username)).await?;
        self.flush(account, &live);
        Ok(found.and_then(|p| Self::chat_of(&p, None)))
    }

    async fn messages(&self, account: &str, chat: &Chat, limit: usize) -> Result<Vec<Msg>, CoreError> {
        let live = self.live_for(account)?;
        let peer = self.peer_ref(&live, chat).await?;
        let mut iter = live.client.iter_messages(peer).limit(limit);
        let mut out = Vec::new();
        while let Some(m) = timed(iter.next()).await? {
            out.push(Self::msg_of(&m, &chat.id, &chat.title));
        }
        self.flush(account, &live);
        Ok(out)
    }

    async fn search(
        &self,
        account: &str,
        chat: Option<&Chat>,
        query: &str,
        limit: usize,
    ) -> Result<Vec<Msg>, CoreError> {
        let live = self.live_for(account)?;
        let mut out = Vec::new();
        if let Some(chat) = chat {
            let peer = self.peer_ref(&live, chat).await?;
            let mut iter = live.client.search_messages(peer).query(query).limit(limit);
            while let Some(m) = timed(iter.next()).await? {
                out.push(Self::msg_of(&m, &chat.id, &chat.title));
            }
        } else {
            let mut iter = live.client.search_all_messages().query(query).limit(limit);
            while let Some(m) = timed(iter.next()).await? {
                let (id, title) = m
                    .peer()
                    .and_then(|p| Self::chat_of(p, None))
                    .map_or_else(|| (String::new(), String::new()), |c| (c.id, c.title));
                out.push(Self::msg_of(&m, &id, &title));
            }
        }
        self.flush(account, &live);
        Ok(out)
    }

    async fn send(&self, account: &str, chat: &Chat, body: &str, reply_to: Option<i32>) -> Result<i64, CoreError> {
        let live = self.live_for(account)?;
        let peer = self.peer_ref(&live, chat).await?;
        let sent = timed(live.client.send_message(peer, InputMessage::new().text(body).reply_to(reply_to))).await?;
        self.flush(account, &live);
        Ok(i64::from(sent.id()))
    }

    fn configured(&self) -> bool {
        Grammers::configured(self)
    }

    async fn status(&self, account: &str) -> GmailStatus {
        let live = match self.live_for(account) {
            Ok(live) => live,
            Err(CoreError::ServiceNeedsAttention {
                ..
            }) => return GmailStatus::NeedsConsent,
            Err(e) => {
                return GmailStatus::Unavailable {
                    message: e.to_string(),
                };
            }
        };
        match timed(live.client.is_authorized()).await {
            Ok(true) => GmailStatus::Ready,
            Ok(false)
            | Err(CoreError::ServiceNeedsAttention {
                ..
            }) => GmailStatus::NeedsConsent,
            Err(e) => GmailStatus::Unavailable {
                message: e.to_string(),
            },
        }
    }

    async fn request_code(&self, phone: &str) -> Result<(), CoreError> {
        if !self.configured() {
            return Err(CoreError::service("Telegram is not set up in this build of Rewarden"));
        }
        let account = phone_account(phone)?;
        let live = self.connect(SessionData::default());
        let token = timed(live.client.request_login_code(&account, &self.api_hash)).await?;
        *self.login.lock().await = Some(PendingLogin {
            live,
            phone: account,
            token: Some(token),
            password: None,
        });
        Ok(())
    }

    async fn submit_code(&self, code: &str) -> Result<LoginProgress, CoreError> {
        let mut guard = self.login.lock().await;
        let pending = guard.as_mut().ok_or_else(|| CoreError::invalid("Ask for a code first"))?;
        let token = pending.token.as_ref().ok_or_else(|| CoreError::invalid("Ask for a code first"))?;
        let code: String = code.chars().filter(char::is_ascii_digit).collect();
        match tokio::time::timeout(TIMEOUT, pending.live.client.sign_in(token, &code)).await {
            Err(_) => Err(CoreError::Network {
                reason: "Telegram took too long to answer".to_owned(),
            }),
            Ok(Ok(_user)) => {
                let account = pending.phone.clone();
                let done = guard.take().ok_or_else(|| CoreError::invalid("Ask for a code first"))?;
                self.flush(&account, &done.live);
                self.live.lock().unwrap_or_else(PoisonError::into_inner).insert(account.clone(), done.live);
                Ok(LoginProgress::Done {
                    account,
                })
            }
            Ok(Err(SignInError::PasswordRequired(password))) => {
                let hint = password.hint().map(str::to_owned);
                pending.password = Some(password);
                Ok(LoginProgress::NeedsPassword {
                    hint,
                })
            }
            Ok(Err(SignInError::InvalidCode)) => {
                Err(CoreError::invalid("That code is wrong or has expired. Ask for a new one."))
            }
            Ok(Err(SignInError::SignUpRequired)) => {
                Err(CoreError::invalid("That number has no Telegram account. Create one in the Telegram app first."))
            }
            Ok(Err(SignInError::InvalidPassword(_))) => Err(CoreError::invalid("Wrong password")),
            Ok(Err(SignInError::Other(e))) => Err(map_error(&e)),
        }
    }

    async fn submit_password(&self, password: &str) -> Result<String, CoreError> {
        let mut guard = self.login.lock().await;
        let pending = guard.as_mut().ok_or_else(|| CoreError::invalid("Ask for a code first"))?;
        let token = pending.password.take().ok_or_else(|| CoreError::invalid("No password is needed"))?;
        match tokio::time::timeout(TIMEOUT, pending.live.client.check_password(token, password)).await {
            Err(_) => Err(CoreError::Network {
                reason: "Telegram took too long to answer".to_owned(),
            }),
            Ok(Ok(_user)) => {
                let account = pending.phone.clone();
                let done = guard.take().ok_or_else(|| CoreError::invalid("Ask for a code first"))?;
                self.flush(&account, &done.live);
                self.live.lock().unwrap_or_else(PoisonError::into_inner).insert(account.clone(), done.live);
                Ok(account)
            }
            Ok(Err(SignInError::InvalidPassword(again))) => {
                pending.password = Some(again);
                Err(CoreError::invalid("Wrong password"))
            }
            Ok(Err(SignInError::Other(e))) => Err(map_error(&e)),
            Ok(Err(other)) => Err(CoreError::invalid(other.to_string())),
        }
    }

    async fn sign_out(&self, account: &str) -> Result<(), CoreError> {
        let live = self.live.lock().unwrap_or_else(PoisonError::into_inner).remove(account);
        if let Some(live) = live {
            // Best effort: the session is gone from this phone either way.
            tokio::time::timeout(Duration::from_secs(10), live.client.sign_out()).await.ok();
        }
        self.store.secret_delete(SERVICE, account)
    }
}
