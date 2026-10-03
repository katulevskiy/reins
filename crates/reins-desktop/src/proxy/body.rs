//! Request bodies the proxy must hold before forwarding: kept in memory up to [`MEMORY_LIMIT`], then in a temporary
//! file (0600, deleted on drop), never beyond the caller's limit. A push body is then decoded (gzip) and split into
//! its head and its pack.

use std::io::{Read, Write as _};

use bytes::Bytes;
use http_body_util::BodyExt as _;
use sha2::{Digest as _, Sha256};
use tempfile::NamedTempFile;
use tokio::io::AsyncWriteExt as _;

use crate::git::pktline::{FLUSH, PktError};
use crate::git::{ReceiveHead, parse_receive_head};

pub const MEMORY_LIMIT: usize = 8 << 20;
/// The largest push the proxy accepts (GitHub's own limit is 2 GiB).
pub const MAX_PUSH: u64 = 2 << 30;
/// The commands and options before the pack must fit in this.
const HEAD_LIMIT: u64 = 1 << 20;
const CHUNK: usize = 64 << 10;

#[derive(Debug, thiserror::Error)]
pub enum BodyError {
    #[error("the request is larger than {0} bytes")]
    TooLarge(u64),
    #[error("reading the request: {0}")]
    Read(String),
    #[error("temporary file: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Invalid(String),
}

/// A held request body.
#[derive(Debug)]
pub enum Spool {
    Memory(Bytes),
    File {
        file: NamedTempFile,
        len: u64,
    },
}

impl Spool {
    #[must_use]
    pub fn len(&self) -> u64 {
        match self {
            Self::Memory(b) => b.len() as u64,
            Self::File {
                len,
                ..
            } => *len,
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Reads it from the start (blocking).
    pub fn reader(&self) -> std::io::Result<Box<dyn Read + Send + '_>> {
        Ok(match self {
            Self::Memory(b) => Box::new(std::io::Cursor::new(b.as_ref())),
            Self::File {
                file,
                ..
            } => Box::new(std::io::BufReader::with_capacity(CHUNK, file.reopen()?)),
        })
    }

    /// A body for the upstream request; can be made again (redirects).
    pub fn body(&self) -> std::io::Result<reqwest::Body> {
        Ok(match self {
            Self::Memory(b) => reqwest::Body::from(b.clone()),
            Self::File {
                file,
                ..
            } => {
                let f = tokio::fs::File::from_std(file.reopen()?);
                let stream = futures_util::stream::try_unfold(f, async |mut f| {
                    let mut buf = vec![0u8; CHUNK];
                    let n = tokio::io::AsyncReadExt::read(&mut f, &mut buf).await?;
                    if n == 0 {
                        return Ok::<_, std::io::Error>(None);
                    }
                    buf.truncate(n);
                    Ok(Some((Bytes::from(buf), f)))
                });
                reqwest::Body::wrap_stream(stream)
            }
        })
    }
}

/// Reads a whole request body, refusing more than `limit` bytes.
pub async fn read_body<B>(mut body: B, limit: u64) -> Result<Spool, BodyError>
where
    B: hyper::body::Body<Data = Bytes> + Unpin,
    B::Error: std::fmt::Display,
{
    let mut mem: Vec<u8> = Vec::new();
    let mut file: Option<(NamedTempFile, tokio::fs::File)> = None;
    let mut len = 0u64;
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|e| BodyError::Read(e.to_string()))?;
        let Ok(data) = frame.into_data() else {
            continue;
        };
        len += data.len() as u64;
        if len > limit {
            return Err(BodyError::TooLarge(limit));
        }
        if let Some((_, f)) = &mut file {
            f.write_all(&data).await?;
        } else {
            mem.extend_from_slice(&data);
            if mem.len() > MEMORY_LIMIT {
                let tmp = NamedTempFile::new()?;
                let mut f = tokio::fs::File::from_std(tmp.as_file().try_clone()?);
                f.write_all(&mem).await?;
                mem = Vec::new();
                file = Some((tmp, f));
            }
        }
    }
    match file {
        Some((tmp, mut f)) => {
            f.flush().await?;
            Ok(Spool::File {
                file: tmp,
                len,
            })
        }
        None => Ok(Spool::Memory(Bytes::from(mem))),
    }
}

