//! Reading the pack a push sends, from a file, without holding it in memory: one pass checks every zlib stream and the
//! trailer and hashes whole objects as they stream by; deltas are then resolved base first, inflating on demand with a
//! size-bounded cache. Thin packs build on objects the server has: [`Pack::missing_bases`] says which, and
//! [`Pack::add_base`] takes them (checked against their id).

use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Arc;

use flate2::{Decompress, FlushDecompress, Status};
use sha1::{Digest, Sha1};

use super::object::{Hasher, ObjectKind, Oid, hash_object};

/// Largest object read into memory for the analysis.
pub const MAX_OBJECT: u64 = 64 << 20;
/// Largest delta (instructions and inserted data) applied: a delta of an object near [`MAX_OBJECT`] stays under it.
const MAX_DELTA: u64 = 2 * MAX_OBJECT;
const CACHE_BYTES: u64 = 96 << 20;
const EXTERNAL_BYTES: u64 = 256 << 20;
const MAX_CHAIN: usize = 10_000;
const READ_BUF: usize = 128 * 1024;
const TRAILER: u64 = 20;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PackError {
    #[error("the pack is truncated")]
    Truncated,
    #[error("the pack is damaged: {0}")]
    Corrupt(&'static str),
    #[error("pack version {0} is not supported")]
    Version(u32),
    #[error("the object is larger than 64 MiB")]
    TooLarge,
    #[error("reading the pack failed: {0}")]
    Io(String),
}

impl From<std::io::Error> for PackError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}

fn usize_of(n: u64) -> Result<usize, PackError> {
    usize::try_from(n).map_err(|_| PackError::TooLarge)
}

/// An object's kind and content.
pub type Object = (ObjectKind, Arc<[u8]>);

#[derive(Clone, Copy, Debug)]
enum Base {
    Whole(ObjectKind),
    Ofs(u64),
    Ref(Oid),
}

#[derive(Clone, Copy, Debug)]
struct Resolved {
    oid: Oid,
    kind: ObjectKind,
    len: u64,
}

#[derive(Debug)]
struct Entry {
    offset: u64,
    data_offset: u64,
    /// Inflated length of the entry's data: the object, or the delta.
    data_len: u64,
    base: Base,
    /// Known for whole objects after the scan, for deltas once their base is.
    resolved: Option<Resolved>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Loc {
    Entry(usize),
    External(usize),
}

/// Least recently used objects, bounded by their total size.
#[derive(Default)]
struct Cache {
    items: HashMap<usize, (Arc<[u8]>, u64)>,
    order: BTreeMap<u64, usize>,
    tick: u64,
    bytes: u64,
}

impl Cache {
    fn get(&mut self, idx: usize) -> Option<Arc<[u8]>> {
        let (data, tick) = self.items.get_mut(&idx)?;
        self.order.remove(tick);
        self.tick += 1;
        *tick = self.tick;
        self.order.insert(self.tick, idx);
        Some(Arc::clone(data))
    }

    fn put(&mut self, idx: usize, data: &Arc<[u8]>) {
        let len = data.len() as u64;
        if len > CACHE_BYTES / 2 || self.items.contains_key(&idx) {
            return;
        }
        self.tick += 1;
        self.items.insert(idx, (Arc::clone(data), self.tick));
        self.order.insert(self.tick, idx);
        self.bytes += len;
        while self.bytes > CACHE_BYTES {
            let Some((_, old)) = self.order.pop_first() else {
                break;
            };
            if let Some((d, _)) = self.items.remove(&old) {
                self.bytes -= d.len() as u64;
            }
        }
    }
}

/// A pack on disk with an index of its entries.
pub struct Pack {
    file: File,
    entries: Vec<Entry>,
    by_oid: HashMap<Oid, Loc>,
    externals: Vec<(Oid, ObjectKind, Arc<[u8]>)>,
    external_bytes: u64,
    /// Entry indexes of `OFS_DELTA`s by the offset of their base.
    ofs_children: HashMap<u64, Vec<usize>>,
    /// Entry indexes of `REF_DELTA`s whose base is not known yet.
    ref_waiting: HashMap<Oid, Vec<usize>>,
    cache: Cache,
    damage: Option<PackError>,
}

impl std::fmt::Debug for Pack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pack").field("entries", &self.entries.len()).finish_non_exhaustive()
    }
}

