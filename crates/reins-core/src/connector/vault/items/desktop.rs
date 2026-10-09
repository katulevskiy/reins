//! What only the paired Reins desktop app may ask of the vault (the flow has checked its pinned key):
//!
//! - `secret_release`: secrets for one command or API route (`reins run`, the API proxy), named `item/field`. The
//!   user sees the command, the purpose, the items' names and the fields' names, never a value; the values go to the
//!   app as a [`SecretGrant`] sealed to its key, to be forgotten when the lease ends. Permissions: the item, or
//!   `secrets` when several items are named.
//! - `ssh_keys`: the public halves of the vault's SSH keys, for the app's SSH agent.
//! - `phone_key`, `secret_store`, `names`: `reins vault add` and `reins vault list` ([`store`]).
//! - `ssh_sign`: one SSH sign-in, signed here with the vault's private key (which never leaves the phone) and sealed to
//!   the app as an [`SshSignature`]. Only an SSH user authentication request is signed, and only with the key it names,
//!   so that "Sign in to <host> with <key>" is exactly what happens. Permissions: `<fingerprint>@<host>`, within
//!   `<fingerprint>`; a sign-in to a server nobody named is asked for every time.

use data_encoding::{BASE64, BASE64_NOPAD};
use reins_proto::connector::{ConnectorCall, VAULT};
use reins_proto::desktop::{SEALED_FIELD, SecretGrant, SshSignature};
use serde_json::{Map, Value, json};
use signature::{RandomizedSigner as _, SignatureEncoding as _, Signer as _};
use ssh_key::private::KeypairData;
use ssh_key::sha2::{Digest as _, Sha256, Sha512};
use ssh_key::{EcdsaCurve, HashAlg, PrivateKey};
use zeroize::Zeroizing;

use super::model::{Entry, Kind, Snapshot, State};
use crate::connector::sealed::{client_key, nonce_arg, seal};
use crate::connector::vault::Vault;
use crate::connector::vault::totp::{totp_code, unix_seconds};
use crate::connector::{Item, Preview};
use crate::store::unix_now;
use crate::types::{SecretReleaseView, SshSignView};
use crate::{CoreError, text};

mod store;

/// The resource of a release that names several items.
const SECRETS: &str = "secrets";
/// The resource of the list of SSH keys.
const SSH_KEYS: &str = "ssh_keys";
const DEFAULT_LEASE: i64 = 3_600;
const MIN_LEASE: i64 = 60;
const MAX_LEASE: i64 = 86_400;
const MAX_SECRETS: usize = 10;
/// SSH agent flags (draft-miller-ssh-agent): which hash an RSA signature uses.
const RSA_SHA2_256: i64 = 2;
const RSA_SHA2_512: i64 = 4;
/// SSH_MSG_USERAUTH_REQUEST.
const USERAUTH_REQUEST: u8 = 50;
const MAX_HOST: usize = 140;

fn bad(message: impl Into<String>) -> CoreError {
    CoreError::service(message)
}

/// Lists and reads. `None` when the operation is not one of this area's.
pub(in crate::connector::vault) async fn fetch(
    vault: &Vault,
    account: &str,
    call: &ConnectorCall,
) -> Option<Result<Vec<Item>, CoreError>> {
    (call.op == "ssh_keys").then_some(())?;
    Some(ssh_keys(vault, account).await)
}

/// What a write would do. `None` when the operation is not one of this area's.
pub(in crate::connector::vault) async fn preview(
    vault: &Vault,
    account: &str,
    call: &ConnectorCall,
) -> Option<Result<Preview, CoreError>> {
    Some(match call.op.as_str() {
        "secret_release" => preview_release(vault, account, call).await,
        "ssh_sign" => preview_sign(vault, account, call).await,
        "phone_key" => store::preview_phone_key(vault, call),
        "secret_store" => store::preview_store(vault, account, call).await,
        "names" => store::preview_names(vault, account, call).await,
        _ => return None,
    })
}

