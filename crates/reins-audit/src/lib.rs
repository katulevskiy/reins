//! Verifies a Reins organization's audit export (`docs/design/decisions-v2/06-trust-and-compliance.md` §5.2): the core
//! of `reins-audit verify`.
//!
//! An export is JSON Lines of [`ExportLine`]s: the events of one org in order, the signed checkpoints after the event
//! they cover, and a signed head as the last line. [`verify`] proves, with only the decision service's public key:
//!
//! - **nothing was edited:** every event's hash is recomputed ([`reins_proto::org::event_hash`]);
//! - **nothing was removed, inserted or reordered:** sequence numbers run without a gap and each event names its
//!   predecessor's hash, from the genesis hash or from the export's first signed checkpoint;
//! - **the tail wasn't cut:** the signed head covers the last event;
//! - **every checkpoint is on this chain** and signed by a trusted key;
//! - **the chain wasn't rewritten since an earlier checkpoint** (`anchor`, from an earlier export or an external
//!   witness): the export still has that checkpoint's hash at its sequence number. A server holding the decision
//!   service key could re-sign a rewritten chain; it can't make it agree with a checkpoint someone kept.

pub mod cli;
pub mod evidence;

use std::collections::BTreeMap;
use std::io::BufRead;

use reins_proto::canon::parse_sig;
use reins_proto::managed::{AuditEvent, Checkpoint, Envelope, EnvelopeKind, ExportLine, envelope_message};
use reins_proto::org::GENESIS_HASH;
use ring::signature::{ED25519, UnparsedPublicKey};

/// A public key the verifier trusts (the decision service's, for checkpoints and heads).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustedKey {
    pub key_id: String,
    pub public: [u8; 32],
}

impl TrustedKey {
    /// Reads `<key_id>=<base64url or base64>` (as `reins-audit verify --key` takes it and the console shows it).
    #[must_use]
    pub fn parse(spec: &str) -> Option<Self> {
        let (id, key) = spec.split_once('=')?;
        let key = key.trim().trim_end_matches('=');
        let bytes = data_encoding::BASE64URL_NOPAD
            .decode(key.as_bytes())
            .or_else(|_| data_encoding::BASE64_NOPAD.decode(key.as_bytes()))
            .ok()?;
        Some(Self {
            key_id: id.trim().to_owned(),
            public: bytes.try_into().ok()?,
        })
    }
}

/// What a verified export holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub org: String,
    pub events: u64,
    /// The first event's sequence number (1 from the genesis; after the anchor checkpoint otherwise).
    pub first_seq: i64,
    pub last_seq: i64,
    pub head_hash: String,
    pub checkpoints: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum VerifyError {
    #[error("line {line} is not an export line")]
    Parse {
        line: usize,
    },
    #[error("event {at} is missing or out of order")]
    Gap {
        at: i64,
    },
    #[error("event {seq} does not follow the event before it")]
    BrokenLink {
        seq: i64,
    },
    #[error("event {seq} was changed: its hash does not match")]
    BadHash {
        seq: i64,
    },
    #[error("the export mixes organizations")]
    MixedTenants,
    #[error("line {line} is not signed by a trusted key")]
    BadSignature {
        line: usize,
    },
    #[error("the checkpoint at event {seq} is not on this chain")]
    CheckpointMismatch {
        seq: i64,
    },
    #[error("the export has no signed head")]
    NoHead,
    #[error("the signed head does not cover the export's last event (it was cut or extended)")]
    HeadMismatch,
    #[error("the export does not extend the checkpoint at event {seq} (rewritten or rolled back)")]
    DoesNotExtend {
        seq: i64,
    },
}

/// Whether `env` is signed by one of `keys` (matched by key id) over [`envelope_message`].
#[must_use]
pub fn verify_envelope(env: &Envelope, keys: &[TrustedKey]) -> bool {
    let Some(sig) = parse_sig(&env.sig) else {
        return false;
    };
    keys.iter().filter(|k| k.key_id == env.key_id).any(|k| {
        reins_proto::canon::strict_ed25519_key(&k.public)
            && UnparsedPublicKey::new(&ED25519, k.public)
                .verify(&envelope_message(env.kind, &env.org, &env.key_id, &env.body), &sig)
                .is_ok()
    })
}

fn checkpoint_of(
    env: &Envelope,
    kind: EnvelopeKind,
    keys: &[TrustedKey],
    line: usize,
) -> Result<Checkpoint, VerifyError> {
    if env.kind != kind || !verify_envelope(env, keys) {
        return Err(VerifyError::BadSignature {
            line,
        });
    }
    let cp: Checkpoint = env.parse().map_err(|_| VerifyError::Parse {
        line,
    })?;
    if cp.org != env.org {
        return Err(VerifyError::MixedTenants);
    }
    Ok(cp)
}