impl Pack {
    /// Reads and checks the whole pack once (zlib streams, declared sizes, the trailing SHA-1), then resolves every
    /// delta whose base is in the pack. Deltas that build on something else wait for [`Pack::add_base`].
    pub fn open(path: &Path) -> Result<Self, PackError> {
        let file = File::open(path)?;
        let len = file.metadata()?.len();
        if len < 12 + TRAILER {
            return Err(PackError::Truncated);
        }
        let mut s = Scanner::new(file.try_clone()?, len - TRAILER);
        let mut head = [0u8; 12];
        s.read_exact(&mut head)?;
        if &head[..4] != b"PACK" {
            return Err(PackError::Corrupt("it does not start with PACK"));
        }
        let version = u32::from_be_bytes([head[4], head[5], head[6], head[7]]);
        if version != 2 && version != 3 {
            return Err(PackError::Version(version));
        }
        let count = u32::from_be_bytes([head[8], head[9], head[10], head[11]]);
        let mut entries: Vec<Entry> = Vec::new();
        let mut by_oid = HashMap::new();
        let mut ofs_children: HashMap<u64, Vec<usize>> = HashMap::new();
        let mut ref_waiting: HashMap<Oid, Vec<usize>> = HashMap::new();
        for idx in 0..count as usize {
            let offset = s.position();
            let (ty, size) = s.entry_header()?;
            let base = match ty {
                1 => Base::Whole(ObjectKind::Commit),
                2 => Base::Whole(ObjectKind::Tree),
                3 => Base::Whole(ObjectKind::Blob),
                4 => Base::Whole(ObjectKind::Tag),
                6 => {
                    let back = s.ofs_distance()?;
                    let base = offset.checked_sub(back).ok_or(PackError::Corrupt("a delta base is before the pack"))?;
                    if entries.binary_search_by_key(&base, |e| e.offset).is_err() {
                        return Err(PackError::Corrupt("a delta base is not an object"));
                    }
                    ofs_children.entry(base).or_default().push(idx);
                    Base::Ofs(base)
                }
                7 => {
                    let mut raw = [0u8; 20];
                    s.read_exact(&mut raw)?;
                    let oid = Oid(raw);
                    ref_waiting.entry(oid).or_default().push(idx);
                    Base::Ref(oid)
                }
                _ => return Err(PackError::Corrupt("an object has an unknown type")),
            };
            let data_offset = s.position();
            let resolved = if let Base::Whole(kind) = base {
                let mut h = Hasher::new(kind, size);
                s.inflate(size, |chunk| h.update(chunk))?;
                let oid = h.finish();
                by_oid.entry(oid).or_insert(Loc::Entry(idx));
                Some(Resolved {
                    oid,
                    kind,
                    len: size,
                })
            } else {
                s.inflate(size, |_| {})?;
                None
            };
            entries.push(Entry {
                offset,
                data_offset,
                data_len: size,
                base,
                resolved,
            });
        }
        s.finish()?;
        let mut pack = Self {
            file,
            entries,
            by_oid,
            externals: Vec::new(),
            external_bytes: 0,
            ofs_children,
            ref_waiting,
            cache: Cache::default(),
            damage: None,
        };
        let whole: Vec<Loc> = (0..pack.entries.len())
            .filter(|&i| matches!(pack.entries[i].base, Base::Whole(_)))
            .map(Loc::Entry)
            .collect();
        pack.resolve_from(whole);
        Ok(pack)
    }

    /// Objects in the pack (entries).
    #[must_use]
    pub fn object_count(&self) -> usize {
        self.entries.len()
    }

    /// How many entries are `OFS_DELTA`s and `REF_DELTA`s.
    #[must_use]
    pub fn delta_counts(&self) -> (usize, usize) {
        self.entries.iter().fold((0, 0), |(o, r), e| match e.base {
            Base::Whole(_) => (o, r),
            Base::Ofs(_) => (o + 1, r),
            Base::Ref(_) => (o, r + 1),
        })
    }

    /// Entries whose id is still unknown: their base is missing, too large, or damaged.
    #[must_use]
    pub fn unresolved(&self) -> usize {
        self.entries.iter().filter(|e| e.resolved.is_none()).count()
    }

    /// A delta that could not be applied (the pack is damaged; git would refuse it too).
    #[must_use]
    pub fn damage(&self) -> Option<&PackError> {
        self.damage.as_ref()
    }