/// Does the write. `None` when the operation is not one of this area's.
pub(in crate::connector::vault) async fn perform(
    vault: &Vault,
    account: &str,
    call: &ConnectorCall,
) -> Option<Result<Value, CoreError>> {
    Some(match call.op.as_str() {
        "secret_release" => perform_release(vault, account, call).await,
        "ssh_sign" => perform_sign(vault, account, call).await,
        "phone_key" => store::perform_phone_key(vault, call),
        "secret_store" => store::perform_store(vault, account, call).await,
        "names" => store::perform_names(vault, account, call).await,
        _ => return None,
    })
}

// ---- secrets --------------------------------------------------------------------------------------------------------

/// A field of an item a release can name.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Field {
    Password,
    Username,
    /// The current one-time code.
    Totp,
    Notes,
    /// The first website.
    Uri,
    /// A custom field, by name.
    Custom(String),
}

impl Field {
    fn parse(name: &str) -> Self {
        match name.to_ascii_lowercase().as_str() {
            "password" => Self::Password,
            "username" => Self::Username,
            "totp" => Self::Totp,
            "notes" => Self::Notes,
            "uri" => Self::Uri,
            _ => Self::Custom(name.to_owned()),
        }
    }

    fn label(&self) -> String {
        match self {
            Self::Password => "password".to_owned(),
            Self::Username => "username".to_owned(),
            Self::Totp => "one-time code".to_owned(),
            Self::Notes => "notes".to_owned(),
            Self::Uri => "website".to_owned(),
            Self::Custom(name) => text::one_line(name),
        }
    }

    /// The value in `entry`; an error names the item and the field, never a value.
    fn value(&self, entry: &Entry) -> Result<Zeroizing<String>, CoreError> {
        let name = entry.name();
        let missing = || bad(format!("{name} has no {}.", self.label()));
        let login = || {
            if entry.kind == Kind::Login {
                Ok(())
            } else {
                Err(bad(format!("{name} is not a login, so it has no {}.", self.label())))
            }
        };
        let present = |v: Option<Zeroizing<String>>| v.filter(|v| !v.is_empty()).ok_or_else(missing);
        match self {
            Self::Password => login().and_then(|()| present(entry.dec("/login/password"))),
            Self::Username => login().and_then(|()| present(entry.dec("/login/username"))),
            Self::Totp => {
                login()?;
                let stored = present(entry.dec("/login/totp"))?;
                Ok(Zeroizing::new(totp_code(&stored, unix_seconds())?.0))
            }
            Self::Notes => present(entry.dec("/notes")),
            Self::Uri => login().and_then(|()| entry.uris().into_iter().next().map(Zeroizing::new).ok_or_else(missing)),
            Self::Custom(wanted) => {
                let fields = entry.custom_fields();
                let found = fields
                    .iter()
                    .find(|c| c.name == *wanted)
                    .or_else(|| fields.iter().find(|c| c.name.eq_ignore_ascii_case(wanted)))
                    .ok_or_else(|| bad(format!("{name} has no field named {}.", text::one_line(wanted))))?;
                Ok(found.value.clone())
            }
        }
    }
}

/// One `item/field` reference, resolved: the item (an index into the snapshot) and the field.
struct Wanted {
    reference: String,
    entry: usize,
    field: Field,
}

/// The items a reference's item part names: an id, else an exact name; never one in the trash.
fn named(snap: &Snapshot, item: &str) -> Vec<usize> {
    let usable = |e: &Entry| e.state() != State::Trash;
    let by_id: Vec<usize> =
        snap.entries.iter().enumerate().filter(|(_, e)| usable(e) && e.id == item).map(|(i, _)| i).collect();
    if !by_id.is_empty() {
        return by_id;
    }
    snap.entries.iter().enumerate().filter(|(_, e)| usable(e) && e.name() == item).map(|(i, _)| i).collect()
}