/// Verifies an export read from `input` (see the crate docs). `keys`: the trusted decision-service keys. `anchor`: a
/// checkpoint obtained earlier, which the export must still contain.
pub fn verify(input: impl BufRead, keys: &[TrustedKey], anchor: Option<&Checkpoint>) -> Result<Report, VerifyError> {
    verify_collect(input, keys, anchor).map(|(report, _)| report)
}

/// [`verify`], also returning the export's events in order (what [`evidence::build`] reads). The events are only
/// returned when the whole export verified.
pub fn verify_collect(
    input: impl BufRead,
    keys: &[TrustedKey],
    anchor: Option<&Checkpoint>,
) -> Result<(Report, Vec<AuditEvent>), VerifyError> {
    let mut org: Option<String> = None;
    let mut same_org = |o: &str| -> Result<(), VerifyError> {
        match &org {
            Some(x) if x != o => Err(VerifyError::MixedTenants),
            Some(_) => Ok(()),
            None => {
                org = Some(o.to_owned());
                Ok(())
            }
        }
    };
    let mut next_seq: i64 = 1;
    let mut prev = GENESIS_HASH.to_owned();
    let mut first_seq: Option<i64> = None;
    let mut hashes: BTreeMap<i64, String> = BTreeMap::new();
    let mut events = 0_u64;
    let mut checkpoints = 0_u64;
    let mut head: Option<Checkpoint> = None;
    let mut collected: Vec<AuditEvent> = Vec::new();
    for (i, line) in input.lines().enumerate() {
        let n = i + 1;
        let line = line.map_err(|_| VerifyError::Parse {
            line: n,
        })?;
        if line.trim().is_empty() {
            continue;
        }
        if head.is_some() {
            // Nothing may follow the head.
            return Err(VerifyError::HeadMismatch);
        }
        let parsed: ExportLine = serde_json::from_str(&line).map_err(|_| VerifyError::Parse {
            line: n,
        })?;
        match parsed {
            ExportLine::Event(e) => {
                same_org(&e.org)?;
                if e.seq != next_seq {
                    return Err(VerifyError::Gap {
                        at: next_seq,
                    });
                }
                if e.prev_hash != prev {
                    return Err(VerifyError::BrokenLink {
                        seq: e.seq,
                    });
                }
                if e.expected_hash() != e.hash {
                    return Err(VerifyError::BadHash {
                        seq: e.seq,
                    });
                }
                first_seq.get_or_insert(e.seq);
                hashes.insert(e.seq, e.hash.clone());
                prev.clone_from(&e.hash);
                next_seq += 1;
                events += 1;
                collected.push(e);
            }
            ExportLine::Checkpoint(env) => {
                let cp = checkpoint_of(&env, EnvelopeKind::Checkpoint, keys, n)?;
                same_org(&cp.org)?;
                checkpoints += 1;
                if events == 0 && first_seq.is_none() && hashes.is_empty() {
                    // The export starts from this checkpoint: the chain continues after it.
                    hashes.insert(cp.seq, cp.hash.clone());
                    next_seq = cp.seq + 1;
                    prev = cp.hash;
                    continue;
                }
                if hashes.get(&cp.seq) != Some(&cp.hash) {
                    return Err(VerifyError::CheckpointMismatch {
                        seq: cp.seq,
                    });
                }
            }
            ExportLine::Head(env) => {
                let cp = checkpoint_of(&env, EnvelopeKind::Head, keys, n)?;
                same_org(&cp.org)?;
                head = Some(cp);
            }
        }
    }
    let head = head.ok_or(VerifyError::NoHead)?;
    let last_seq = next_seq - 1;
    if head.seq != last_seq || head.hash != prev {
        return Err(VerifyError::HeadMismatch);
    }
    let org = org.unwrap_or_default();
    if let Some(a) = anchor
        && (a.org != org || hashes.get(&a.seq) != Some(&a.hash))
    {
        return Err(VerifyError::DoesNotExtend {
            seq: a.seq,
        });
    }
    let report = Report {
        org,
        events,
        first_seq: first_seq.unwrap_or(next_seq),
        last_seq,
        head_hash: prev,
        checkpoints,
    };
    Ok((report, collected))
}

#[cfg(test)]
pub(crate) mod tests {
    use reins_proto::canon::encode_sig;
    use ring::rand::SystemRandom;
    use ring::signature::{Ed25519KeyPair, KeyPair};

    use super::*;

    pub(crate) struct Signer {
        key: Ed25519KeyPair,
        id: String,
    }

    impl Signer {
        pub(crate) fn new(id: &str) -> Self {
            let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
            Self {
                key: Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap(),
                id: id.to_owned(),
            }
        }

        pub(crate) fn trusted(&self) -> TrustedKey {
            TrustedKey {
                key_id: self.id.clone(),
                public: self.key.public_key().as_ref().try_into().unwrap(),
            }
        }

