//! RFC 3161 timestamps of audit checkpoints (`docs/design/decisions-v2/06-trust-and-compliance.md` §5.2: "a signed
//! checkpoint … is published to an external witness, e.g. … an RFC 3161 timestamp").
//!
//! Every stored checkpoint (its signed envelope, byte for byte as stored and as the console serves it) is sent to the
//! configured time-stamping authority (`REINS_ORG_AUDIT_TSA_URL`) by the sweeper, outside any transaction; the TSA's
//! whole response is kept. An auditor checks it with `openssl ts -verify -data <seq>.json -in <seq>.tsr -CAfile
//! <tsa chain>`: the checkpoint existed, unchanged, at the TSA's time, so a chain rewritten later can't agree with it.
//!
//! The server only checks what it asked for came back (status granted, this imprint, this nonce); the CMS signature
//! is the auditor's to verify. Timestamps aren't audit events: an event would need a checkpoint, which would need a
//! timestamp, every ten minutes for ever.

use std::time::Duration;

use ring::rand::{SecureRandom, SystemRandom};

use crate::{
    CONFIG,
    api::reins::now_unix,
    db::{
        DbPool,
        models::{
            reins_org_compliance::{self, ReinsOrgWitness},
            reins_org_exec::ReinsOrgCheckpoint,
        },
    },
};

/// Checkpoints timestamped per sweeper pass, oldest first.
const PER_PASS: i64 = 20;
/// The largest TSA response kept.
const MAX_RESPONSE: usize = 64 * 1024;

/// `AlgorithmIdentifier { id-sha256, NULL }`.
const SHA256_ALG: [u8; 15] = [0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01, 0x05, 0x00];
/// `AlgorithmIdentifier { id-sha256 }` (parameters absent, RFC 5754), as some authorities echo it.
const SHA256_ALG_BARE: [u8; 13] = [0x30, 0x0b, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01];

/// What the authority is asked to timestamp: SHA-256 of the stored checkpoint envelope text.
#[must_use]
pub fn imprint(envelope_json: &str) -> [u8; 32] {
    reins_proto::canon::sha256(envelope_json.as_bytes())
}

fn der_len(n: usize, out: &mut Vec<u8>) {
    if n < 0x80 {
        out.push(u8::try_from(n).unwrap_or(0));
    } else {
        let bytes: Vec<u8> = n.to_be_bytes().into_iter().skip_while(|b| *b == 0).collect();
        out.push(0x80 | u8::try_from(bytes.len()).unwrap_or(0));
        out.extend_from_slice(&bytes);
    }
}

fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    der_len(content.len(), &mut out);
    out.extend_from_slice(content);
    out
}

/// A positive INTEGER (minimal, with a leading zero when the high bit is set).
fn der_uint(v: u64) -> Vec<u8> {
    let mut bytes: Vec<u8> = v.to_be_bytes().into_iter().skip_while(|b| *b == 0).collect();
    if bytes.is_empty() || bytes[0] & 0x80 != 0 {
        bytes.insert(0, 0);
    }
    tlv(0x02, &bytes)
}

fn message_imprint(alg: &[u8], imprint: &[u8; 32]) -> Vec<u8> {
    let mut content = alg.to_vec();
    content.extend(tlv(0x04, imprint));
    tlv(0x30, &content)
}

/// A `TimeStampReq` (RFC 3161 §2.4.1): version 1, the SHA-256 imprint, `nonce`, and the TSA's certificate asked for.
#[must_use]
pub fn request_der(imprint: &[u8; 32], nonce: u64) -> Vec<u8> {
    let mut content = der_uint(1);
    content.extend(message_imprint(&SHA256_ALG, imprint));
    content.extend(der_uint(nonce));
    content.extend([0x01, 0x01, 0xff]);
    tlv(0x30, &content)
}