/// Resolves `item/field`. Item names and field names may contain `/`: every split is tried, and exactly one must name
/// an item.
fn resolve(snap: &Snapshot, reference: &str) -> Result<Wanted, CoreError> {
    let shown = text::truncate_chars(&text::one_line(reference), 100);
    let mut found: Vec<(usize, &str)> = Vec::new();
    for (at, _) in reference.match_indices('/') {
        let (item, field) = (&reference[..at], &reference[at + 1..]);
        if item.is_empty() || field.is_empty() {
            continue;
        }
        found.extend(named(snap, item).into_iter().map(|i| (i, field)));
    }
    match found.as_slice() {
        [] if !reference.contains('/') => Err(bad(format!("`{shown}` must be item/field."))),
        [] => Err(bad(format!("No vault item matches `{shown}`. Name an item by its id or its exact name."))),
        [(entry, field)] => Ok(Wanted {
            reference: reference.to_owned(),
            entry: *entry,
            field: Field::parse(field),
        }),
        _ => Err(bad(format!("`{shown}` could mean several vault items. Use the item's id."))),
    }
}

struct Release {
    wanted: Vec<Wanted>,
    command: String,
    purpose: Option<String>,
    lease: i64,
}

fn release_args(snap: &Snapshot, call: &ConnectorCall) -> Result<Release, CoreError> {
    client_key(call)?;
    nonce_arg(call)?;
    let refs = call.list_arg("secrets");
    if refs.is_empty() || refs.len() > MAX_SECRETS {
        return Err(bad(format!("`secrets` names 1 to {MAX_SECRETS} item/field references.")));
    }
    let command = text::one_line(call.str_arg("command").unwrap_or_default());
    if command.is_empty() {
        return Err(bad("`command` is required."));
    }
    let lease = call.int_arg("lease_secs").unwrap_or(DEFAULT_LEASE);
    if !(MIN_LEASE..=MAX_LEASE).contains(&lease) {
        return Err(bad(format!("`lease_secs` must be {MIN_LEASE} to {MAX_LEASE}.")));
    }
    let wanted = refs.iter().map(|r| resolve(snap, r)).collect::<Result<Vec<_>, _>>()?;
    Ok(Release {
        wanted,
        command,
        purpose: call.str_arg("purpose").map(text::one_line).filter(|p| !p.is_empty()),
        lease,
    })
}

fn secret_count(call: &ConnectorCall) -> usize {
    call.list_arg("secrets").len()
}

async fn preview_release(vault: &Vault, account: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let snap = Snapshot::load(vault, account).await?;
    let release = release_args(&snap, call)?;
    let mut lines = vec![format!(
        "Use {} for {}",
        if release.wanted.len() == 1 {
            "1 secret".to_owned()
        } else {
            format!("{} secrets", release.wanted.len())
        },
        release.command
    )];
    // One line per secret, in order (`secret_release_view` reads them back): what it is, never its value. Each is
    // checked to exist now, so that a mistake is told before the user is asked.
    for w in &release.wanted {
        let entry = &snap.entries[w.entry];
        w.field.value(entry)?;
        lines.push(format!("{} \u{b7} {}", entry.name(), w.field.label()));
    }
    if let Some(p) = &release.purpose {
        lines.push(format!("Why: {p}"));
    }
    lines.push(format!(
        "The computer keeps them for {}",
        crate::views::duration_text(u64::try_from(release.lease).unwrap_or(0))
    ));
    let mut items: Vec<usize> = release.wanted.iter().map(|w| w.entry).collect();
    items.sort_unstable();
    items.dedup();
    let (resource, resource_label, parents) = match items.as_slice() {
        [only] => {
            let entry = &snap.entries[*only];
            (entry.resource(), entry.name(), snap.parents(entry.folder(), entry.kind))
        }
        _ => (SECRETS.to_owned(), format!("{} vault items", items.len()), Vec::new()),
    };
    Ok(Preview {
        resource,
        resource_label,
        lines,
        parents,
        once_only: false,
        ..Preview::default()
    })
}