    /// Bases of `REF_DELTA`s that are not in the pack (a thin pack builds on the server's objects), in pack order.
    #[must_use]
    pub fn missing_bases(&self) -> Vec<Oid> {
        let mut out: Vec<(usize, Oid)> =
            self.ref_waiting.iter().map(|(oid, idxs)| (idxs.iter().copied().min().unwrap_or(0), *oid)).collect();
        out.sort_unstable();
        out.into_iter().map(|(_, oid)| oid).collect()
    }

    /// An object from outside the pack that deltas build on. Refused unless `oid` is really its id.
    pub fn add_base(&mut self, oid: Oid, kind: ObjectKind, data: Vec<u8>) -> Result<(), PackError> {
        if self.by_oid.contains_key(&oid) {
            return Ok(());
        }
        if hash_object(kind, &data) != oid {
            return Err(PackError::Corrupt("a base object does not match its id"));
        }
        let len = data.len() as u64;
        if len > MAX_OBJECT || self.external_bytes + len > EXTERNAL_BYTES {
            return Err(PackError::TooLarge);
        }
        self.external_bytes += len;
        self.externals.push((oid, kind, data.into()));
        let loc = Loc::External(self.externals.len() - 1);
        self.by_oid.insert(oid, loc);
        self.resolve_from(vec![loc]);
        Ok(())
    }

    /// Whether the pack itself carries `oid` (not a base added from the server).
    #[must_use]
    pub fn has(&self, oid: &Oid) -> bool {
        matches!(self.by_oid.get(oid), Some(Loc::Entry(_)))
    }

    /// Kind and size of an object the pack knows, without reading it.
    #[must_use]
    pub fn header(&self, oid: &Oid) -> Option<(ObjectKind, u64)> {
        match *self.by_oid.get(oid)? {
            Loc::Entry(i) => self.entries[i].resolved.map(|r| (r.kind, r.len)),
            Loc::External(i) => Some((self.externals[i].1, self.externals[i].2.len() as u64)),
        }
    }

    /// The object, when the pack knows it (`Ok(None)` when not).
    pub fn read(&mut self, oid: &Oid) -> Result<Option<Object>, PackError> {
        match self.by_oid.get(oid).copied() {
            None => Ok(None),
            Some(Loc::External(i)) => Ok(Some((self.externals[i].1, Arc::clone(&self.externals[i].2)))),
            Some(Loc::Entry(i)) => {
                let Some(r) = self.entries[i].resolved else {
                    return Ok(None);
                };
                if r.len > MAX_OBJECT {
                    return Err(PackError::TooLarge);
                }
                Ok(Some((r.kind, self.load(i)?)))
            }
        }
    }

    /// Resolves, depth first, every delta that builds on the given objects and on what they resolve.
    fn resolve_from(&mut self, mut stack: Vec<Loc>) {
        while let Some(node) = stack.pop() {
            let (oid, kind, offset) = match node {
                Loc::Entry(i) => {
                    let Some(r) = self.entries[i].resolved else {
                        continue;
                    };
                    (r.oid, r.kind, Some(self.entries[i].offset))
                }
                Loc::External(i) => (self.externals[i].0, self.externals[i].1, None),
            };
            let mut children = self.ref_waiting.remove(&oid).unwrap_or_default();
            if let Some(children_by_offset) = offset.and_then(|o| self.ofs_children.remove(&o)) {
                children.extend(children_by_offset);
            }
            for child in children {
                if self.entries[child].resolved.is_some() {
                    continue;
                }
                match self.load(child) {
                    Ok(data) => {
                        let oid = hash_object(kind, &data);
                        self.entries[child].resolved = Some(Resolved {
                            oid,
                            kind,
                            len: data.len() as u64,
                        });
                        self.by_oid.entry(oid).or_insert(Loc::Entry(child));
                        stack.push(Loc::Entry(child));
                    }
                    // Left unresolved: the analysis notes what it cannot read.
                    Err(PackError::TooLarge) => {}
                    Err(e) => {
                        self.damage.get_or_insert(e);
                    }
                }
            }
        }
    }

