//! Moving blob bytes without holding them in memory: writing a stream to disk (hashing and sniffing on the way),
//! building request bodies from a file (as is, or base64 inside JSON), and byte ranges.

use std::path::{Path, PathBuf};

use data_encoding::{BASE64, HEXLOWER};
use futures::{Stream, StreamExt, stream};
use reins_proto::blob::SendBody;
use serde_json::{Map, Value};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

use super::{
    blob::Received,
    outbound::{BodySource, OutboundError},
    sniff::Sniffer,
};

/// Read size for streaming; a multiple of 3 so base64 needs no carry in the common case.
const CHUNK: usize = 48 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum WriteError {
    /// More than the allowed bytes arrived.
    TooLarge,
    /// The source broke off or the disk failed (no content in the text).
    Io(String),
}

/// Writes one blob file: created new and private, bytes counted, hashed and sniffed as they pass.
pub struct BlobWriter {
    file: tokio::fs::File,
    limit: u64,
    size: u64,
    hash: ring::digest::Context,
    sniffer: Sniffer,
}

impl BlobWriter {
    pub async fn create(path: &Path, limit: u64) -> Result<Self, WriteError> {
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let file = options.open(path).await.map_err(|e| WriteError::Io(format!("cannot create the blob file: {e}")))?;
        Ok(Self {
            file,
            limit,
            size: 0,
            hash: ring::digest::Context::new(&ring::digest::SHA256),
            sniffer: Sniffer::new(),
        })
    }

    pub async fn write(&mut self, chunk: &[u8]) -> Result<(), WriteError> {
        self.size = self.size.saturating_add(chunk.len() as u64);
        if self.size > self.limit {
            return Err(WriteError::TooLarge);
        }
        self.hash.update(chunk);
        self.sniffer.feed(chunk);
        self.file.write_all(chunk).await.map_err(|e| WriteError::Io(format!("cannot write the blob file: {e}")))
    }

    pub async fn finish(mut self) -> Result<Received, WriteError> {
        self.file.flush().await.map_err(|e| WriteError::Io(format!("cannot write the blob file: {e}")))?;
        let sniffed = self.sniffer.finish();
        Ok(Received {
            size: self.size,
            sha256: HEXLOWER.encode(self.hash.finish().as_ref()),
            content_type: sniffed.content_type,
            preview: sniffed.preview,
        })
    }
}

/// Streams `reader` into a new blob file at `path`, at most `limit` bytes.
pub async fn write_reader<R: AsyncRead + Unpin>(
    mut reader: R,
    path: &Path,
    limit: u64,
) -> Result<Received, WriteError> {
    let mut writer = BlobWriter::create(path, limit).await?;
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = reader.read(&mut buf).await.map_err(|_| WriteError::Io("the upload broke off".to_owned()))?;
        if n == 0 {
            return writer.finish().await;
        }
        writer.write(&buf[..n]).await?;
    }
}

/// Writes bytes held in memory as a blob file.
pub async fn write_bytes(bytes: &[u8], path: &Path) -> Result<Received, WriteError> {
    let mut writer = BlobWriter::create(path, bytes.len() as u64).await?;
    writer.write(bytes).await?;
    writer.finish().await
}

/// Length of the base64 form of `n` bytes (padded).
pub fn base64_len(n: u64) -> u64 {
    n.div_ceil(3) * 4
}

/// The JSON text around the base64 for [`SendBody::JsonBase64`]: everything before the value of `field`, and after.
pub fn json_frame(json: &Map<String, Value>, field: &str) -> Result<(String, String), String> {
    if field.is_empty() || field.len() > 100 || field.chars().any(char::is_control) {
        return Err("`body.field` must be a short JSON key".to_owned());
    }
    let mut rest = json.clone();
    rest.remove(field);
    let key = serde_json::to_string(field).map_err(|e| e.to_string())?;
    let mut prefix = serde_json::to_string(&rest).map_err(|e| e.to_string())?;
    prefix.pop(); // the closing brace
    if !rest.is_empty() {
        prefix.push(',');
    }
    prefix.push_str(&key);
    prefix.push_str(":\"");
    Ok((prefix, "\"}".to_owned()))
}