async fn perform_release(vault: &Vault, account: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    // A fresh read: the values are what the vault holds now (a one-time code is the current one).
    let snap = Snapshot::load(vault, account).await?;
    let release = release_args(&snap, call)?;
    let mut secrets = Vec::with_capacity(release.wanted.len());
    for w in &release.wanted {
        let value = w.field.value(&snap.entries[w.entry])?;
        secrets.push((w.reference.clone(), value.to_string()));
    }
    let expires_at = unix_now().saturating_add(release.lease);
    let grant = SecretGrant {
        v: 1,
        nonce: nonce_arg(call)?,
        secrets,
        expires_at,
    };
    let sealed = seal(client_key(call)?, &grant);
    // The grant holds the values in the clear: wipe them.
    for (_, value) in grant.secrets {
        drop(Zeroizing::new(value));
    }
    Ok(json!({ SEALED_FIELD: sealed?, "expires_at": expires_at }))
}

/// The release as the approval shows it (`None` for anything else).
pub fn secret_release_view(call: &ConnectorCall, preview: &Preview) -> Option<SecretReleaseView> {
    (call.service == VAULT && call.op == "secret_release").then_some(())?;
    let n = secret_count(call);
    Some(SecretReleaseView {
        command: text::one_line(call.str_arg("command").unwrap_or_default()),
        purpose: call.str_arg("purpose").map(text::one_line).filter(|p| !p.is_empty()),
        items: preview.lines.iter().skip(1).take(n).map(|l| text::one_line(l)).collect(),
        lease_secs: u64::try_from(call.int_arg("lease_secs").unwrap_or(DEFAULT_LEASE)).unwrap_or(0),
    })
}

// ---- SSH keys -------------------------------------------------------------------------------------------------------

/// A vault SSH key this phone can sign with.
struct SshKey<'a> {
    entry: &'a Entry,
    private: PrivateKey,
    fingerprint: String,
}

impl SshKey<'_> {
    /// `ssh-ed25519 AAAA…`, without a comment.
    fn public_line(&self) -> Result<String, CoreError> {
        let mut public = self.private.public_key().clone();
        public.set_comment("");
        public.to_openssh().map_err(|_| bad("An SSH key could not be written out."))
    }

    fn kind(&self) -> &'static str {
        match self.private.key_data() {
            KeypairData::Ed25519(_) => "ED25519",
            KeypairData::Rsa(_) => "RSA",
            KeypairData::Ecdsa(k) if k.curve() == EcdsaCurve::NistP256 => "ECDSA P-256",
            KeypairData::Ecdsa(_) => "ECDSA P-384",
            _ => "SSH",
        }
    }
}

/// Whether the phone can sign with a key of this kind.
fn supported(key: &PrivateKey) -> bool {
    match key.key_data() {
        KeypairData::Ed25519(_) => true,
        // At least 2048 bits: shorter RSA keys are refused by servers and not worth signing with.
        KeypairData::Rsa(k) => k.public.n.as_positive_bytes().is_some_and(|n| n.len() >= 256),
        KeypairData::Ecdsa(k) => matches!(k.curve(), EcdsaCurve::NistP256 | EcdsaCurve::NistP384),
        _ => false,
    }
}

/// The vault's SSH keys in use that the phone can sign with (an encrypted or unknown key is left out).
fn ssh_keys_of(snap: &Snapshot) -> Vec<SshKey<'_>> {
    snap.entries
        .iter()
        .filter(|e| e.kind == Kind::SshKey && e.state() == State::Active)
        .filter_map(|entry| {
            let pem = entry.dec("/sshKey/privateKey")?;
            let private = PrivateKey::from_openssh(pem.as_bytes()).ok()?;
            if private.is_encrypted() || !supported(&private) {
                return None;
            }
            let fingerprint = private.public_key().fingerprint(HashAlg::Sha256).to_string();
            Some(SshKey {
                entry,
                private,
                fingerprint,
            })
        })
        .collect()
}

