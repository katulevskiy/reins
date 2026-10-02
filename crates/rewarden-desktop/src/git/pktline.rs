//! Git pkt-lines (`0005a`, `0000` flush, `0001` delim), the head of a receive-pack request (the ref update commands,
//! push options and where the pack starts) and side-band framing for answers.

pub const FLUSH: &[u8] = b"0000";
pub const DELIM: &[u8] = b"0001";
/// Largest pkt-line git allows (length prefix included).
pub const MAX_PKT: usize = 65_520;
/// Side-band-64k payload limit (the band byte and the length prefix take 5).
pub const MAX_BAND_PAYLOAD: usize = MAX_PKT - 5;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum PktError {
    #[error("truncated pkt-line")]
    Truncated,
    #[error("malformed pkt-line length")]
    BadLength,
    #[error("malformed ref update command")]
    BadCommand,
    #[error("signed pushes are not supported")]
    SignedPush,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pkt<'a> {
    Flush,
    Delim,
    ResponseEnd,
    Data(&'a [u8]),
}

/// One pkt-line carrying `data` (which must be shorter than [`MAX_PKT`] - 4).
#[must_use]
pub fn encode(data: &[u8]) -> Vec<u8> {
    debug_assert!(data.len() + 4 <= MAX_PKT);
    let mut out = format!("{:04x}", data.len() + 4).into_bytes();
    out.extend_from_slice(data);
    out
}

/// `data` on side-band `band` (1 data, 2 progress, 3 error), split into as many packets as needed.
#[must_use]
pub fn sideband(band: u8, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 16);
    for chunk in data.chunks(MAX_BAND_PAYLOAD) {
        let mut payload = Vec::with_capacity(chunk.len() + 1);
        payload.push(band);
        payload.extend_from_slice(chunk);
        out.extend_from_slice(&encode(&payload));
    }
    out
}

/// Reads pkt-lines from a byte slice.
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    #[must_use]
    pub fn new(buf: &'a [u8]) -> Self {
        Self {
            buf,
            pos: 0,
        }
    }

    /// Bytes consumed so far.
    #[must_use]
    pub fn position(&self) -> usize {
        self.pos
    }

    /// The next packet; `Ok(None)` at the end of the input.
    pub fn next_pkt(&mut self) -> Result<Option<Pkt<'a>>, PktError> {
        if self.pos == self.buf.len() {
            return Ok(None);
        }
        let head = self.buf.get(self.pos..self.pos + 4).ok_or(PktError::Truncated)?;
        let text = std::str::from_utf8(head).map_err(|_| PktError::BadLength)?;
        let len = usize::from_str_radix(text, 16).map_err(|_| PktError::BadLength)?;
        let pkt = match len {
            0 => Pkt::Flush,
            1 => Pkt::Delim,
            2 => Pkt::ResponseEnd,
            3 => return Err(PktError::BadLength),
            n if n > MAX_PKT => return Err(PktError::BadLength),
            n => Pkt::Data(self.buf.get(self.pos + 4..self.pos + n).ok_or(PktError::Truncated)?),
        };
        self.pos += len.max(4);
        Ok(Some(pkt))
    }
}

/// One `old new ref` line of a push.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub old: String,
    pub new: String,
    pub name: String,
}

/// The part of a receive-pack request before the pack.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReceiveHead {
    pub commands: Vec<Command>,
    /// What the client asked for on the first command line (`report-status`, `side-band-64k`, `push-options`, ...).
    pub capabilities: Vec<String>,
    pub push_options: Vec<String>,
    /// `shallow <oid>` lines the client sent first.
    pub shallow: Vec<String>,
    /// Where the pack starts in the body (the body's length when there is none: deletions only, or a probe).
    pub pack_offset: usize,
}

impl ReceiveHead {
    #[must_use]
    pub fn has_capability(&self, name: &str) -> bool {
        self.capabilities.iter().any(|c| c == name || c.split_once('=').is_some_and(|(k, _)| k == name))
    }
}

