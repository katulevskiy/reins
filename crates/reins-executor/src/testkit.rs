//! Test fixtures: an org with an issuer and a decision-service key, a roster of members with P-256 device keys, a
//! policy, and jobs countersigned as the server would sign them.

use reins_audit::TrustedKey;
use reins_policy::context::{FactKind, Zone};
use reins_policy::matrix::{Cell, Issuer, Rule, Then, When};
use reins_policy::org::{
    ManagedOutcome, ManagedRequest, MemberPolicy, OrgPolicy, OrgRule, ReviewSpec, Reviewers, evaluate_managed,
};
use reins_proto::managed::{
    DECISION_VERSION, DecisionRecord, Envelope, EnvelopeKind, Job, JobKind, LeaseAccess, LeaseBinding, LeaseJob,
    ReviewDecision, ReviewSignature, ReviewSubject, Roster, RosterDevice, RosterMember, SignedMemberPolicy, Verdict,
    canonical_json, envelope_message, lease_args, member_policy_message, request_digest, review_message,
};
use reins_proto::org::Role;
use ring::rand::SystemRandom;
use ring::signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, Ed25519KeyPair, KeyPair};
use serde_json::{Value, json};

use crate::config::Pins;

pub const ORG: &str = "o1";
pub const LEE: &str = "u-lee";
pub const KIM: &str = "u-kim";
pub const SAM: &str = "u-sam";
pub const CONNECTION: &str = "c1";
/// The server's web origin, where web approvals are asserted.
pub const WEB_ORIGIN: &str = "https://reins.example";

const P256_SPKI_PREFIX: [u8; 26] = [
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce,
    0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
];

pub fn now() -> i64 {
    crate::now()
}

pub fn ed25519(seed: u8) -> Ed25519KeyPair {
    Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).unwrap()
}

pub fn trusted(key_id: &str, pair: &Ed25519KeyPair) -> TrustedKey {
    TrustedKey {
        key_id: key_id.to_owned(),
        public: pair.public_key().as_ref().try_into().unwrap(),
    }
}

pub fn sign_env(kind: EnvelopeKind, org: &str, key_id: &str, pair: &Ed25519KeyPair, body: &Value) -> Envelope {
    let body = canonical_json(body);
    let sig = pair.sign(&envelope_message(kind, org, key_id, &body));
    Envelope {
        kind,
        org: org.to_owned(),
        body,
        key_id: key_id.to_owned(),
        sig: data_encoding::BASE64URL_NOPAD.encode(sig.as_ref()),
    }
}

/// An enrolled phone's key.
pub struct Device {
    pub id: String,
    pair: EcdsaKeyPair,
}

impl Device {
    pub fn new(id: &str) -> Self {
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
        Self {
            id: id.to_owned(),
            pair: EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng).unwrap(),
        }
    }

    pub fn spki(&self) -> String {
        let mut der = P256_SPKI_PREFIX.to_vec();
        der.extend_from_slice(self.pair.public_key().as_ref());
        data_encoding::BASE64.encode(&der)
    }

    pub fn sign(&self, msg: &[u8]) -> String {
        data_encoding::BASE64.encode(self.pair.sign(&SystemRandom::new(), msg).unwrap().as_ref())
    }

    /// A soft WebAuthn authenticator: an assertion of `challenge` at `origin`, with these flags (UP|UV = 0x05).
    pub fn assert(&self, challenge: &str, origin: &str, flags: u8) -> (String, reins_proto::managed::WebAuthnProof) {
        let b64 = data_encoding::BASE64URL_NOPAD;
        let rp = origin.split("://").nth(1).unwrap_or_default().split(':').next().unwrap_or_default();
        let mut auth = reins_proto::canon::sha256(rp.as_bytes()).to_vec();
        auth.push(flags);
        auth.extend_from_slice(&[0, 0, 0, 1]);
        let client = json!({"type": "webauthn.get", "challenge": challenge, "origin": origin}).to_string();
        let mut signed = auth.clone();
        signed.extend_from_slice(&reins_proto::canon::sha256(client.as_bytes()));
        (
            self.sign(&signed),
            reins_proto::managed::WebAuthnProof {
                authenticator_data: b64.encode(&auth),
                client_data_json: b64.encode(client.as_bytes()),
            },
        )
    }
}

/// The org: Lee (the requester, group `eng`), Kim (group `sec`, a passing device) and Sam (group `sec`, a device that
/// fails posture).
pub struct Fixture {
    pub issuer: Ed25519KeyPair,
    pub decision: Ed25519KeyPair,
    pub lee: Device,
    pub kim: Device,
    pub sam: Device,
    /// Kim's device-bound web passkey (credential `cred-kim`), and Lee's (`cred-lee`).
    pub kim_passkey: Device,
    pub lee_passkey: Device,
    /// Lee's manager in the roster.
    pub manager: Option<String>,
    pub policy: OrgPolicy,
    pub roster_at: i64,
}