/// A blocking writer with the same memory-then-file behaviour.
struct SpoolWriter {
    mem: Vec<u8>,
    file: Option<NamedTempFile>,
    len: u64,
    limit: u64,
}

impl SpoolWriter {
    fn new(limit: u64) -> Self {
        Self {
            mem: Vec::new(),
            file: None,
            len: 0,
            limit,
        }
    }

    fn write(&mut self, data: &[u8]) -> Result<(), BodyError> {
        self.len += data.len() as u64;
        if self.len > self.limit {
            return Err(BodyError::TooLarge(self.limit));
        }
        if let Some(f) = &mut self.file {
            f.write_all(data)?;
        } else {
            self.mem.extend_from_slice(data);
            if self.mem.len() > MEMORY_LIMIT {
                let mut f = NamedTempFile::new()?;
                f.write_all(&self.mem)?;
                self.mem = Vec::new();
                self.file = Some(f);
            }
        }
        Ok(())
    }

    fn finish(self) -> Result<Spool, BodyError> {
        match self.file {
            Some(mut f) => {
                f.flush()?;
                Ok(Spool::File {
                    file: f,
                    len: self.len,
                })
            }
            None => Ok(Spool::Memory(Bytes::from(self.mem))),
        }
    }
}

fn copy_into(mut from: impl Read, to: &mut SpoolWriter) -> Result<(), BodyError> {
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = match from.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) if e.kind() == std::io::ErrorKind::InvalidInput || e.kind() == std::io::ErrorKind::InvalidData => {
                return Err(BodyError::Invalid(format!("the gzip request body is damaged: {e}")));
            }
            Err(e) => return Err(e.into()),
        };
        to.write(&buf[..n])?;
    }
}

/// A push body taken apart: what git asks for and the pack (exactly its bytes, in its own file, for the analysis).
#[derive(Debug)]
pub struct Push {
    pub head: ReceiveHead,
    pub pack: Option<NamedTempFile>,
    /// Lowercase hex SHA-256 of the pack bytes (of nothing, without a pack).
    pub pack_sha256: String,
}

/// Parses a `git-receive-pack` body (blocking). `gzip`: the body is `Content-Encoding: gzip`.
pub fn parse_push(raw: &Spool, gzip: bool) -> Result<Push, BodyError> {
    let decoded_owned;
    let decoded = if gzip {
        let mut w = SpoolWriter::new(MAX_PUSH);
        copy_into(flate2::read::GzDecoder::new(raw.reader()?), &mut w)?;
        decoded_owned = w.finish()?;
        &decoded_owned
    } else {
        raw
    };
    let mut prefix = Vec::new();
    decoded.reader()?.take(HEAD_LIMIT).read_to_end(&mut prefix)?;
    let too_many = || BodyError::Invalid("too many ref updates in one push".to_owned());
    let head = match parse_receive_head(&prefix) {
        Ok(h) => h,
        Err(PktError::Truncated) if prefix.len() as u64 == HEAD_LIMIT => return Err(too_many()),
        Err(e) => return Err(BodyError::Invalid(format!("this push could not be read: {e}"))),
    };
    let ends_with_flush = head.pack_offset >= 4 && prefix.get(head.pack_offset - 4..head.pack_offset) == Some(FLUSH);
    if (head.pack_offset as u64) < decoded.len() && !ends_with_flush {
        return Err(too_many());
    }
    let mut hash = Sha256::new();
    let pack = if !head.commands.is_empty() && (head.pack_offset as u64) < decoded.len() {
        let mut r = decoded.reader()?;
        std::io::copy(&mut (&mut r).take(head.pack_offset as u64), &mut std::io::sink())?;
        let mut f = NamedTempFile::new()?;
        let mut buf = vec![0u8; CHUNK];
        loop {
            let n = r.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hash.update(&buf[..n]);
            f.write_all(&buf[..n])?;
        }
        f.flush()?;
        Some(f)
    } else {
        None
    };
    Ok(Push {
        head,
        pack,
        pack_sha256: data_encoding::HEXLOWER.encode(&hash.finalize()),
    })
}

#[cfg(test)]
mod tests {
    use http_body_util::Full;

