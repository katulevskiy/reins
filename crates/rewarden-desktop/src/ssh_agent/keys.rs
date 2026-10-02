//! SSH public keys: OpenSSH key lines, `SHA256:` fingerprints, signature checks (Ed25519, ECDSA P-256/P-384, RSA with
//! SHA-1/256/512), and the host names `~/.ssh/known_hosts` gives a host key (unhashed entries only).

use data_encoding::{BASE64, BASE64_NOPAD};
use ring::signature::{self, UnparsedPublicKey};
use sha2::{Digest as _, Sha256};

use super::wire::Reader;

/// The key type a public key blob starts with (`ssh-ed25519`, `ssh-rsa`, `ecdsa-sha2-nistp256`...).
#[must_use]
pub fn key_type(blob: &[u8]) -> Option<&str> {
    Reader::new(blob).str()
}

/// `SHA256:<base64>`, as `ssh-keygen -l` and `ssh-add -l` show it.
#[must_use]
pub fn fingerprint(blob: &[u8]) -> String {
    format!("SHA256:{}", BASE64_NOPAD.encode(&Sha256::digest(blob)))
}

/// A public key line (`ssh-ed25519 AAAA… comment`): the blob and the comment.
pub fn parse_line(line: &str) -> Result<(Vec<u8>, String), String> {
    let mut parts = line.split_whitespace();
    let (Some(kind), Some(b64)) = (parts.next(), parts.next()) else {
        return Err("not an SSH public key line".to_owned());
    };
    let blob = BASE64.decode(b64.as_bytes()).map_err(|_| "the key is not base64".to_owned())?;
    if key_type(&blob) != Some(kind) {
        return Err("the key's type does not match its line".to_owned());
    }
    Ok((blob, parts.collect::<Vec<_>>().join(" ")))
}