/// The bytes of a reader as base64 text, chunk by chunk.
fn base64_stream<R: AsyncRead + Unpin + Send + 'static>(
    reader: R,
) -> impl Stream<Item = Result<Vec<u8>, std::io::Error>> + Send + 'static {
    stream::try_unfold((reader, Vec::<u8>::new(), false), async move |(mut reader, mut carry, done)| {
        if done {
            return Ok(None);
        }
        let mut buf = vec![0u8; CHUNK];
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            let tail = BASE64.encode(&carry).into_bytes();
            return Ok(Some((tail, (reader, Vec::new(), true))));
        }
        carry.extend_from_slice(&buf[..n]);
        let whole = carry.len() - carry.len() % 3;
        let encoded = BASE64.encode(&carry[..whole]).into_bytes();
        let carry = carry.split_off(whole);
        Ok(Some((encoded, (reader, carry, false))))
    })
}

fn file_stream<R: AsyncRead + Unpin + Send + 'static>(
    reader: R,
) -> impl Stream<Item = Result<Vec<u8>, std::io::Error>> + Send + 'static {
    stream::try_unfold(reader, async move |mut reader| {
        let mut buf = vec![0u8; CHUNK];
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            return Ok(None);
        }
        buf.truncate(n);
        Ok(Some((buf, reader)))
    })
}

/// The body of a [`BlobSend`](reins_proto::blob::BlobSend): the blob file, as is or as base64 inside JSON. Opened
/// anew for each hop so a 307 can resend it.
pub struct FileBody {
    pub file: PathBuf,
    pub size: u64,
    pub mode: SendBody,
}

impl FileBody {
    /// The stream and its exact length.
    pub async fn open(
        &self,
    ) -> Result<(std::pin::Pin<Box<dyn Stream<Item = Result<Vec<u8>, std::io::Error>> + Send>>, u64), String> {
        let file = tokio::fs::File::open(&self.file).await.map_err(|_| "The file is gone.".to_owned())?;
        match &self.mode {
            SendBody::Raw => Ok((Box::pin(file_stream(file)), self.size)),
            SendBody::JsonBase64 {
                json,
                field,
            } => {
                let (prefix, suffix) = json_frame(json, field)?;
                let length = prefix.len() as u64 + base64_len(self.size) + suffix.len() as u64;
                let body = stream::once(async move { Ok(prefix.into_bytes()) })
                    .chain(base64_stream(file))
                    .chain(stream::once(async move { Ok(suffix.into_bytes()) }));
                Ok((Box::pin(body), length))
            }
        }
    }
}

impl BodySource for FileBody {
    async fn body(&self) -> Result<Option<(reqwest::Body, u64)>, OutboundError> {
        let (stream, length) = self.open().await.map_err(OutboundError::Refused)?;
        Ok(Some((reqwest::Body::wrap_stream(stream), length)))
    }
}