async fn ssh_keys(vault: &Vault, account: &str) -> Result<Vec<Item>, CoreError> {
    let snap = Snapshot::load(vault, account).await?;
    let mut items = Vec::new();
    for key in ssh_keys_of(&snap) {
        let name = key.entry.name();
        let mut extra = Map::new();
        extra.insert("name".to_owned(), json!(name));
        extra.insert("fingerprint".to_owned(), json!(key.fingerprint));
        extra.insert("public_key".to_owned(), json!(key.public_line()?));
        extra.insert("type".to_owned(), json!(key.kind()));
        items.push(Item {
            id: key.fingerprint.clone(),
            resource: SSH_KEYS.to_owned(),
            resource_label: "Your SSH keys (public halves)".to_owned(),
            title: name,
            snippet: format!("{} {}", key.kind(), key.fingerprint),
            extra,
            ..Item::default()
        });
    }
    items.sort_by_key(|i| i.title.to_lowercase());
    Ok(items)
}

// ---- SSH signing ----------------------------------------------------------------------------------------------------

/// Reads one SSH `string`.
fn take_string<'a>(buf: &mut &'a [u8]) -> Option<&'a [u8]> {
    let (len, rest) = buf.split_first_chunk::<4>()?;
    let len = usize::try_from(u32::from_be_bytes(*len)).ok()?;
    let (s, rest) = (rest.get(..len)?, rest.get(len..)?);
    *buf = rest;
    Some(s)
}

/// What an SSH client asks an agent to sign for a sign-in (RFC 4252 §7, and OpenSSH's host-bound variant).
#[derive(Debug, PartialEq, Eq)]
struct Userauth {
    user: String,
    /// The public key blob the request is for.
    key_blob: Vec<u8>,
    /// The server's host key fingerprint, when the request is bound to it.
    host_key: Option<String>,
}

fn fingerprint_of(blob: &[u8]) -> String {
    format!("SHA256:{}", BASE64_NOPAD.encode(&Sha256::digest(blob)))
}

fn parse_userauth(data: &[u8]) -> Option<Userauth> {
    let mut buf = data;
    let session = take_string(&mut buf)?;
    let (&kind, rest) = buf.split_first()?;
    buf = rest;
    let user = std::str::from_utf8(take_string(&mut buf)?).ok()?;
    let service = take_string(&mut buf)?;
    let method = take_string(&mut buf)?;
    let (&has_signature, rest) = buf.split_first()?;
    buf = rest;
    let _algorithm = take_string(&mut buf)?;
    let key_blob = take_string(&mut buf)?.to_vec();
    let bound = method == b"publickey-hostbound-v00@openssh.com";
    let host_key = if bound {
        Some(fingerprint_of(take_string(&mut buf)?))
    } else {
        None
    };
    let ok = !session.is_empty()
        && kind == USERAUTH_REQUEST
        && service == b"ssh-connection"
        && (method == b"publickey" || bound)
        && has_signature != 0
        && buf.is_empty()
        && !user.chars().any(char::is_control);
    ok.then(|| Userauth {
        user: user.to_owned(),
        key_blob,
        host_key,
    })
}

/// A host name as the desktop app sends it: lower case, no spaces or control characters, no `@` or `/`.
fn host_arg(call: &ConnectorCall) -> Result<Option<String>, CoreError> {
    match call.str_arg("host").map(str::trim) {
        None | Some("") => Ok(None),
        Some(h)
            if h.len() <= MAX_HOST
                && h.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b':' | b'[' | b']')) =>
        {
            Ok(Some(h.to_ascii_lowercase()))
        }
        Some(_) => Err(bad("`host` must be a host name or address.")),
    }
}

fn host_key_arg(call: &ConnectorCall) -> Result<Option<String>, CoreError> {
    match call.str_arg("host_key").map(str::trim) {
        None | Some("") => Ok(None),
        Some(k)
            if k.len() <= 100
                && k.strip_prefix("SHA256:").is_some_and(|b| {
                    !b.is_empty() && b.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'/' | b'='))
                }) =>
        {
            Ok(Some(k.trim_end_matches('=').to_owned()))
        }
        Some(_) => Err(bad("`host_key` must be a SHA256 fingerprint (SHA256:…).")),
    }
}