/// The signature algorithm the agent protocol's `flags` ask for with a key of `kind`.
#[must_use]
pub fn signature_algorithm(kind: &str, flags: u32) -> &str {
    match kind {
        "ssh-rsa" if flags & 4 != 0 => "rsa-sha2-512",
        "ssh-rsa" if flags & 2 != 0 => "rsa-sha2-256",
        other => other,
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Verified {
    Good,
    Bad,
    /// A key or signature type this app cannot check.
    Unsupported,
}

/// The signature blob's algorithm name.
#[must_use]
pub fn signature_type(signature: &[u8]) -> Option<&str> {
    Reader::new(signature).str()
}

/// Left-pads (or strips leading zeros of) a big-endian integer to `len` bytes.
fn fixed(n: &[u8], len: usize) -> Option<Vec<u8>> {
    let start = n.iter().position(|&b| b != 0).unwrap_or(n.len());
    let n = &n[start..];
    if n.len() > len {
        return None;
    }
    let mut out = vec![0; len - n.len()];
    out.extend_from_slice(n);
    Some(out)
}

/// Whether `signature` (an SSH signature blob: `string alg, string sig`) is `key`'s signature over `data`.
#[must_use]
pub fn verify(key: &[u8], data: &[u8], signature: &[u8]) -> Verified {
    verify_inner(key, data, signature).unwrap_or(Verified::Bad)
}

fn verify_inner(key: &[u8], data: &[u8], signature: &[u8]) -> Option<Verified> {
    let mut sig = Reader::new(signature);
    let alg = sig.str()?;
    let raw = sig.string()?;
    if !sig.is_empty() {
        return Some(Verified::Bad);
    }
    let mut k = Reader::new(key);
    let kind = k.str()?;
    let good = |ok: bool| {
        Some(if ok {
            Verified::Good
        } else {
            Verified::Bad
        })
    };
    match kind {
        "ssh-ed25519" => {
            let pk = k.string()?;
            if alg != kind || pk.len() != 32 {
                return Some(Verified::Bad);
            }
            good(UnparsedPublicKey::new(&signature::ED25519, pk).verify(data, raw).is_ok())
        }
        "ecdsa-sha2-nistp256" | "ecdsa-sha2-nistp384" => {
            let (_curve, point) = (k.string()?, k.string()?);
            if alg != kind {
                return Some(Verified::Bad);
            }
            let (width, scheme) = if kind.ends_with("256") {
                (32, &signature::ECDSA_P256_SHA256_FIXED)
            } else {
                (48, &signature::ECDSA_P384_SHA384_FIXED)
            };
            let mut rs = Reader::new(raw);
            let mut fixed_sig = fixed(rs.mpint()?, width)?;
            fixed_sig.extend(fixed(rs.mpint()?, width)?);
            good(UnparsedPublicKey::new(scheme, point).verify(data, &fixed_sig).is_ok())
        }
        "ssh-rsa" => {
            let (e, n) = (k.mpint()?, k.mpint()?);
            let scheme: &dyn signature::VerificationAlgorithm = match alg {
                "rsa-sha2-256" => &signature::RSA_PKCS1_2048_8192_SHA256,
                "rsa-sha2-512" => &signature::RSA_PKCS1_2048_8192_SHA512,
                "ssh-rsa" => &signature::RSA_PKCS1_2048_8192_SHA1_FOR_LEGACY_USE_ONLY,
                _ => return Some(Verified::Bad),
            };
            let padded = fixed(raw, n.len())?;
            // ring wants the RSAPublicKey DER.
            let der = rsa_public_key_der(n, e);
            good(UnparsedPublicKey::new(scheme, &der).verify(data, &padded).is_ok())
        }
        _ => Some(Verified::Unsupported),
    }
}

/// `RSAPublicKey ::= SEQUENCE { modulus INTEGER, publicExponent INTEGER }`, DER.
fn rsa_public_key_der(n: &[u8], e: &[u8]) -> Vec<u8> {
    fn len(out: &mut Vec<u8>, n: usize) {
        if n < 0x80 {
            out.push(u8::try_from(n).unwrap_or(0));
        } else {
            let bytes: Vec<u8> = n.to_be_bytes().into_iter().skip_while(|&b| b == 0).collect();
            out.push(0x80 | u8::try_from(bytes.len()).unwrap_or(0));
            out.extend(bytes);
        }
    }
    fn integer(out: &mut Vec<u8>, v: &[u8]) {
        let pad = v.first().is_none_or(|b| b & 0x80 != 0);
        out.push(0x02);
        len(out, v.len() + usize::from(pad));
        if pad {
            out.push(0);
        }
        out.extend_from_slice(v);
    }
    let mut body = Vec::new();
    integer(&mut body, n);
    integer(&mut body, e);
    let mut out = vec![0x30];
    len(&mut out, body.len());
    out.extend(body);
    out
}

/// The name `known_hosts` gives `host_key`: the first plain host pattern of the first unhashed line with that key
/// (`[host]:port` becomes `host:port`, and `host` alone for port 22). Hashed, revoked and CA lines are skipped.
#[must_use]
pub fn host_name(known_hosts: &str, host_key: &[u8]) -> Option<String> {
    known_hosts.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hosts = parts.next()?;
        // Comments; `@revoked` and `@cert-authority` lines, which never name a host's own key; hashed names.
        if hosts.starts_with(['#', '@', '|']) {
            return None;
        }
        let (_kind, b64) = (parts.next()?, parts.next()?);
        if BASE64.decode(b64.as_bytes()).ok()? != host_key {
            return None;
        }
        hosts.split(',').find_map(|h| {
            if h.is_empty() || h.starts_with('!') || h.contains(['*', '?']) {
                return None;
            }
            match h.strip_prefix('[').and_then(|r| r.split_once("]:")) {
                Some((host, "22")) => Some(host.to_owned()),
                Some((host, port)) => Some(format!("{host}:{port}")),
                None => Some(h.to_owned()),
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use ring::rand::SystemRandom;
    use ring::signature::{Ed25519KeyPair, KeyPair as _};

    use super::super::wire::Writer;
    use super::*;

    fn ed25519() -> (Ed25519KeyPair, Vec<u8>) {
        let pair = Ed25519KeyPair::from_seed_unchecked(&[7; 32]).unwrap();
        let mut w = Writer::new();
        w.string(b"ssh-ed25519").string(pair.public_key().as_ref());
        (pair, w.buf)
    }

    fn sig_blob(alg: &str, raw: &[u8]) -> Vec<u8> {
        let mut w = Writer::new();
        w.string(alg.as_bytes()).string(raw);
        w.buf
    }

    #[test]
    fn key_lines_parse_and_fingerprint_like_openssh() {
        let (_, blob) = ed25519();
        let line = format!("ssh-ed25519 {} Deploy key", BASE64.encode(&blob));
        let (parsed, comment) = parse_line(&line).unwrap();
        assert_eq!(parsed, blob);
        assert_eq!(comment, "Deploy key");
        assert!(fingerprint(&blob).starts_with("SHA256:"));
        assert_eq!(fingerprint(&blob).len(), 7 + 43);
        assert!(parse_line(&format!("ssh-rsa {}", BASE64.encode(&blob))).is_err());
        assert!(parse_line("ssh-ed25519").is_err());
    }

    #[test]
    fn ed25519_signatures_are_checked() {
        let (pair, blob) = ed25519();
        let sig = sig_blob("ssh-ed25519", pair.sign(b"data").as_ref());
        assert_eq!(verify(&blob, b"data", &sig), Verified::Good);
        assert_eq!(verify(&blob, b"other", &sig), Verified::Bad);
        assert_eq!(verify(&blob, b"data", &sig_blob("ssh-rsa", pair.sign(b"data").as_ref())), Verified::Bad);
        let mut w = Writer::new();
        w.string(b"sk-ssh-ed25519@openssh.com").string(&[0; 32]).string(b"ssh:");
        assert_eq!(verify(&w.buf, b"data", &sig), Verified::Unsupported);
    }

    #[test]
    fn ecdsa_signatures_are_checked() {
        let rng = SystemRandom::new();
        let pkcs8 = signature::EcdsaKeyPair::generate_pkcs8(&signature::ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
        let pair =
            signature::EcdsaKeyPair::from_pkcs8(&signature::ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng)
                .unwrap();
        let mut key = Writer::new();
        key.string(b"ecdsa-sha2-nistp256").string(b"nistp256").string(pair.public_key().as_ref());
        let raw = pair.sign(&rng, b"data").unwrap();
        let (r, s) = raw.as_ref().split_at(32);
        let mut inner = Writer::new();
        inner.mpint(r).mpint(s);
        let sig = sig_blob("ecdsa-sha2-nistp256", &inner.buf);
        assert_eq!(verify(&key.buf, b"data", &sig), Verified::Good);
        assert_eq!(verify(&key.buf, b"datA", &sig), Verified::Bad);
    }

    #[test]
    fn rsa_flags_pick_the_hash() {
        assert_eq!(signature_algorithm("ssh-rsa", 0), "ssh-rsa");
        assert_eq!(signature_algorithm("ssh-rsa", 2), "rsa-sha2-256");
        assert_eq!(signature_algorithm("ssh-rsa", 4), "rsa-sha2-512");
        assert_eq!(signature_algorithm("ssh-ed25519", 4), "ssh-ed25519");
    }

    #[test]
    fn rsa_public_keys_become_der() {
        // SEQUENCE { INTEGER 0x00c1 (padded), INTEGER 3 }
        assert_eq!(rsa_public_key_der(&[0xc1], &[3]), [0x30, 7, 2, 2, 0, 0xc1, 2, 1, 3]);
        let long = rsa_public_key_der(&[0x7f; 256], &[1, 0, 1]);
        assert_eq!(&long[..4], &[0x30, 0x82, 0x01, 0x09]);
    }

    #[test]
    fn known_hosts_names_a_host_key_when_unhashed() {
        let (_, key) = ed25519();
        let b64 = BASE64.encode(&key);
        let other = BASE64.encode(b"\0\0\0\x0bssh-ed25519\0\0\0\x01x");
        let text = format!(
            "# comment\n|1|abc=|def= ssh-ed25519 {b64}\n@revoked bad.example ssh-ed25519 {b64}\nother.example ssh-ed25519 {other}\n*.example,!x,github.com,140.82.121.4 ssh-ed25519 {b64}\n"
        );
        assert_eq!(host_name(&text, &key).as_deref(), Some("github.com"));
        assert_eq!(
            host_name(&format!("[git.example]:2222 ssh-ed25519 {b64}\n"), &key).as_deref(),
            Some("git.example:2222")
        );
        assert_eq!(host_name(&format!("[git.example]:22 ssh-ed25519 {b64}\n"), &key).as_deref(), Some("git.example"));
        assert_eq!(host_name(&format!("|1|abc=|def= ssh-ed25519 {b64}\n"), &key), None, "hashed");
    }
}