    /// The content of entry `idx`, applying its delta chain (from the cache where possible).
    fn load(&mut self, idx: usize) -> Result<Arc<[u8]>, PackError> {
        if let Some(d) = self.cache.get(idx) {
            return Ok(d);
        }
        let mut chain = Vec::new();
        let mut cur = idx;
        let mut data: Arc<[u8]> = loop {
            if cur != idx
                && let Some(d) = self.cache.get(cur)
            {
                break d;
            }
            let e = &self.entries[cur];
            match e.base {
                Base::Whole(_) => {
                    if e.data_len > MAX_OBJECT {
                        return Err(PackError::TooLarge);
                    }
                    let d: Arc<[u8]> = self.inflate_at(e.data_offset, e.data_len)?.into();
                    self.cache.put(cur, &d);
                    break d;
                }
                Base::Ofs(off) => {
                    chain.push(cur);
                    cur = self
                        .entries
                        .binary_search_by_key(&off, |e| e.offset)
                        .map_err(|_| PackError::Corrupt("a delta base is not an object"))?;
                }
                Base::Ref(oid) => {
                    chain.push(cur);
                    match self.by_oid.get(&oid) {
                        Some(Loc::Entry(i)) => cur = *i,
                        Some(Loc::External(i)) => break Arc::clone(&self.externals[*i].2),
                        None => return Err(PackError::Corrupt("a delta base is missing")),
                    }
                }
            }
            if chain.len() > MAX_CHAIN {
                return Err(PackError::Corrupt("a delta chain is too long"));
            }
        };
        for &i in chain.iter().rev() {
            let (offset, len) = (self.entries[i].data_offset, self.entries[i].data_len);
            if len > MAX_DELTA {
                return Err(PackError::TooLarge);
            }
            let delta = self.inflate_at(offset, len)?;
            data = apply_delta(&data, &delta, MAX_OBJECT)?.into();
            self.cache.put(i, &data);
        }
        Ok(data)
    }

    fn inflate_at(&mut self, offset: u64, len: u64) -> Result<Vec<u8>, PackError> {
        self.file.seek(SeekFrom::Start(offset))?;
        let mut out = Vec::with_capacity(usize_of(len)?);
        let mut z = flate2::read::ZlibDecoder::new(std::io::BufReader::new(&self.file)).take(len + 1);
        z.read_to_end(&mut out).map_err(|_| PackError::Corrupt("an object does not inflate"))?;
        if out.len() as u64 != len {
            return Err(PackError::Corrupt("an object does not have its declared size"));
        }
        Ok(out)
    }
}

/// Sequential reader for the scan: buffered, hashing everything it reads for the trailer, never past `limit`.
struct Scanner {
    file: File,
    buf: Vec<u8>,
    start: usize,
    end: usize,
    /// File offset just past `buf[end - 1]`.
    read: u64,
    limit: u64,
    hasher: Sha1,
    /// Reused for every entry (a fresh inflater per object is costly in packs of many small objects).
    z: Decompress,
    out: Vec<u8>,
}

impl Scanner {
    fn new(file: File, limit: u64) -> Self {
        Self {
            file,
            buf: vec![0; READ_BUF],
            start: 0,
            end: 0,
            read: 0,
            limit,
            hasher: Sha1::new(),
            z: Decompress::new(true),
            out: vec![0; 32 * 1024],
        }
    }

    fn position(&self) -> u64 {
        self.read - (self.end - self.start) as u64
    }