impl Fixture {
    pub fn new() -> Self {
        Self {
            issuer: ed25519(1),
            decision: ed25519(2),
            lee: Device::new("d-lee"),
            kim: Device::new("d-kim"),
            sam: Device::new("d-sam"),
            kim_passkey: Device::new("cred-kim"),
            lee_passkey: Device::new("cred-lee"),
            manager: Some(KIM.to_owned()),
            policy: acme(),
            roster_at: now(),
        }
    }

    pub fn pins(&self) -> Pins {
        Pins {
            org: ORG.to_owned(),
            issuer: trusted("iss-1", &self.issuer),
            decision: trusted("dec-1", &self.decision),
            web_origin: Some(WEB_ORIGIN.to_owned()),
        }
    }

    pub fn policy_env(&self, policy: &OrgPolicy) -> Envelope {
        sign_env(EnvelopeKind::Policy, ORG, "iss-1", &self.issuer, &serde_json::to_value(policy).unwrap())
    }

    pub fn roster(&self) -> Roster {
        let member = |user: &str, groups: &[&str], roles: Vec<Role>, d: &Device, may_decide: bool| RosterMember {
            user: user.to_owned(),
            email: format!("{}@acme.com", user.trim_start_matches("u-")),
            groups: groups.iter().map(|g| (*g).to_owned()).collect(),
            roles,
            devices: vec![RosterDevice {
                device: d.id.clone(),
                key: d.spki(),
                may_decide,
            }],
            manager: None,
            passkeys: Vec::new(),
        };
        let passkey = |d: &Device| reins_proto::managed::RosterPasskey {
            credential_id: d.id.clone(),
            key: d.spki(),
        };
        let mut lee = member(LEE, &["eng"], vec![Role::Member], &self.lee, true);
        lee.manager.clone_from(&self.manager);
        lee.passkeys = vec![passkey(&self.lee_passkey)];
        let mut kim = member(KIM, &["sec"], vec![Role::SecurityAdmin], &self.kim, true);
        kim.passkeys = vec![passkey(&self.kim_passkey)];
        Roster {
            org: ORG.to_owned(),
            version: 1,
            issued_at: self.roster_at,
            members: vec![lee, kim, member(SAM, &["sec"], vec![Role::Member], &self.sam, false)],
            backups: std::collections::BTreeMap::new(),
        }
    }

    pub fn roster_env(&self) -> Envelope {
        sign_env(EnvelopeKind::Roster, ORG, "iss-1", &self.issuer, &serde_json::to_value(self.roster()).unwrap())
    }

    /// The record the decision service would make for Lee's `tool(args)`, with `approvals`.
    pub fn record(&self, tool: &str, args: &Value, member: Option<&MemberPolicy>) -> DecisionRecord {
        let at = now();
        let v = evaluate_managed(
            &ManagedRequest {
                org: ORG,
                member: LEE,
                connection: CONNECTION,
                tool,
                args,
                now: at,
                reason: None,
            },
            &self.policy,
            member,
        );
        DecisionRecord {
            v: DECISION_VERSION,
            org: ORG.to_owned(),
            request: "or_1".to_owned(),
            digest: request_digest(ORG, LEE, CONNECTION, tool, args),
            member: LEE.to_owned(),
            connection: CONNECTION.to_owned(),
            tool: tool.to_owned(),
            verdict: match v.outcome {
                ManagedOutcome::Auto => Verdict::Auto,
                ManagedOutcome::Review => Verdict::Approved,
                ManagedOutcome::Deny => Verdict::Denied,
            },
            snapshot: self.policy.hash(),
            member_policy: member.map(MemberPolicy::hash),
            obligations: v.obligation_ids(),
            approvals: Vec::new(),
            lease: None,
            resolved: std::collections::BTreeMap::new(),
            reason: None,
            evaluated_at: at,
            rollout: None,
            decided_at: at,
            valid_until: at + 3600,
        }
    }

    /// `who`'s signed approval of `record`.
    #[allow(clippy::unused_self)]
    pub fn approve(&self, record: &DecisionRecord, member: &str, device: &Device) -> ReviewSignature {
        let at = record.decided_at;
        let msg = review_message(
            ORG,
            ReviewSubject::Request,
            &record.request,
            &record.digest,
            ReviewDecision::Approve,
            &record.snapshot,
            at,
        );
        ReviewSignature {
            member: member.to_owned(),
            device: device.id.clone(),
            decision: ReviewDecision::Approve,
            at,
            sig: device.sign(&msg),
            webauthn: None,
        }
    }

    /// `member`'s signed web approval of `record` with `passkey` (flags as asserted; 0x05 is UP|UV).
    #[allow(clippy::unused_self)]
    pub fn web_approve(&self, record: &DecisionRecord, member: &str, passkey: &Device, flags: u8) -> ReviewSignature {
        let at = record.decided_at;
        let msg = review_message(
            ORG,
            ReviewSubject::Request,
            &record.request,
            &record.digest,
            ReviewDecision::Approve,
            &record.snapshot,
            at,
        );
        let (sig, proof) = passkey.assert(&reins_proto::managed::webauthn_challenge(&msg), WEB_ORIGIN, flags);
        ReviewSignature {
            member: member.to_owned(),
            device: passkey.id.clone(),
            decision: ReviewDecision::Approve,
            at,
            sig,
            webauthn: Some(proof),
        }
    }