/// A sign-in request, checked against the arguments: the data, its user, and the server it is for.
struct SignIn {
    data: Zeroizing<Vec<u8>>,
    auth: Userauth,
    host: Option<String>,
    host_key: Option<String>,
}

impl SignIn {
    /// The server, as the user can recognize it.
    fn target(&self) -> Option<&str> {
        self.host.as_deref().or(self.host_key.as_deref())
    }
}

fn sign_in_args(call: &ConnectorCall) -> Result<SignIn, CoreError> {
    let encoded = call.str_arg("data_base64").unwrap_or_default().trim();
    let data = Zeroizing::new(
        BASE64
            .decode(encoded.as_bytes())
            .or_else(|_| BASE64_NOPAD.decode(encoded.as_bytes()))
            .map_err(|_| bad("`data_base64` is not base64."))?,
    );
    let auth = parse_userauth(&data).ok_or_else(|| {
        bad("Only an SSH sign-in (a user authentication request) can be signed with a vault key, and this is not one.")
    })?;
    let host = host_arg(call)?;
    let given = host_key_arg(call)?;
    let host_key = match (given, &auth.host_key) {
        (Some(g), Some(bound)) if g != *bound => {
            return Err(bad("The server's host key differs from the one the sign-in is bound to."));
        }
        (given, bound) => given.or_else(|| bound.clone()),
    };
    Ok(SignIn {
        data,
        auth,
        host,
        host_key,
    })
}

/// The vault key a call names by fingerprint, checked against the key the sign-in is for.
fn signing_key<'a>(
    keys: &'a [SshKey<'a>],
    call: &ConnectorCall,
    sign_in: &SignIn,
) -> Result<&'a SshKey<'a>, CoreError> {
    let wanted = call.str_arg("key").unwrap_or_default().trim().trim_end_matches('=');
    let key = keys
        .iter()
        .find(|k| k.fingerprint == wanted)
        .ok_or_else(|| bad("No SSH key with that fingerprint is in the vault. Use vault_ssh_keys to list them."))?;
    let blob = key.private.public_key().to_bytes().map_err(|_| bad("The SSH key could not be read."))?;
    if blob != sign_in.auth.key_blob {
        return Err(bad("The sign-in is for another key than the one named."));
    }
    Ok(key)
}

async fn preview_sign(vault: &Vault, account: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    client_key(call)?;
    nonce_arg(call)?;
    let sign_in = sign_in_args(call)?;
    let snap = Snapshot::load(vault, account).await?;
    let keys = ssh_keys_of(&snap);
    let key = signing_key(&keys, call, &sign_in)?;
    rsa_hash(&key.private, call)?;
    let name = key.entry.name();
    let fp = &key.fingerprint;
    let mut lines = vec![format!("Sign in to {} with {name}", sign_in.target().unwrap_or("an unknown server"))];
    lines.push(format!("As user {}", text::one_line(&sign_in.auth.user)));
    if let (Some(_), Some(host_key)) = (&sign_in.host, &sign_in.host_key) {
        lines.push(format!("Server key: {host_key}"));
    }
    Ok(match sign_in.target() {
        Some(target) => Preview {
            resource: format!("{fp}@{target}"),
            resource_label: format!("{name} on {target}"),
            lines,
            parents: vec![(fp.clone(), name)],
            once_only: false,
            ..Preview::default()
        },
        // Nobody said which server: never remembered.
        None => Preview {
            resource: fp.clone(),
            resource_label: name,
            lines,
            parents: Vec::new(),
            once_only: true,
            ..Preview::default()
        },
    })
}

/// The hash an RSA signature uses, from the agent flags; SHA-1 (`ssh-rsa`) is not made.
fn rsa_hash(key: &PrivateKey, call: &ConnectorCall) -> Result<Option<HashAlg>, CoreError> {
    if !matches!(key.key_data(), KeypairData::Rsa(_)) {
        return Ok(None);
    }
    let flags = call.int_arg("flags").unwrap_or(0);
    if flags & RSA_SHA2_512 != 0 {
        Ok(Some(HashAlg::Sha512))
    } else if flags & RSA_SHA2_256 != 0 {
        Ok(Some(HashAlg::Sha256))
    } else {
        Err(bad(
            "RSA keys sign with rsa-sha2-256 or rsa-sha2-512 only, not SHA-1: the server must accept one of those.",
        ))
    }
}