    /// Reads more after what is buffered; 0 at the limit (or with a full buffer).
    fn fill(&mut self) -> Result<usize, PackError> {
        if self.start > 0 {
            self.buf.copy_within(self.start..self.end, 0);
            self.end -= self.start;
            self.start = 0;
        }
        let room = (self.buf.len() - self.end).min(usize_of(self.limit - self.read).unwrap_or(usize::MAX));
        if room == 0 {
            return Ok(0);
        }
        let n = loop {
            match self.file.read(&mut self.buf[self.end..self.end + room]) {
                Ok(n) => break n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e.into()),
            }
        };
        self.hasher.update(&self.buf[self.end..self.end + n]);
        self.end += n;
        self.read += n as u64;
        Ok(n)
    }

    fn byte(&mut self) -> Result<u8, PackError> {
        if self.start == self.end && self.fill()? == 0 {
            return Err(PackError::Truncated);
        }
        self.start += 1;
        Ok(self.buf[self.start - 1])
    }

    fn read_exact(&mut self, out: &mut [u8]) -> Result<(), PackError> {
        for b in out {
            *b = self.byte()?;
        }
        Ok(())
    }

    /// Type (3 bits) and inflated size (little-endian base-128) of an entry.
    fn entry_header(&mut self) -> Result<(u8, u64), PackError> {
        let mut c = self.byte()?;
        let ty = (c >> 4) & 7;
        let mut size = u64::from(c & 0x0f);
        let mut shift = 4;
        while c & 0x80 != 0 {
            c = self.byte()?;
            if shift > 57 {
                return Err(PackError::Corrupt("an object size is too large"));
            }
            size |= u64::from(c & 0x7f) << shift;
            shift += 7;
        }
        Ok((ty, size))
    }

    /// How far back an `OFS_DELTA` base is (git's "offset encoding": big-endian base-128, plus one per extra byte).
    fn ofs_distance(&mut self) -> Result<u64, PackError> {
        let mut c = self.byte()?;
        let mut n = u64::from(c & 0x7f);
        while c & 0x80 != 0 {
            c = self.byte()?;
            n = n
                .checked_add(1)
                .and_then(|n| n.checked_mul(128))
                .ok_or(PackError::Corrupt("a delta offset is too large"))?
                | u64::from(c & 0x7f);
        }
        if n == 0 {
            return Err(PackError::Corrupt("a delta is its own base"));
        }
        Ok(n)
    }

    /// Inflates one zlib stream, which must give exactly `size` bytes, passing the output to `sink`.
    fn inflate(&mut self, size: u64, mut sink: impl FnMut(&[u8])) -> Result<(), PackError> {
        self.z.reset(true);
        loop {
            if self.start == self.end && self.fill()? == 0 {
                return Err(PackError::Truncated);
            }
            let (in0, out0) = (self.z.total_in(), self.z.total_out());
            let status = self
                .z
                .decompress(&self.buf[self.start..self.end], &mut self.out, FlushDecompress::None)
                .map_err(|_| PackError::Corrupt("an object does not inflate"))?;
            let used = usize_of(self.z.total_in() - in0)?;
            let made = usize_of(self.z.total_out() - out0)?;
            self.start += used;
            if self.z.total_out() > size {
                return Err(PackError::Corrupt("an object is larger than declared"));
            }
            sink(&self.out[..made]);
            match status {
                Status::StreamEnd => break,
                Status::Ok | Status::BufError if used == 0 && made == 0 => {
                    if self.fill()? == 0 {
                        return Err(if self.read == self.limit {
                            PackError::Truncated
                        } else {
                            PackError::Corrupt("an object does not inflate")
                        });
                    }
                }
                Status::Ok | Status::BufError => {}
            }
        }
        if self.z.total_out() != size {
            return Err(PackError::Corrupt("an object is smaller than declared"));
        }
        Ok(())
    }

    /// Checks that the objects end where the trailer starts and that the trailer is their SHA-1.
    fn finish(mut self) -> Result<(), PackError> {
        if self.start != self.end || self.fill()? != 0 {
            return Err(PackError::Corrupt("there is data after the last object"));
        }
        let mut trailer = [0u8; 20];
        self.file.seek(SeekFrom::Start(self.limit))?;
        self.file.read_exact(&mut trailer)?;
        let sum: [u8; 20] = self.hasher.finalize().into();
        if sum != trailer {
            return Err(PackError::Corrupt("the checksum does not match"));
        }
        Ok(())
    }
}

/// Size header of a delta: little-endian base-128.
fn delta_varint(delta: &[u8], pos: &mut usize) -> Result<u64, PackError> {
    let mut n = 0u64;
    let mut shift = 0;
    loop {
        let c = *delta.get(*pos).ok_or(PackError::Corrupt("a delta is truncated"))?;
        *pos += 1;
        if shift > 63 {
            return Err(PackError::Corrupt("a delta size is too large"));
        }
        n |= u64::from(c & 0x7f) << shift;
        shift += 7;
        if c & 0x80 == 0 {
            return Ok(n);
        }
    }
}