/// A `Range: bytes=…` request against a file of `size` bytes: `Ok(None)` for the whole file, `Ok(Some((start,
/// length)))` for one range, `Err(())` when it cannot be satisfied. Several ranges are answered whole.
pub fn parse_range(header: Option<&str>, size: u64) -> Result<Option<(u64, u64)>, ()> {
    let Some(spec) = header.and_then(|h| h.trim().strip_prefix("bytes=")) else {
        return Ok(None);
    };
    if spec.contains(',') {
        return Ok(None);
    }
    let Some((start, end)) = spec.split_once('-') else {
        return Ok(None);
    };
    let (start, end) = (start.trim(), end.trim());
    let parse = |s: &str| s.parse::<u64>().map_err(|_| ());
    let (first, last) = if start.is_empty() {
        let suffix = parse(end)?;
        if suffix == 0 {
            return Err(());
        }
        (size.saturating_sub(suffix), size.checked_sub(1).ok_or(())?)
    } else {
        let first = parse(start)?;
        let last = if end.is_empty() {
            size.checked_sub(1).ok_or(())?
        } else {
            parse(end)?.min(size.saturating_sub(1))
        };
        (first, last)
    };
    if first >= size || last < first {
        return Err(());
    }
    Ok(Some((first, last - first + 1)))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    async fn collect(body: &FileBody) -> (Vec<u8>, u64) {
        let (mut stream, length) = body.open().await.unwrap();
        let mut out = Vec::new();
        while let Some(chunk) = stream.next().await {
            out.extend_from_slice(&chunk.unwrap());
        }
        (out, length)
    }

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("reins-blob-io-{name}-{}", crate::util::get_uuid()))
    }

    #[tokio::test]
    async fn json_base64_bodies_wrap_the_file_and_know_their_length() {
        for size in [0usize, 1, 2, 3, 4, CHUNK - 1, CHUNK, CHUNK + 1, 3 * CHUNK + 2] {
            let path = temp("json");
            let data: Vec<u8> = (0..size).map(|i| u8::try_from(i % 251).unwrap()).collect();
            std::fs::write(&path, &data).unwrap();
            let json = json!({"message": "Add file", "branch": "main", "content": "replaced"});
            let body = FileBody {
                file: path.clone(),
                size: size as u64,
                mode: SendBody::JsonBase64 {
                    json: json.as_object().unwrap().clone(),
                    field: "content".to_owned(),
                },
            };
            let (bytes, length) = collect(&body).await;
            assert_eq!(bytes.len() as u64, length, "size {size}");
            let parsed: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(parsed["message"], "Add file");
            assert_eq!(BASE64.decode(parsed["content"].as_str().unwrap().as_bytes()).unwrap(), data, "size {size}");
            std::fs::remove_file(path).ok();
        }
    }

    #[tokio::test]
    async fn raw_bodies_are_the_file() {
        let path = temp("raw");
        std::fs::write(&path, b"hello world").unwrap();
        let body = FileBody {
            file: path.clone(),
            size: 11,
            mode: SendBody::Raw,
        };
        assert_eq!(collect(&body).await, (b"hello world".to_vec(), 11));
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn json_frames_handle_empty_objects_and_odd_keys() {
        let (prefix, suffix) = json_frame(&Map::new(), "data").unwrap();
        assert_eq!(format!("{prefix}QQ=={suffix}"), r#"{"data":"QQ=="}"#);
        let (prefix, _) = json_frame(json!({"a": 1}).as_object().unwrap(), "we\"ird").unwrap();
        assert_eq!(prefix, r#"{"a":1,"we\"ird":""#);
        assert!(json_frame(&Map::new(), "").is_err());
        assert_eq!((base64_len(0), base64_len(1), base64_len(3), base64_len(4)), (0, 4, 4, 8));
    }

    #[tokio::test]
    async fn writes_are_hashed_sniffed_and_bounded() {
        let path = temp("write");
        let received = write_reader(&b"hello"[..], &path, 5).await.unwrap();
        assert_eq!(received.size, 5);
        assert_eq!(received.sha256, "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");
        assert_eq!(received.content_type, super::super::sniff::TEXT_TYPE);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        assert!(write_reader(&b"again"[..], &path, 5).await.is_err(), "never overwrites");
        std::fs::remove_file(&path).ok();
        assert_eq!(write_reader(&b"toolong"[..], &path, 5).await.unwrap_err(), WriteError::TooLarge);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn ranges() {
        assert_eq!(parse_range(None, 10), Ok(None));
        assert_eq!(parse_range(Some("bytes=0-3"), 10), Ok(Some((0, 4))));
        assert_eq!(parse_range(Some("bytes=5-"), 10), Ok(Some((5, 5))));
        assert_eq!(parse_range(Some("bytes=-3"), 10), Ok(Some((7, 3))));
        assert_eq!(parse_range(Some("bytes=-30"), 10), Ok(Some((0, 10))));
        assert_eq!(parse_range(Some("bytes=8-100"), 10), Ok(Some((8, 2))));
        assert_eq!(parse_range(Some("bytes=0-1,4-5"), 10), Ok(None));
        assert_eq!(parse_range(Some("items=0-1"), 10), Ok(None));
        assert_eq!(parse_range(Some("bytes=10-"), 10), Err(()));
        assert_eq!(parse_range(Some("bytes=5-2"), 10), Err(()));
        assert_eq!(parse_range(Some("bytes=-0"), 10), Err(()));
        assert_eq!(parse_range(Some("bytes=x-1"), 10), Err(()));
        assert_eq!(parse_range(Some("bytes=0-"), 0), Err(()));
    }
}
