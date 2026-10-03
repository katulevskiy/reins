//! The SSH wire encoding (RFC 4251 §5): `uint32`, `string` (length-prefixed bytes), `mpint`, `boolean`, `byte`.

/// Reads SSH-encoded values from a byte slice; every read fails cleanly on short or malformed input.
pub struct Reader<'a> {
    buf: &'a [u8],
}

impl<'a> Reader<'a> {
    #[must_use]
    pub fn new(buf: &'a [u8]) -> Self {
        Self {
            buf,
        }
    }

    pub fn byte(&mut self) -> Option<u8> {
        let (&b, rest) = self.buf.split_first()?;
        self.buf = rest;
        Some(b)
    }

    pub fn bool(&mut self) -> Option<bool> {
        self.byte().map(|b| b != 0)
    }

    pub fn u32(&mut self) -> Option<u32> {
        let (head, rest) = self.buf.split_first_chunk::<4>()?;
        self.buf = rest;
        Some(u32::from_be_bytes(*head))
    }

    pub fn string(&mut self) -> Option<&'a [u8]> {
        let len = usize::try_from(self.u32()?).ok()?;
        if len > self.buf.len() {
            return None;
        }
        let (s, rest) = self.buf.split_at(len);
        self.buf = rest;
        Some(s)
    }

    pub fn str(&mut self) -> Option<&'a str> {
        std::str::from_utf8(self.string()?).ok()
    }

    /// An `mpint` as unsigned big-endian bytes without leading zeros (negative numbers are refused).
    pub fn mpint(&mut self) -> Option<&'a [u8]> {
        let s = self.string()?;
        if s.first().is_some_and(|b| b & 0x80 != 0) {
            return None;
        }
        let start = s.iter().position(|&b| b != 0).unwrap_or(s.len());
        Some(&s[start..])
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
}

/// Builds SSH-encoded values.
#[derive(Default)]
pub struct Writer {
    pub buf: Vec<u8>,
}

impl Writer {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn byte(&mut self, b: u8) -> &mut Self {
        self.buf.push(b);
        self
    }

    pub fn u32(&mut self, n: u32) -> &mut Self {
        self.buf.extend_from_slice(&n.to_be_bytes());
        self
    }

    pub fn string(&mut self, s: &[u8]) -> &mut Self {
        self.u32(u32::try_from(s.len()).unwrap_or(u32::MAX));
        self.buf.extend_from_slice(s);
        self
    }

    /// An `mpint` from unsigned big-endian bytes.
    pub fn mpint(&mut self, n: &[u8]) -> &mut Self {
        let start = n.iter().position(|&b| b != 0).unwrap_or(n.len());
        let n = &n[start..];
        if n.first().is_some_and(|b| b & 0x80 != 0) {
            let mut padded = Vec::with_capacity(n.len() + 1);
            padded.push(0);
            padded.extend_from_slice(n);
            self.string(&padded)
        } else {
            self.string(n)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_round_trip_and_short_input_fails() {
        let mut w = Writer::new();
        w.byte(5).u32(7).string(b"hi").mpint(&[0, 0x80, 1]).mpint(&[0, 0, 1]);
        let mut r = Reader::new(&w.buf);
        assert_eq!(r.byte(), Some(5));
        assert_eq!(r.u32(), Some(7));
        assert_eq!(r.str(), Some("hi"));
        assert_eq!(r.mpint(), Some(&[0x80, 1][..]));
        assert_eq!(r.mpint(), Some(&[1][..]));
        assert!(r.is_empty());
        assert_eq!(r.byte(), None);
        assert_eq!(Reader::new(&[0, 0, 0, 9, 1]).string(), None);
        assert_eq!(Reader::new(&[0, 0, 0, 1, 0x80]).mpint(), None, "negative");
    }
}