/// Applies a git delta (copy from `base` / insert literal) to `base`; refuses results larger than `max`.
pub fn apply_delta(base: &[u8], delta: &[u8], max: u64) -> Result<Vec<u8>, PackError> {
    const BAD: PackError = PackError::Corrupt("a delta does not apply");
    let mut p = 0;
    if delta_varint(delta, &mut p)? != base.len() as u64 {
        return Err(PackError::Corrupt("a delta does not fit its base"));
    }
    let target = delta_varint(delta, &mut p)?;
    if target > max {
        return Err(PackError::TooLarge);
    }
    let target = usize_of(target)?;
    let mut out = Vec::with_capacity(target);
    while p < delta.len() {
        let op = delta[p];
        p += 1;
        if op & 0x80 != 0 {
            let mut field = |bits: u8, count: usize| -> Result<usize, PackError> {
                let mut v = 0usize;
                for i in 0..count {
                    if bits & (1 << i) != 0 {
                        v |= usize::from(*delta.get(p).ok_or(BAD)?) << (8 * i);
                        p += 1;
                    }
                }
                Ok(v)
            };
            let offset = field(op & 0x0f, 4)?;
            let size = match field((op >> 4) & 0x07, 3)? {
                0 => 0x10000,
                n => n,
            };
            let end = offset.checked_add(size).filter(|&e| e <= base.len()).ok_or(BAD)?;
            if out.len() + size > target {
                return Err(BAD);
            }
            out.extend_from_slice(&base[offset..end]);
        } else if op != 0 {
            let n = usize::from(op);
            let lit = delta.get(p..p + n).ok_or(BAD)?;
            if out.len() + n > target {
                return Err(BAD);
            }
            out.extend_from_slice(lit);
            p += n;
        } else {
            return Err(BAD);
        }
    }
    if out.len() != target {
        return Err(BAD);
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::Write;

    use super::*;

    fn varint(mut n: u64, out: &mut Vec<u8>) {
        loop {
            let b = (n & 0x7f) as u8;
            n >>= 7;
            if n == 0 {
                out.push(b);
                return;
            }
            out.push(b | 0x80);
        }
    }

    /// A delta header: base and target sizes.
    fn delta_head(base: usize, target: usize) -> Vec<u8> {
        let mut d = Vec::new();
        varint(base as u64, &mut d);
        varint(target as u64, &mut d);
        d
    }

    pub(crate) enum Item<'a> {
        Whole(ObjectKind, &'a [u8]),
        /// A delta against the entry at this index, as `OFS_DELTA`.
        Ofs(usize, Vec<u8>),
        Ref(Oid, Vec<u8>),
    }

    fn zlib(data: &[u8]) -> Vec<u8> {
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(data).unwrap();
        z.finish().unwrap()
    }

    /// Builds a pack by hand: the bytes and the offset of every entry.
    pub(crate) fn build(items: &[Item<'_>]) -> Vec<u8> {
        let mut out = b"PACK".to_vec();
        out.extend_from_slice(&2u32.to_be_bytes());
        out.extend_from_slice(&u32::try_from(items.len()).unwrap().to_be_bytes());
        let mut offsets = Vec::new();
        for item in items {
            let offset = out.len() as u64;
            offsets.push(offset);
            let (ty, data): (u8, &[u8]) = match item {
                Item::Whole(k, d) => (
                    match k {
                        ObjectKind::Commit => 1,
                        ObjectKind::Tree => 2,
                        ObjectKind::Blob => 3,
                        ObjectKind::Tag => 4,
                    },
                    d,
                ),
                Item::Ofs(_, d) => (6, d),
                Item::Ref(_, d) => (7, d),
            };
            let mut size = data.len() as u64;
            let mut c = (ty << 4) | (size & 0x0f) as u8;
            size >>= 4;
            while size != 0 {
                out.push(c | 0x80);
                c = (size & 0x7f) as u8;
                size >>= 7;
            }
            out.push(c);
            match item {
                Item::Ofs(base, _) => {
                    let mut n = offset - offsets[*base];
                    let mut bytes = vec![(n & 0x7f) as u8];
                    n >>= 7;
                    while n != 0 {
                        n -= 1;
                        bytes.push(0x80 | (n & 0x7f) as u8);
                        n >>= 7;
                    }
                    bytes.reverse();
                    out.extend_from_slice(&bytes);
                }
                Item::Ref(oid, _) => out.extend_from_slice(&oid.0),
                Item::Whole(..) => {}
            }
            out.extend_from_slice(&zlib(data));
        }
        let sum = Sha1::digest(&out);
        out.extend_from_slice(&sum);
        out
    }

    fn open(bytes: &[u8]) -> Result<Pack, PackError> {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(bytes).unwrap();
        Pack::open(f.path())
    }

    /// A delta that keeps `base` and appends `tail`.
    fn append(base: &[u8], tail: &[u8]) -> Vec<u8> {
        let mut d = delta_head(base.len(), base.len() + tail.len());
        let mut off = 0;
        while off < base.len() {
            let n = (base.len() - off).min(0xffff);
            d.push(0x80 | 0x0f | 0x30);
            d.extend_from_slice(&u32::try_from(off).unwrap().to_le_bytes());
            d.extend_from_slice(&u16::try_from(n).unwrap().to_le_bytes());
            off += n;
        }
        for chunk in tail.chunks(127) {
            d.push(u8::try_from(chunk.len()).unwrap());
            d.extend_from_slice(chunk);
        }
        d
    }

    #[test]
    fn deltas_copy_and_insert() {
        let base = b"hello world, hello git";
        let mut d = delta_head(base.len(), 12);
        d.extend_from_slice(&[0x90, 5]); // copy offset 0, size 5
        d.extend_from_slice(&[2, b',', b' ']); // insert ", "
        d.extend_from_slice(&[0x91, 13, 5]); // copy offset 13, size 5
        assert_eq!(apply_delta(base, &d, MAX_OBJECT).unwrap(), b"hello, hello");

        // Size 0 means 0x10000.
        let big = vec![7u8; 0x10000];
        let mut d = delta_head(big.len(), big.len());
        d.push(0x80);
        assert_eq!(apply_delta(&big, &d, MAX_OBJECT).unwrap(), big);
        assert_eq!(apply_delta(base, &append(base, b"!"), MAX_OBJECT).unwrap(), b"hello world, hello git!");
    }

    #[test]
    fn hostile_deltas_are_refused_without_panicking() {
        let base = b"0123456789";
        let mut cases: Vec<Vec<u8>> = vec![
            vec![],
            vec![0x80],
            delta_head(9, 1),
            {
                let mut d = delta_head(10, 5);
                d.extend_from_slice(&[0x91, 8, 5]); // copy past the end of the base
                d
            },
            {
                let mut d = delta_head(10, 5);
                d.extend_from_slice(&[0x9f, 0xff, 0xff, 0xff, 0xff, 1]); // offset overflow
                d
            },
            {
                let mut d = delta_head(10, 2);
                d.extend_from_slice(&[3, 1, 2, 3]); // more than the target
                d
            },
            {
                let mut d = delta_head(10, 3);
                d.extend_from_slice(&[5, 1]); // truncated literal
                d
            },
            {
                let mut d = delta_head(10, 3);
                d.push(0); // reserved
                d
            },
            {
                let mut d = delta_head(10, 4);
                d.extend_from_slice(&[0x90, 3]); // shorter than the target
                d
            },
            vec![0xff; 12],
        ];
        for d in cases.drain(..) {
            assert!(apply_delta(base, &d, MAX_OBJECT).is_err(), "{d:?}");
        }
        assert_eq!(apply_delta(base, &delta_head(10, 1 << 40), MAX_OBJECT), Err(PackError::TooLarge));
    }

    #[test]
    fn packs_resolve_ofs_and_ref_deltas_in_any_order() {
        let base = b"line one\nline two\n".repeat(20);
        let v2 = [base.as_slice(), b"three\n"].concat();
        let v3 = [v2.as_slice(), b"four\n"].concat();
        let base_id = hash_object(ObjectKind::Blob, &base);
        let v2_id = hash_object(ObjectKind::Blob, &v2);
        let v3_id = hash_object(ObjectKind::Blob, &v3);
        // A REF_DELTA before its base (allowed), then an OFS_DELTA on a delta.
        let bytes = build(&[
            Item::Ref(base_id, append(&base, b"three\n")),
            Item::Whole(ObjectKind::Blob, &base),
            Item::Ofs(0, append(&v2, b"four\n")),
        ]);
        let mut pack = open(&bytes).unwrap();
        assert_eq!((pack.object_count(), pack.unresolved()), (3, 0));
        assert!(pack.missing_bases().is_empty());
        for (oid, want) in [(base_id, &base), (v2_id, &v2), (v3_id, &v3)] {
            assert!(pack.has(&oid));
            assert_eq!(pack.header(&oid), Some((ObjectKind::Blob, want.len() as u64)));
            assert_eq!(&*pack.read(&oid).unwrap().unwrap().1, want.as_slice());
        }
        assert_eq!(pack.read(&Oid([9; 20])).unwrap(), None);
    }

    #[test]
    fn thin_packs_take_checked_bases() {
        let base = b"server side content\n".repeat(10);
        let base_id = hash_object(ObjectKind::Blob, &base);
        let new = [base.as_slice(), b"more\n"].concat();
        let bytes = build(&[Item::Ref(base_id, append(&base, b"more\n")), Item::Ofs(0, append(&new, b"x"))]);
        let mut pack = open(&bytes).unwrap();
        assert_eq!(pack.unresolved(), 2);
        assert_eq!(pack.missing_bases(), vec![base_id]);
        assert!(pack.add_base(base_id, ObjectKind::Blob, b"not it".to_vec()).is_err());
        assert!(pack.add_base(base_id, ObjectKind::Tree, base.clone()).is_err());
        pack.add_base(base_id, ObjectKind::Blob, base).unwrap();
        assert_eq!(pack.unresolved(), 0);
        assert!(!pack.has(&base_id));
        let new_id = hash_object(ObjectKind::Blob, &new);
        assert!(pack.has(&new_id));
        assert_eq!(&*pack.read(&new_id).unwrap().unwrap().1, new.as_slice());
        let newer = [new.as_slice(), b"x"].concat();
        assert!(pack.has(&hash_object(ObjectKind::Blob, &newer)));
    }

    #[test]
    fn damaged_packs_are_refused() {
        let good = build(&[Item::Whole(ObjectKind::Blob, b"hello\n"), Item::Whole(ObjectKind::Blob, b"world\n")]);
        open(&good).unwrap();
        let mut bad_sum = good.clone();
        *bad_sum.last_mut().unwrap() ^= 1;
        let mut bad_magic = good.clone();
        bad_magic[0] = b'X';
        let mut bad_version = good.clone();
        bad_version[7] = 9;
        let mut more_count = good.clone();
        more_count[11] = 3;
        let mut garbage = good[..good.len() - 20].to_vec();
        garbage.push(0);
        let sum = Sha1::digest(&garbage);
        garbage.extend_from_slice(&sum);
        let mut bad_type = good.clone();
        bad_type[12] = (bad_type[12] & 0x8f) | 0x50;
        for (bytes, what) in [
            (&good[..good.len() - 1], "truncated trailer"),
            (&good[..30], "truncated object"),
            (&good[..10], "tiny"),
            (&bad_sum[..], "checksum"),
            (&bad_magic[..], "magic"),
            (&bad_version[..], "version"),
            (&more_count[..], "count"),
            (&garbage[..], "garbage"),
            (&bad_type[..], "type"),
        ] {
            assert!(open(bytes).is_err(), "{what}");
        }
        // A zlib stream that inflates to more than declared.
        let mut lying = build(&[Item::Whole(ObjectKind::Blob, b"hello\n")]);
        lying[12] = (lying[12] & 0xf0) | 2;
        let body = lying[..lying.len() - 20].to_vec();
        let mut lying = body.clone();
        lying.extend_from_slice(&Sha1::digest(&body));
        assert_eq!(open(&lying).unwrap_err(), PackError::Corrupt("an object is larger than declared"));
        // An OFS_DELTA pointing at no object.
        let base = b"0123456789".to_vec();
        let mut ofs = build(&[Item::Whole(ObjectKind::Blob, &base), Item::Ofs(0, append(&base, b"x"))]);
        let at = 12 + 1 + zlib(&base).len();
        assert_eq!(ofs[at] >> 4, 6);
        ofs[at + 1] = 1;
        let body = ofs[..ofs.len() - 20].to_vec();
        let mut ofs = body.clone();
        ofs.extend_from_slice(&Sha1::digest(&body));
        assert!(open(&ofs).is_err());
    }

    #[test]
    fn deltas_that_do_not_apply_are_damage_not_failure() {
        let base = b"0123456789".to_vec();
        let mut bad = delta_head(10, 5);
        bad.extend_from_slice(&[0x91, 8, 5]);
        let mut pack = open(&build(&[Item::Whole(ObjectKind::Blob, &base), Item::Ofs(0, bad)])).unwrap();
        assert!(pack.damage().is_some());
        assert_eq!(pack.unresolved(), 1);
        assert!(pack.read(&hash_object(ObjectKind::Blob, &base)).unwrap().is_some());
    }

    #[test]
    fn lru_cache_stays_bounded() {
        let mut c = Cache::default();
        let chunk: Arc<[u8]> = vec![0u8; usize::try_from(CACHE_BYTES / 4).unwrap()].into();
        for i in 0..10 {
            c.put(i, &chunk);
        }
        assert!(c.bytes <= CACHE_BYTES);
        assert!(c.get(0).is_none());
        assert!(c.get(9).is_some());
        let huge: Arc<[u8]> = vec![0u8; usize::try_from(CACHE_BYTES / 2 + 1).unwrap()].into();
        c.put(100, &huge);
        assert!(c.get(100).is_none());
    }
}