/// An SSH signature blob: `string algorithm, string signature`.
fn signature_blob(algorithm: &str, signature: &[u8]) -> Result<Vec<u8>, CoreError> {
    let len = |n: usize| u32::try_from(n).map(u32::to_be_bytes).map_err(|_| bad("The signature is too large."));
    let mut out = Vec::with_capacity(8 + algorithm.len() + signature.len());
    out.extend_from_slice(&len(algorithm.len())?);
    out.extend_from_slice(algorithm.as_bytes());
    out.extend_from_slice(&len(signature.len())?);
    out.extend_from_slice(signature);
    Ok(out)
}

/// The RSA private key of an SSH key pair. (`ssh-key` 0.6.7's own conversion passes `p` twice instead of `p` and `q`,
/// so it is done here.)
fn rsa_private(k: &ssh_key::private::RsaKeypair) -> Option<rsa::RsaPrivateKey> {
    let int = |m: &ssh_key::Mpint| m.as_positive_bytes().map(rsa::BigUint::from_bytes_be);
    let key = rsa::RsaPrivateKey::from_components(
        int(&k.public.n)?,
        int(&k.public.e)?,
        int(&k.private.d)?,
        vec![int(&k.private.p)?, int(&k.private.q)?],
    )
    .ok()?;
    key.validate().ok()?;
    Some(key)
}

/// Signs `data` with `key`: Ed25519, ECDSA (P-256, P-384), or RSA with the hash given.
fn sign(key: &PrivateKey, data: &[u8], hash: Option<HashAlg>) -> Result<Vec<u8>, CoreError> {
    let failed = || bad("The SSH key could not sign.");
    let signature: ssh_key::Signature = match key.key_data() {
        KeypairData::Ed25519(k) => k.try_sign(data).map_err(|_| failed())?,
        KeypairData::Ecdsa(k) => k.try_sign(data).map_err(|_| failed())?,
        KeypairData::Rsa(k) => {
            let private = rsa_private(k).ok_or_else(failed)?;
            let rng = &mut crypto_box::aead::OsRng;
            let (algorithm, bytes) = match hash {
                Some(HashAlg::Sha256) => (
                    "rsa-sha2-256",
                    rsa::pkcs1v15::SigningKey::<Sha256>::new(private)
                        .try_sign_with_rng(rng, data)
                        .map_err(|_| failed())?
                        .to_vec(),
                ),
                _ => (
                    "rsa-sha2-512",
                    rsa::pkcs1v15::SigningKey::<Sha512>::new(private)
                        .try_sign_with_rng(rng, data)
                        .map_err(|_| failed())?
                        .to_vec(),
                ),
            };
            return signature_blob(algorithm, &bytes);
        }
        _ => return Err(bad("The phone cannot sign with this kind of SSH key.")),
    };
    signature_blob(signature.algorithm().as_str(), signature.as_bytes())
}

async fn perform_sign(vault: &Vault, account: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let sign_in = sign_in_args(call)?;
    let snap = Snapshot::load(vault, account).await?;
    let keys = ssh_keys_of(&snap);
    let key = signing_key(&keys, call, &sign_in)?;
    let blob = sign(&key.private, &sign_in.data, rsa_hash(&key.private, call)?)?;
    let answer = SshSignature {
        v: 1,
        nonce: nonce_arg(call)?,
        signature_base64: BASE64.encode(&blob),
    };
    Ok(json!({ SEALED_FIELD: seal(client_key(call)?, &answer)? }))
}