/// Reads one DER TLV at `at`: (tag, content range, end).
fn read_tlv(der: &[u8], at: usize) -> Option<(u8, std::ops::Range<usize>, usize)> {
    let tag = *der.get(at)?;
    let first = *der.get(at + 1)?;
    let (len, start) = if first < 0x80 {
        (usize::from(first), at + 2)
    } else {
        let n = usize::from(first & 0x7f);
        if n == 0 || n > 4 {
            return None;
        }
        let mut len = 0usize;
        for i in 0..n {
            len = (len << 8) | usize::from(*der.get(at + 2 + i)?);
        }
        (len, at + 2 + n)
    };
    let end = start.checked_add(len)?;
    (end <= der.len()).then_some((tag, start..end, end))
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Checks a `TimeStampResp` answers this request: status granted (0) or granted with modifications (1), a token
/// present, holding this imprint and this nonce. Returns the token's `genTime` (`YYYYMMDDhhmmss[.f]Z`).
pub fn check_response(der: &[u8], imprint: &[u8; 32], nonce: u64) -> Result<String, String> {
    let (tag, outer, _) = read_tlv(der, 0).ok_or("not a timestamp response")?;
    if tag != 0x30 {
        return Err("not a timestamp response".to_owned());
    }
    let (tag, status_info, token_at) = read_tlv(der, outer.start).ok_or("no status")?;
    let (itag, status, _) = read_tlv(der, status_info.start).ok_or("no status")?;
    if tag != 0x30 || itag != 0x02 || status.len() != 1 {
        return Err("no status".to_owned());
    }
    match der[status.start] {
        0 | 1 => {}
        s => return Err(format!("the authority refused (status {s})")),
    }
    if token_at >= outer.end {
        return Err("no timestamp token".to_owned());
    }
    let token = &der[token_at..outer.end];
    let (at, len) = [message_imprint(&SHA256_ALG, imprint), message_imprint(&SHA256_ALG_BARE, imprint)]
        .iter()
        .find_map(|m| find(token, m).map(|i| (i, m.len())))
        .ok_or("the token is for something else")?;
    // TSTInfo: … messageImprint, serialNumber, genTime, … nonce …
    let after = at + len;
    let (stag, _, serial_end) = read_tlv(token, after).ok_or("no serial number")?;
    let (gtag, gen_time, _) = read_tlv(token, serial_end).ok_or("no time")?;
    if stag != 0x02 || gtag != 0x18 {
        return Err("not a TSTInfo".to_owned());
    }
    if find(&token[after..], &der_uint(nonce)).is_none() {
        return Err("the token is for another request".to_owned());
    }
    String::from_utf8(token[gen_time].to_vec()).map_err(|_| "not a time".to_owned())
}

fn client() -> Result<reqwest::Client, String> {
    rustls::crypto::ring::default_provider().install_default().ok();
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())
}

/// Asks the authority at `tsa` to timestamp `cp`; returns the response (DER) once it checked out.
async fn stamp(tsa: &str, cp: &ReinsOrgCheckpoint) -> Result<Vec<u8>, String> {
    let mut n = [0u8; 8];
    SystemRandom::new().fill(&mut n).map_err(|_| "no randomness")?;
    let nonce = u64::from_be_bytes(n) >> 1;
    let digest = imprint(&cp.envelope);
    let response = client()?
        .post(tsa)
        .header(reqwest::header::CONTENT_TYPE, "application/timestamp-query")
        .header(reqwest::header::ACCEPT, "application/timestamp-reply")
        .body(request_der(&digest, nonce))
        .send()
        .await
        .map_err(|e| format!("the authority can't be reached: {}", e.without_url()))?;
    if !response.status().is_success() {
        return Err(format!("the authority answered {}", response.status().as_u16()));
    }
    let (body, more) =
        crate::api::reins::outbound::read_limited(response, MAX_RESPONSE).await.map_err(|e| e.to_string())?;
    if more {
        return Err("the answer is too large".to_owned());
    }
    check_response(&body, &digest, nonce)?;
    Ok(body)
}