        pub(crate) fn envelope(&self, kind: EnvelopeKind, cp: &Checkpoint) -> Envelope {
            let body = serde_json::to_string(cp).unwrap();
            let sig = self.key.sign(&envelope_message(kind, &cp.org, &self.id, &body));
            Envelope {
                kind,
                org: cp.org.clone(),
                body,
                key_id: self.id.clone(),
                sig: encode_sig(sig.as_ref().try_into().unwrap()),
            }
        }
    }

    /// A chain of `n` events of org `o1`, from `prev` at `start` (exclusive).
    pub(crate) fn chain(n: i64) -> Vec<AuditEvent> {
        let mut out: Vec<AuditEvent> = Vec::new();
        let mut prev = GENESIS_HASH.to_owned();
        for seq in 1..=n {
            let mut e = AuditEvent {
                org: "o1".into(),
                seq,
                at: 100 + seq,
                kind: "decision.auto".into(),
                actor: Some("u1".into()),
                subject: None,
                detail: format!("{{\"n\":{seq}}}"),
                prev_hash: prev.clone(),
                hash: String::new(),
            };
            e.hash = e.expected_hash();
            prev.clone_from(&e.hash);
            out.push(e);
        }
        out
    }

    pub(crate) fn rehash(events: &mut [AuditEvent]) {
        let mut prev = GENESIS_HASH.to_owned();
        for e in events {
            e.prev_hash.clone_from(&prev);
            e.hash = e.expected_hash();
            prev.clone_from(&e.hash);
        }
    }

    pub(crate) fn cp(e: &AuditEvent) -> Checkpoint {
        Checkpoint {
            org: e.org.clone(),
            seq: e.seq,
            hash: e.hash.clone(),
            at: e.at,
        }
    }

    /// The export of `events` with a checkpoint after event 2 and a head.
    pub(crate) fn export(events: &[AuditEvent], s: &Signer) -> String {
        let mut lines: Vec<String> = Vec::new();
        for e in events {
            lines.push(serde_json::to_string(&ExportLine::Event(e.clone())).unwrap());
            if e.seq == 2 {
                let env = s.envelope(EnvelopeKind::Checkpoint, &cp(e));
                lines.push(serde_json::to_string(&ExportLine::Checkpoint(env)).unwrap());
            }
        }
        let last = events.last().unwrap();
        lines.push(serde_json::to_string(&ExportLine::Head(s.envelope(EnvelopeKind::Head, &cp(last)))).unwrap());
        lines.join("\n")
    }

    fn run(text: &str, s: &Signer, anchor: Option<&Checkpoint>) -> Result<Report, VerifyError> {
        verify(text.as_bytes(), &[s.trusted()], anchor)
    }

    #[test]
    fn an_intact_export_verifies() {
        let s = Signer::new("dk1");
        let events = chain(5);
        let r = run(&export(&events, &s), &s, Some(&cp(&events[2]))).unwrap();
        assert_eq!((r.events, r.first_seq, r.last_seq, r.checkpoints), (5, 1, 5, 1));
        assert_eq!(r.head_hash, events[4].hash);
        assert_eq!(r.org, "o1");
    }

    #[test]
    fn an_edited_detail_fails() {
        let s = Signer::new("dk1");
        let mut events = chain(4);
        events[1].detail = "{\"n\":99}".into();
        assert_eq!(
            run(&export(&events, &s), &s, None),
            Err(VerifyError::BadHash {
                seq: 2
            })
        );
    }

    #[test]
    fn a_swap_fails() {
        let s = Signer::new("dk1");
        let mut events = chain(4);
        events.swap(1, 2);
        assert!(matches!(run(&export(&events, &s), &s, None), Err(VerifyError::Gap { .. })));
        // Renumbered to hide the swap, the links break.
        let mut events = chain(4);
        events.swap(1, 2);
        events[1].seq = 2;
        events[2].seq = 3;
        assert!(matches!(
            run(&export(&events, &s), &s, None),
            Err(VerifyError::BrokenLink { .. } | VerifyError::BadHash { .. })
        ));
    }

    #[test]
    fn a_gap_fails() {
        let s = Signer::new("dk1");
        let mut events = chain(4);
        events.remove(2);
        assert_eq!(
            run(&export(&events, &s), &s, None),
            Err(VerifyError::Gap {
                at: 3
            })
        );
    }