/// The sign-in as the approval shows it (`None` for anything else).
pub fn ssh_sign_view(call: &ConnectorCall, preview: &Preview) -> Option<SshSignView> {
    (call.service == VAULT && call.op == "ssh_sign").then_some(())?;
    let sign_in = sign_in_args(call).ok();
    Some(SshSignView {
        key_name: text::one_line(preview.parents.first().map_or(&preview.resource_label, |(_, name)| name)),
        key_fingerprint: text::one_line(call.str_arg("key").unwrap_or_default()),
        host: sign_in.as_ref().and_then(|s| s.host.clone()),
        host_key: sign_in.and_then(|s| s.host_key),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn string(out: &mut Vec<u8>, s: &[u8]) {
        out.extend_from_slice(&u32::try_from(s.len()).unwrap().to_be_bytes());
        out.extend_from_slice(s);
    }

    fn request(method: &str, service: &str, host_key: Option<&[u8]>) -> Vec<u8> {
        let mut out = Vec::new();
        string(&mut out, &[7; 32]);
        out.push(USERAUTH_REQUEST);
        string(&mut out, b"git");
        string(&mut out, service.as_bytes());
        string(&mut out, method.as_bytes());
        out.push(1);
        string(&mut out, b"ssh-ed25519");
        string(&mut out, b"KEYBLOB");
        if let Some(k) = host_key {
            string(&mut out, k);
        }
        out
    }

    #[test]
    fn sign_in_requests_are_recognised_and_anything_else_is_not() {
        let plain = parse_userauth(&request("publickey", "ssh-connection", None)).unwrap();
        assert_eq!((plain.user.as_str(), plain.key_blob.as_slice(), plain.host_key), ("git", &b"KEYBLOB"[..], None));
        let bound =
            parse_userauth(&request("publickey-hostbound-v00@openssh.com", "ssh-connection", Some(b"HOST"))).unwrap();
        assert_eq!(bound.host_key, Some(fingerprint_of(b"HOST")));
        assert!(parse_userauth(&request("publickey", "other", None)).is_none());
        assert!(parse_userauth(&request("password", "ssh-connection", None)).is_none());
        let mut trailing = request("publickey", "ssh-connection", None);
        trailing.push(0);
        assert!(parse_userauth(&trailing).is_none());
        assert!(parse_userauth(b"SSHSIG\0\0\0\x01").is_none());
        assert!(parse_userauth(&[]).is_none());
    }

    #[test]
    fn fields_are_named_in_any_case_and_others_are_custom() {
        assert_eq!(Field::parse("Password"), Field::Password);
        assert_eq!(Field::parse("TOTP"), Field::Totp);
        assert_eq!(Field::parse("API key"), Field::Custom("API key".to_owned()));
        assert_eq!(Field::Uri.label(), "website");
    }

    #[test]
    fn every_supported_kind_of_key_signs() {
        for (pem, hash, alg) in [
            (include_str!("../../../../tests/fixtures/ssh/k_ed"), None, "ssh-ed25519"),
            (include_str!("../../../../tests/fixtures/ssh/k_rsa"), Some(HashAlg::Sha256), "rsa-sha2-256"),
            (include_str!("../../../../tests/fixtures/ssh/k_rsa"), Some(HashAlg::Sha512), "rsa-sha2-512"),
            (include_str!("../../../../tests/fixtures/ssh/k_p256"), None, "ecdsa-sha2-nistp256"),
            (include_str!("../../../../tests/fixtures/ssh/k_p384"), None, "ecdsa-sha2-nistp384"),
        ] {
            let key = PrivateKey::from_openssh(pem).unwrap();
            assert!(supported(&key), "{alg}");
            let blob = sign(&key, b"data", hash).unwrap_or_else(|e| panic!("{alg}: {e}"));
            let mut rest = blob.as_slice();
            assert_eq!(take_string(&mut rest).unwrap(), alg.as_bytes());
            assert!(!take_string(&mut rest).unwrap().is_empty() && rest.is_empty());
        }
    }

    #[test]
    fn signature_blobs_are_two_strings() {
        assert_eq!(signature_blob("x", &[1, 2]).unwrap(), [0, 0, 0, 1, b'x', 0, 0, 0, 2, 1, 2]);
    }
}