    use super::*;
    use crate::git::pktline::encode;

    const A: &str = "1111111111111111111111111111111111111111";
    const B: &str = "2222222222222222222222222222222222222222";

    fn push_body(pack: &[u8]) -> Vec<u8> {
        let mut body = encode(format!("{A} {B} refs/heads/main\0report-status side-band-64k\n").as_bytes());
        body.extend_from_slice(FLUSH);
        body.extend_from_slice(pack);
        body
    }

    fn gzip(data: &[u8]) -> Vec<u8> {
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }

    fn sha(data: &[u8]) -> String {
        data_encoding::HEXLOWER.encode(&Sha256::digest(data))
    }

    #[tokio::test]
    async fn small_bodies_stay_in_memory_and_large_ones_spill() {
        let small = read_body(Full::new(Bytes::from_static(b"0000")), 100).await.unwrap();
        assert!(matches!(small, Spool::Memory(_)));
        let big = vec![7u8; MEMORY_LIMIT + 10];
        let spool = read_body(Full::new(Bytes::from(big.clone())), MAX_PUSH).await.unwrap();
        assert!(matches!(spool, Spool::File { .. }));
        assert_eq!(spool.len(), big.len() as u64);
        let mut back = Vec::new();
        spool.reader().unwrap().read_to_end(&mut back).unwrap();
        assert_eq!(back, big);
        let streamed = http_body_util::BodyExt::collect(spool.body().unwrap()).await.unwrap().to_bytes();
        assert_eq!(streamed.len(), big.len());
        assert!(matches!(read_body(Full::new(Bytes::from(vec![0u8; 101])), 100).await, Err(BodyError::TooLarge(100))));
    }

    #[test]
    fn a_push_gives_its_head_and_its_pack_plain_or_gzipped() {
        let pack = b"PACK\0\0\0\x02\0\0\0\0rest".to_vec();
        let body = push_body(&pack);
        let plain = parse_push(&Spool::Memory(Bytes::from(body.clone())), false).unwrap();
        assert_eq!(plain.head.commands[0].name, "refs/heads/main");
        assert_eq!(std::fs::read(plain.pack.as_ref().unwrap().path()).unwrap(), pack);
        assert_eq!(plain.pack_sha256, sha(&pack));
        let zipped = parse_push(&Spool::Memory(Bytes::from(gzip(&body))), true).unwrap();
        assert_eq!(zipped.head, plain.head);
        assert_eq!(zipped.pack_sha256, plain.pack_sha256);
    }

    #[test]
    fn probes_deletions_and_bad_bodies() {
        let probe = parse_push(&Spool::Memory(Bytes::from_static(b"0000")), false).unwrap();
        assert!(probe.head.commands.is_empty() && probe.pack.is_none());
        let delete = parse_push(&Spool::Memory(Bytes::from(push_body(b""))), false).unwrap();
        assert!(delete.pack.is_none());
        assert_eq!(delete.pack_sha256, sha(b""));
        assert!(matches!(parse_push(&Spool::Memory(Bytes::from_static(b"zzzz")), false), Err(BodyError::Invalid(_))));
        assert!(matches!(
            parse_push(&Spool::Memory(Bytes::from_static(b"not gzip")), true),
            Err(BodyError::Invalid(_) | BodyError::Io(_))
        ));
        // A gzip bomb stops at the limit instead of filling the disk: checked through the writer's limit.
        let mut w = SpoolWriter::new(10);
        assert!(matches!(copy_into(&[0u8; 11][..], &mut w), Err(BodyError::TooLarge(10))));
    }

    #[test]
    fn a_head_longer_than_the_limit_is_refused() {
        let mut body = encode(format!("{A} {B} refs/heads/b0\0report-status\n").as_bytes());
        let mut i = 1;
        while (body.len() as u64) < HEAD_LIMIT + 10 {
            body.extend(encode(format!("{A} {B} refs/heads/b{i}\n").as_bytes()));
            i += 1;
        }
        body.extend_from_slice(FLUSH);
        let err = parse_push(&Spool::Memory(Bytes::from(body)), false).unwrap_err();
        assert_eq!(err.to_string(), "too many ref updates in one push");
    }
}