/// Timestamps the oldest checkpoints that have none yet (when an authority is configured). Failures are logged and
/// tried again on the next pass.
pub async fn stamp_due(pool: &DbPool) {
    let tsa = CONFIG.reins_org_audit_tsa_url();
    if tsa.trim().is_empty() {
        return;
    }
    let Ok(conn) = pool.get().await else {
        return;
    };
    let due = match conn.run(|c| reins_org_compliance::q_unwitnessed(c, PER_PASS)).await {
        Ok(d) => d,
        Err(e) => {
            warn!("Audit timestamps: {e:?}");
            return;
        }
    };
    for cp in due {
        match stamp(tsa.trim(), &cp).await {
            Ok(der) => {
                let row = ReinsOrgWitness {
                    org_uuid: cp.org_uuid.clone(),
                    seq: cp.seq,
                    tsa: tsa.trim().to_owned(),
                    response: data_encoding::BASE64.encode(&der),
                    at: now_unix(),
                };
                if let Err(e) = conn.run(move |c| reins_org_compliance::q_insert_witness(c, &row)).await {
                    warn!("Audit timestamp of checkpoint {} of {}: {e:?}", cp.seq, cp.org_uuid);
                }
            }
            Err(e) => {
                warn!("Audit timestamp of checkpoint {} of {}: {e}", cp.seq, cp.org_uuid);
                // The authority is down or refuses: the rest waits for the next pass.
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A response as an authority builds it: status, then a token holding a TSTInfo for `imprint` and `nonce`.
    pub(crate) fn response(status: u8, imprint: &[u8; 32], nonce: u64) -> Vec<u8> {
        let mut tst = der_uint(1);
        tst.extend(tlv(0x06, &[0x2a, 0x03, 0x04]));
        tst.extend(message_imprint(&SHA256_ALG, imprint));
        tst.extend(der_uint(4242));
        tst.extend(tlv(0x18, b"20261027120000Z"));
        tst.extend(der_uint(nonce));
        let token = tlv(0x30, &[tlv(0x06, &[0x2a, 0x86, 0x48]), tlv(0xa0, &tlv(0x04, &tlv(0x30, &tst)))].concat());
        let mut content = tlv(0x30, &der_uint(u64::from(status)));
        if status <= 1 {
            content.extend(token);
        }
        tlv(0x30, &content)
    }

    #[test]
    fn request_der_is_the_rfc_3161_shape() {
        let der = request_der(&[0u8; 32], 1);
        let mut expected = vec![0x30, 0x3c, 0x02, 0x01, 0x01, 0x30, 0x31];
        expected.extend(SHA256_ALG);
        expected.extend([0x04, 0x20]);
        expected.extend([0u8; 32]);
        expected.extend([0x02, 0x01, 0x01, 0x01, 0x01, 0xff]);
        assert_eq!(der, expected);
        // A nonce with the high bit set gets a leading zero (a positive INTEGER).
        assert!(request_der(&[0u8; 32], 0x80).windows(4).any(|w| w == [0x02, 0x02, 0x00, 0x80]));
    }

    #[test]
    fn a_granted_response_for_this_imprint_passes() {
        let im = imprint("{\"kind\":\"checkpoint\"}");
        assert_eq!(check_response(&response(0, &im, 77), &im, 77).unwrap(), "20261027120000Z");
        assert!(check_response(&response(1, &im, 77), &im, 77).is_ok());
    }

    #[test]
    fn a_rejection_or_another_imprint_fails() {
        let im = imprint("a");
        assert!(check_response(&response(2, &im, 77), &im, 77).unwrap_err().contains("refused"));
        assert!(check_response(&response(0, &imprint("b"), 77), &im, 77).is_err());
        assert!(check_response(&response(0, &im, 78), &im, 77).is_err());
        assert!(check_response(b"garbage", &im, 77).is_err());
        assert!(check_response(&[], &im, 77).is_err());
    }

    struct Tsa;

    impl wiremock::Respond for Tsa {
        /// Echoes the request's imprint and nonce, as an authority does.
        fn respond(&self, req: &wiremock::Request) -> wiremock::ResponseTemplate {
            let body = &req.body;
            let at = find(body, &SHA256_ALG).unwrap() + SHA256_ALG.len() + 2;
            let im: [u8; 32] = body[at..at + 32].try_into().unwrap();
            let (_, n, _) = read_tlv(body, at + 32).unwrap();
            let nonce = body[n].iter().fold(0u64, |acc, b| (acc << 8) | u64::from(*b));
            wiremock::ResponseTemplate::new(200).set_body_bytes(response(0, &im, nonce))
        }
    }

    #[tokio::test]
    async fn stamp_asks_the_tsa_and_checks_the_answer() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{header, method},
        };
        let tsa = MockServer::start().await;
        Mock::given(method("POST"))
            .and(header("content-type", "application/timestamp-query"))
            .respond_with(Tsa)
            .mount(&tsa)
            .await;
        let cp = ReinsOrgCheckpoint {
            org_uuid: "o1".into(),
            seq: 3,
            hash: "h".into(),
            at: 1,
            envelope: "{\"kind\":\"checkpoint\"}".into(),
        };
        let der = stamp(&tsa.uri(), &cp).await.unwrap();
        assert!(find(&der, &imprint(&cp.envelope)).is_some());
        // An authority that answers for something else is refused.
        let liar = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(response(0, &imprint("other"), 1)))
            .mount(&liar)
            .await;
        assert!(stamp(&liar.uri(), &cp).await.is_err());
    }

    #[cfg(sqlite)]
    #[test]
    fn unwitnessed_lists_checkpoints_without_a_timestamp() {
        use crate::api::reins::org::{audit, kms::tests::use_test_kms};
        use_test_kms();
        let mut c = super::super::testing::db();
        audit::record(&mut c, "o1", "x.y", None, None, &serde_json::json!({}), 1).unwrap();
        let cp = audit::checkpoint(&mut c, "o1", 2).unwrap().unwrap();
        let due = reins_org_compliance::q_unwitnessed(&mut c, 10).unwrap();
        assert_eq!(due.iter().map(|d| d.seq).collect::<Vec<_>>(), [cp.seq]);
        reins_org_compliance::q_insert_witness(
            &mut c,
            &ReinsOrgWitness {
                org_uuid: "o1".into(),
                seq: cp.seq,
                tsa: "t".into(),
                response: "r".into(),
                at: 3,
            },
        )
        .unwrap();
        assert!(reins_org_compliance::q_unwitnessed(&mut c, 10).unwrap().is_empty());
    }
}