    /// A rollout `id` from `stable` to `candidate` (pilot `pilot`, then 25%, then everyone; a minute's dwell) activated
    /// at `at`: every step signed by the issuer (seq 1..=3, the rollback 4).
    pub fn rollout_steps(&self, id: &str, stable: &str, candidate: &str, pilot: &[&str], at: i64) -> Vec<Envelope> {
        use reins_proto::rollout::{Rollout, RolloutPlan, Stage};
        let plan = RolloutPlan {
            stages: vec![
                Stage::Pilot {
                    members: pilot.iter().map(|m| (*m).to_owned()).collect(),
                },
                Stage::Percent(25),
                Stage::All,
            ],
            dwell_secs: 60,
            ..RolloutPlan::default()
        };
        Rollout::steps(ORG, id, 1, stable, candidate, &plan, at)
            .iter()
            .map(|r| {
                sign_env(EnvelopeKind::Rollout, ORG, "iss-1", &self.issuer, &serde_json::from_str(&r.body()).unwrap())
            })
            .collect()
    }

    pub fn countersign(&self, record: &DecisionRecord) -> Envelope {
        sign_env(EnvelopeKind::Decision, &record.org, "dec-1", &self.decision, &serde_json::to_value(record).unwrap())
    }

    pub fn job(&self, tool: &str, args: Value, record: &DecisionRecord) -> Job {
        Job {
            request: record.request.clone(),
            kind: JobKind::Execute,
            tool: tool.to_owned(),
            args,
            decision: self.countersign(record),
            current_policy: self.policy.hash(),
            member_policy: None,
            lease: None,
        }
    }

    /// An Auto issue on `acme/api`.
    pub fn auto_job(&self) -> Job {
        let args = issue("acme/api");
        let record = self.record("github_create_issue", &args, None);
        assert_eq!(record.verdict, Verdict::Auto);
        self.job("github_create_issue", args, &record)
    }

    /// An issue on `acme/prod-db`, which the org's `prod-review` rule sends to `sec`, approved by Kim.
    pub fn reviewed_job(&self) -> (Job, DecisionRecord) {
        let args = issue("acme/prod-db");
        let mut record = self.record("github_create_issue", &args, None);
        assert_eq!(record.verdict, Verdict::Approved);
        record.approvals = vec![self.approve(&record, KIM, &self.kim)];
        (self.job("github_create_issue", args, &record), record)
    }

    /// A read lease of `acme/api` for Lee.
    pub fn lease_job(&self, box_key: &str) -> Job {
        let lease = LeaseJob {
            lease: "ls_1".to_owned(),
            repo: "acme/api".to_owned(),
            access: LeaseAccess::Read,
            box_key: box_key.to_owned(),
            expires_at: now() + 900,
        };
        let args = lease_args(&lease.repo, lease.access, &lease.box_key);
        let mut record = self.record(reins_proto::managed::LEASE_TOOL, &args, None);
        record.lease = Some(LeaseBinding::of(&lease));
        Job {
            kind: JobKind::Lease,
            lease: Some(lease),
            ..self.job(reins_proto::managed::LEASE_TOOL, args, &record)
        }
    }

    #[allow(clippy::unused_self)]
    pub fn member_policy(&self, policy: &MemberPolicy, device: &Device) -> SignedMemberPolicy {
        let body = canonical_json(&serde_json::to_value(policy).unwrap());
        let (version, at) = (2, now());
        SignedMemberPolicy {
            sig: device.sign(&member_policy_message(ORG, LEE, version, at, &body)),
            body,
            version,
            at,
            device: device.id.clone(),
        }
    }
}

pub fn issue(repo: &str) -> Value {
    json!({"repo": repo, "title": "Flaky test", "body": "x"})
}

pub fn sec_review() -> ReviewSpec {
    ReviewSpec::by(Reviewers::Group("sec".to_owned()))
}

pub fn org_rule(id: &str, resource: &str, then: Then, review: Option<ReviewSpec>) -> OrgRule {
    OrgRule::new(
        Rule {
            id: id.to_owned(),
            issuer: Issuer::Org(ORG.to_owned()),
            when: When {
                resource: Some(resource.to_owned()),
                ..When::default()
            },
            then,
            not_after: None,
            accepted: false,
            explain: format!("{id} says so"),
        },
        review,
    )
}

/// Acme's policy: its repositories are Work (Auto up to L2), and writes on `acme/prod-*` are reviewed by `sec`.
pub fn acme() -> OrgPolicy {
    let mut p = OrgPolicy::empty(ORG);
    p.facts.set(FactKind::Owner, "acme", Zone::Work);
    p.facts.set(FactKind::Domain, "acme.com", Zone::Work);
    p.matrix.work = [Cell::Auto, Cell::Auto, Cell::Auto, Cell::Ask];
    p.rules.push(org_rule("prod-review", "acme/prod-*", Then::Review, Some(sec_review())));
    p
}