    #[test]
    fn a_dropped_tail_fails() {
        let s = Signer::new("dk1");
        let events = chain(4);
        let text = export(&events, &s);
        // Remove the last event line; the head stays.
        let lines: Vec<&str> = text.lines().collect();
        let cut: Vec<&str> = lines.iter().enumerate().filter(|(i, _)| *i != lines.len() - 2).map(|(_, l)| *l).collect();
        assert_eq!(run(&cut.join("\n"), &s, None), Err(VerifyError::HeadMismatch));
        // Without the head at all.
        let headless: Vec<&str> = lines[..lines.len() - 1].to_vec();
        assert_eq!(run(&headless.join("\n"), &s, None), Err(VerifyError::NoHead));
        // A line after the head.
        let after = format!("{text}\n{}", lines[0]);
        assert_eq!(run(&after, &s, None), Err(VerifyError::HeadMismatch));
    }

    #[test]
    fn an_unsigned_or_foreign_key_head_fails() {
        let s = Signer::new("dk1");
        let events = chain(3);
        let mallory = Signer::new("dk1");
        assert!(matches!(run(&export(&events, &mallory), &s, None), Err(VerifyError::BadSignature { .. })));
        let other_id = Signer::new("dk2");
        assert!(matches!(run(&export(&events, &other_id), &s, None), Err(VerifyError::BadSignature { .. })));
    }

    #[test]
    fn a_rewritten_chain_fails_against_an_earlier_checkpoint() {
        let s = Signer::new("dk1");
        let original = chain(4);
        let anchor = cp(&original[2]);
        let mut rewritten = original;
        rewritten[1].detail = "{\"n\":\"rewritten\"}".into();
        rehash(&mut rewritten);
        let text = export(&rewritten, &s);
        // Alone, a rewrite re-signed with the right key verifies…
        assert!(run(&text, &s, None).is_ok());
        // …but not against a checkpoint kept from before.
        assert_eq!(
            run(&text, &s, Some(&anchor)),
            Err(VerifyError::DoesNotExtend {
                seq: 3
            })
        );
        // A rollback to before the anchor fails the same way.
        let short = export(&chain(2), &s);
        assert!(matches!(run(&short, &s, Some(&anchor)), Err(VerifyError::DoesNotExtend { .. })));
    }

    #[test]
    fn a_checkpoint_off_the_chain_fails() {
        let s = Signer::new("dk1");
        let events = chain(3);
        let mut lines: Vec<String> =
            events.iter().map(|e| serde_json::to_string(&ExportLine::Event(e.clone())).unwrap()).collect();
        let mut wrong = cp(&events[1]);
        wrong.hash = events[0].hash.clone();
        lines.push(
            serde_json::to_string(&ExportLine::Checkpoint(s.envelope(EnvelopeKind::Checkpoint, &wrong))).unwrap(),
        );
        lines.push(serde_json::to_string(&ExportLine::Head(s.envelope(EnvelopeKind::Head, &cp(&events[2])))).unwrap());
        assert_eq!(
            run(&lines.join("\n"), &s, None),
            Err(VerifyError::CheckpointMismatch {
                seq: 2
            })
        );
    }

    #[test]
    fn an_export_from_a_checkpoint_verifies() {
        let s = Signer::new("dk1");
        let events = chain(5);
        let mut lines = vec![
            serde_json::to_string(&ExportLine::Checkpoint(s.envelope(EnvelopeKind::Checkpoint, &cp(&events[1]))))
                .unwrap(),
        ];
        for e in &events[2..] {
            lines.push(serde_json::to_string(&ExportLine::Event(e.clone())).unwrap());
        }
        lines.push(serde_json::to_string(&ExportLine::Head(s.envelope(EnvelopeKind::Head, &cp(&events[4])))).unwrap());
        let r = run(&lines.join("\n"), &s, Some(&cp(&events[1]))).unwrap();
        assert_eq!((r.first_seq, r.last_seq, r.events), (3, 5, 3));
    }

    #[test]
    fn mixed_tenants_fail() {
        let s = Signer::new("dk1");
        let mut events = chain(3);
        events[2].org = "o2".into();
        assert_eq!(run(&export(&events, &s), &s, None), Err(VerifyError::MixedTenants));
    }

    #[test]
    fn the_o1_admin_events_hash_the_same() {
        let e = &chain(1)[0];
        let o1 = reins_proto::org::event_hash(
            &reins_proto::org::EventFields {
                org: "o1",
                seq: 1,
                at: 101,
                kind: "decision.auto",
                actor: Some("u1"),
                subject: None,
                detail: "{\"n\":1}",
            },
            GENESIS_HASH,
        );
        assert_eq!(e.hash, o1);
    }

    #[test]
    fn keys_parse_from_the_console_spelling() {
        let s = Signer::new("dk1");
        let b64 = data_encoding::BASE64URL_NOPAD.encode(&s.trusted().public);
        assert_eq!(TrustedKey::parse(&format!("dk1={b64}")), Some(s.trusted()));
        let std = data_encoding::BASE64.encode(&s.trusted().public);
        assert_eq!(TrustedKey::parse(&format!("dk1={std}")), Some(s.trusted()));
        assert_eq!(TrustedKey::parse("nokey"), None);
    }
}