fn is_hex_oid(s: &str) -> bool {
    (s.len() == 40 || s.len() == 64) && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn line(data: &[u8]) -> Result<&str, PktError> {
    let text = std::str::from_utf8(data).map_err(|_| PktError::BadCommand)?;
    Ok(text.strip_suffix('\n').unwrap_or(text))
}

/// Parses the commands, capabilities and push options at the start of a `git-receive-pack` request body. `head` may be
/// a prefix of the body; it must contain everything up to the pack. A body that is just a flush (git's probe before a
/// large chunked request) gives no commands.
pub fn parse_receive_head(head: &[u8]) -> Result<ReceiveHead, PktError> {
    let mut r = Reader::new(head);
    let mut out = ReceiveHead::default();
    loop {
        match r.next_pkt()? {
            None | Some(Pkt::Flush) => break,
            Some(Pkt::Delim | Pkt::ResponseEnd) => return Err(PktError::BadCommand),
            Some(Pkt::Data(data)) => {
                let (cmd, caps) = match data.iter().position(|&b| b == 0) {
                    Some(i) => (&data[..i], Some(&data[i + 1..])),
                    None => (data, None),
                };
                let cmd = line(cmd)?;
                if cmd.starts_with("push-cert") {
                    return Err(PktError::SignedPush);
                }
                if let Some(oid) = cmd.strip_prefix("shallow ") {
                    if !is_hex_oid(oid) {
                        return Err(PktError::BadCommand);
                    }
                    out.shallow.push(oid.to_owned());
                    continue;
                }
                let mut parts = cmd.splitn(3, ' ');
                let (Some(old), Some(new), Some(name)) = (parts.next(), parts.next(), parts.next()) else {
                    return Err(PktError::BadCommand);
                };
                if !is_hex_oid(old) || !is_hex_oid(new) || name.is_empty() || name.contains(['\0', ' ']) {
                    return Err(PktError::BadCommand);
                }
                if let Some(caps) = caps {
                    if !out.commands.is_empty() {
                        return Err(PktError::BadCommand);
                    }
                    out.capabilities = line(caps)?.split(' ').filter(|c| !c.is_empty()).map(str::to_owned).collect();
                }
                out.commands.push(Command {
                    old: old.to_owned(),
                    new: new.to_owned(),
                    name: name.to_owned(),
                });
            }
        }
    }
    if !out.commands.is_empty() && out.has_capability("push-options") {
        loop {
            match r.next_pkt()? {
                None | Some(Pkt::Flush) => break,
                Some(Pkt::Data(data)) => out.push_options.push(line(data)?.to_owned()),
                Some(_) => return Err(PktError::BadCommand),
            }
        }
    }
    out.pack_offset = r.position();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "1111111111111111111111111111111111111111";
    const B: &str = "2222222222222222222222222222222222222222";

    #[test]
    fn packets_round_trip_and_bad_lengths_are_caught() {
        let mut body = encode(b"hello\n");
        body.extend_from_slice(FLUSH);
        body.extend_from_slice(DELIM);
        let mut r = Reader::new(&body);
        assert_eq!(r.next_pkt().unwrap(), Some(Pkt::Data(b"hello\n")));
        assert_eq!(r.next_pkt().unwrap(), Some(Pkt::Flush));
        assert_eq!(r.next_pkt().unwrap(), Some(Pkt::Delim));
        assert_eq!(r.next_pkt().unwrap(), None);
        assert_eq!(Reader::new(b"00").next_pkt(), Err(PktError::Truncated));
        assert_eq!(Reader::new(b"zzzz").next_pkt(), Err(PktError::BadLength));
        assert_eq!(Reader::new(b"0003").next_pkt(), Err(PktError::BadLength));
        assert_eq!(Reader::new(b"0009ab").next_pkt(), Err(PktError::Truncated));
    }

    #[test]
    fn a_push_head_gives_commands_capabilities_options_and_the_pack_offset() {
        let mut body = encode(
            format!("{A} {B} refs/heads/main\0report-status side-band-64k push-options agent=git/2.55\n").as_bytes(),
        );
        body.extend(encode(format!("0000000000000000000000000000000000000000 {B} refs/heads/new").as_bytes()));
        body.extend_from_slice(FLUSH);
        body.extend(encode(b"ci.skip\n"));
        body.extend_from_slice(FLUSH);
        let offset = body.len();
        body.extend_from_slice(b"PACK\0\0\0\x02");
        let head = parse_receive_head(&body).unwrap();
        assert_eq!(head.commands.len(), 2);
        assert_eq!(
            head.commands[0],
            Command {
                old: A.into(),
                new: B.into(),
                name: "refs/heads/main".into()
            }
        );
        assert_eq!(head.commands[1].name, "refs/heads/new");
        assert!(head.has_capability("side-band-64k"));
        assert!(head.has_capability("agent"));
        assert_eq!(head.push_options, vec!["ci.skip"]);
        assert_eq!(head.pack_offset, offset);
        assert_eq!(&body[head.pack_offset..head.pack_offset + 4], b"PACK");
    }

    #[test]
    fn a_probe_has_no_commands_and_odd_input_is_refused() {
        let probe = parse_receive_head(FLUSH).unwrap();
        assert!(probe.commands.is_empty());
        assert_eq!(probe.pack_offset, 4);
        assert_eq!(parse_receive_head(&encode(b"push-cert\0caps")), Err(PktError::SignedPush));
        assert_eq!(parse_receive_head(&encode(b"abc def refs/heads/x")), Err(PktError::BadCommand));
        assert_eq!(parse_receive_head(&encode(format!("{A} {B} ").as_bytes())), Err(PktError::BadCommand));
        let mut two_caps = encode(format!("{A} {B} refs/heads/a\0x").as_bytes());
        two_caps.extend(encode(format!("{A} {B} refs/heads/b\0y").as_bytes()));
        assert_eq!(parse_receive_head(&two_caps), Err(PktError::BadCommand));
    }

    #[test]
    fn sideband_splits_long_payloads() {
        let data = vec![b'x'; MAX_BAND_PAYLOAD + 10];
        let framed = sideband(2, &data);
        let mut r = Reader::new(&framed);
        let Some(Pkt::Data(first)) = r.next_pkt().unwrap() else {
            panic!()
        };
        assert_eq!((first[0], first.len()), (2, MAX_BAND_PAYLOAD + 1));
        let Some(Pkt::Data(second)) = r.next_pkt().unwrap() else {
            panic!()
        };
        assert_eq!((second[0], second.len()), (2, 11));
        assert_eq!(r.next_pkt().unwrap(), None);
    }
}
