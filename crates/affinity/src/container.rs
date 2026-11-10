//! The Affinity archive: header, the chain of file allocation tables (one per save) and
//! bounded, CRC-checked extraction of named entries such as `doc.dat` and raster tiles.

use std::collections::{HashMap, HashSet};
use std::io::Read;

use crate::Error;

pub const MAGIC: &[u8; 4] = b"\x00\xffKA";
/// Oldest and newest container versions seen in real documents (Affinity 1.x writes 8,
/// Affinity 3 writes 12). Version 7 is described by afread but no sample exists.
pub const VERSIONS: std::ops::RangeInclusive<u16> = 8..=12;
/// `Prsn` (stored byte-reversed): documents. Asset, brush and style libraries share the magic.
const DOCUMENT_KIND: &[u8; 4] = b"nsrP";

/// A save appends one table; real files have one to a few hundred.
const MAX_TABLES: usize = 4096;
/// Distinct ids the allocation tables may name; each costs a map slot even when only deleted.
const MAX_ENTRIES: usize = 1 << 22;
/// Bytes of entry names the allocation tables may name. A name is retained twice (in the entry
/// and as its map key), and tables can point many names at the same bytes.
const MAX_NAME_BYTES: usize = 32 << 20;
/// File and directory records parsed across every allocation table. Each table walks its own
/// records, and tables may start inside one another and share a long suffix, so the per-table
/// counts do not bound the work: a few thousand overlapping tables could reparse one large run
/// of records each. Real files hold a few hundred saves of a few thousand entries.
const MAX_TABLE_RECORDS: usize = 1 << 25;
/// zstd window ceiling: real documents use a few MiB; this stops a header from reserving gigabytes.
const MAX_ZSTD_WINDOW: u64 = 64 << 20;

/// Decompression budget for one import. Every entry declares its size, which is checked against
/// these limits before anything is allocated, and the decoded length must match it exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Largest single entry (`doc.dat` or one tile).
    pub max_entry: usize,
    /// Total bytes extracted over the whole import.
    pub max_total: usize,
    /// Bytes of decoded pixels one import may hold: every layer's RGBA together, plus the
    /// temporary planes the layer being read decodes through. The entry limits bound what is
    /// extracted, not what the layers materialized out of it cost (#4960).
    pub max_pixel_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        // The gigabyte the other importers allow decoded pixels (`io`'s `MAX_PIXEL_BYTES`).
        Self { max_entry: 256 << 20, max_total: 1 << 30, max_pixel_bytes: 1 << 30 }
    }
}

/// The container header, all integers little-endian.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub version: u16,
    /// Seen as 0, 4 or 8 in real files; bits 0 and 1 mark container variants that are rejected.
    pub flags: u16,
    pub(crate) fat_offset: u64,
    pub(crate) thumbnail_offset: u64,
    /// Save time in Unix seconds, as written by the application.
    pub saved: u32,
}

/// One FAT record, as of the newest table that mentions its id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub id: u32,
    pub name: String,
    offset: u64,
    size: u64,
    compressed: u64,
    crc: u32,
    compression: u8,
}

impl Entry {
    /// Decompressed size declared by the table.
    pub fn size(&self) -> u64 {
        self.size
    }
}

#[derive(Debug)]
pub struct Archive<'a> {
    bytes: &'a [u8],
    pub header: Header,
    /// Head revision of every named, non-deleted entry.
    entries: HashMap<String, Entry>,
    /// Number of tables (saves) in the chain.
    pub revisions: usize,
    extracted: usize,
    limits: Limits,
}

pub(crate) struct Cursor<'a> {
    bytes: &'a [u8],
    pub(crate) pos: usize,
}

impl<'a> Cursor<'a> {
    pub(crate) fn new(bytes: &'a [u8], pos: usize) -> Self {
        Self { bytes, pos }
    }
    pub(crate) fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.pos)
    }
    pub(crate) fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self.pos.checked_add(n).ok_or(Error::Malformed("length overflow"))?;
        let s = self.bytes.get(self.pos..end).ok_or(Error::Malformed("truncated data"))?;
        self.pos = end;
        Ok(s)
    }
    pub(crate) fn array<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        let s = self.take(N)?;
        let mut a = [0u8; N];
        a.copy_from_slice(s);
        Ok(a)
    }
    pub(crate) fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.array::<1>()?[0])
    }
    pub(crate) fn u16(&mut self) -> Result<u16, Error> {
        Ok(u16::from_le_bytes(self.array()?))
    }
    pub(crate) fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_le_bytes(self.array()?))
    }
    pub(crate) fn u64(&mut self) -> Result<u64, Error> {
        Ok(u64::from_le_bytes(self.array()?))
    }
}

fn offset(v: u64) -> Result<usize, Error> {
    usize::try_from(v).map_err(|_| Error::Malformed("offset beyond addressable memory"))
}

/// True when `bytes` start with the Affinity container signature (documents and libraries).
pub fn is_affinity(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
}

/// Check the signature, version and document kind.
pub fn header(bytes: &[u8]) -> Result<Header, Error> {
    if !is_affinity(bytes) {
        return Err(Error::Malformed("missing container signature"));
    }
    let mut c = Cursor::new(bytes, 4);
    let version = c.u16()?;
    let flags = c.u16()?;
    if !VERSIONS.contains(&version) {
        return Err(Error::Unsupported("unknown container version"));
    }
    if flags & 3 != 0 {
        return Err(Error::Unsupported("container variant"));
    }
    if c.take(4)? != DOCUMENT_KIND {
        return Err(Error::Unsupported("this Affinity file is a library (assets, brushes or styles), not a document"));
    }
    if c.take(4)? != b"#Inf" {
        return Err(Error::Malformed("missing information header"));
    }
    let fat_offset = c.u64()?;
    let thumbnail_offset = c.u64()?;
    c.take(16)?;
    let saved = u32::try_from(c.u64()?).unwrap_or(0);
    c.take(8)?;
    if c.take(4)? != b"Prot" {
        return Err(Error::Malformed("missing protocol header"));
    }
    Ok(Header { version, flags, fat_offset, thumbnail_offset, saved })
}

impl<'a> Archive<'a> {
    pub fn open(bytes: &'a [u8], limits: Limits) -> Result<Self, Error> {
        let header = header(bytes)?;
        // Newest record per id: (table date, table index, record).
        let mut latest: HashMap<u32, (u64, usize, Option<Box<Entry>>)> = HashMap::new();
        let mut names: HashMap<u32, String> = HashMap::new();
        let mut name_bytes = 0usize;
        let mut seen = HashSet::new();
        let mut next = header.fat_offset;
        let mut tables = 0usize;
        let mut records_left = MAX_TABLE_RECORDS;
        while next != 0 {
            if !seen.insert(next) {
                return Err(Error::Malformed("allocation tables form a cycle"));
            }
            tables += 1;
            if tables > MAX_TABLES {
                return Err(Error::Limit("too many saved revisions"));
            }
            let mut c = Cursor::new(bytes, offset(next)?);
            let tag = c.array::<4>()?;
            let level = match &tag {
                b"#FAT" => 1,
                b"#FT2" => 2,
                b"#FT3" => 3,
                b"#FT4" => 4,
                _ => return Err(Error::Malformed("missing allocation table")),
            };
            next = c.u64()?;
            let date = c.u64()?;
            c.take(24)?;
            let files = c.u32()?;
            c.take(8)?;
            let dirs = c.u16()?;
            c.take(1)?;
            // Charge the declared records before walking them.
            let declared = usize::try_from(files).unwrap_or(usize::MAX).saturating_add(usize::from(dirs));
            records_left = records_left.checked_sub(declared).ok_or(Error::Limit("too many allocation records"))?;
            for _ in 0..files {
                let id = c.u32()?;
                let flag = c.u8()?;
                let mut record = None;
                match flag {
                    0 | 1 => {
                        let offset = c.u64()?;
                        let size = c.u64()?;
                        let compressed = c.u64()?;
                        let crc = c.u32()?;
                        let stored = c.u8()?;
                        if level >= 2 {
                            c.take(4)?;
                        }
                        if level >= 4 {
                            c.take(4)?;
                        }
                        // Old tables number the methods; newer ones store the method byte itself.
                        let compression = if level <= 2 {
                            match stored {
                                1 => 0x01,
                                2 => 0x41,
                                3 => 0x81,
                                4 => 0xC1,
                                _ => 0,
                            }
                        } else {
                            stored
                        };
                        record = Some(Box::new(Entry { id, name: String::new(), offset, size, compressed, crc, compression }));
                    }
                    2 => {}
                    _ => return Err(Error::Malformed("unknown allocation record")),
                }
                if flag == 0 {
                    let len = usize::from(c.u16()?);
                    let raw = c.take(len)?;
                    // Only the first name per id is kept: charge and copy just that one.
                    if let std::collections::hash_map::Entry::Vacant(slot) = names.entry(id) {
                        let name = String::from_utf8_lossy(raw).into_owned();
                        name_bytes = name_bytes.saturating_add(name.len());
                        if name_bytes > MAX_NAME_BYTES {
                            return Err(Error::Limit("archive entry names too large"));
                        }
                        slot.insert(name);
                    }
                }
                // Equal dates: the first table in the chain (the newest) wins, as in afread.
                let replace = latest.get(&id).is_none_or(|(d, _, _)| date > *d);
                if replace {
                    // Deletion records are five bytes each: cap what distinct ids may retain.
                    if latest.len() >= MAX_ENTRIES && !latest.contains_key(&id) {
                        return Err(Error::Limit("too many archive entries"));
                    }
                    latest.insert(id, (date, tables, record));
                }
            }
            for _ in 0..dirs {
                let len = usize::from(c.u16()?);
                c.take(10)?;
                c.take(len)?;
            }
        }
        let mut entries = HashMap::new();
        for (id, (_, _, record)) in latest {
            if let (Some(mut e), Some(name)) = (record, names.get(&id)) {
                e.name = name.clone();
                entries.insert(e.name.clone(), *e);
            }
        }
        Ok(Self { bytes, header, entries, revisions: tables, extracted: 0, limits })
    }

    pub fn entry(&self, name: &str) -> Option<&Entry> {
        self.entries.get(name)
    }

    pub fn entry_names(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    /// Bytes extracted so far, counted against [`Limits::max_total`].
    pub fn extracted(&self) -> usize {
        self.extracted
    }

    /// Extract an entry by name: decompress, undo the byte predictor and verify the CRC.
    pub fn read(&mut self, name: &str) -> Result<Vec<u8>, Error> {
        self.claim(name)?.decode()
    }

    /// Extract several entries, decoding them on worker threads. The entries are charged one
    /// after the other exactly as the same number of [`read`](Self::read) calls would charge
    /// them, and the first error in that order is the one returned.
    pub(crate) fn read_many(&mut self, names: &[&str]) -> Result<Vec<Vec<u8>>, Error> {
        let mut claimed = Vec::with_capacity(names.len());
        let mut refused = None;
        for name in names {
            match self.claim(name) {
                Ok(c) => claimed.push(c),
                Err(e) => {
                    refused = Some(e);
                    break;
                }
            }
        }
        let decoded = decode_all(&claimed).into_iter().collect::<Result<Vec<_>, _>>()?;
        refused.map_or(Ok(decoded), Err)
    }

    /// Check an entry against the limits, count it as extracted and locate its stored bytes.
    fn claim(&mut self, name: &str) -> Result<Claimed<'a>, Error> {
        let e = self.entries.get(name).ok_or(Error::Malformed("missing archive entry"))?;
        let size = usize::try_from(e.size).map_err(|_| Error::Limit("archive entry too large"))?;
        if size > self.limits.max_entry {
            return Err(Error::Limit("archive entry too large"));
        }
        let total = self.extracted.checked_add(size).ok_or(Error::Limit("document too large"))?;
        if total > self.limits.max_total {
            return Err(Error::Limit("document too large"));
        }
        // Charge before decoding: a corrupt entry that fails its checksum has already cost the
        // decompression, and an importer that recovers must not get it again for free.
        self.extracted = total;
        let mut c = Cursor::new(self.bytes, offset(e.offset)?);
        if c.take(4)? != b"#Fil" {
            return Err(Error::Malformed("archive entry has no data block"));
        }
        let method = e.compression & 3;
        let stored_len = if matches!(method, 1 | 2) { e.compressed } else { e.size };
        let data = c.take(offset(stored_len)?)?;
        Ok(Claimed { data, size, compression: e.compression, crc: e.crc })
    }
}

/// An entry that is charged and located but not yet decoded.
struct Claimed<'a> {
    data: &'a [u8],
    size: usize,
    compression: u8,
    crc: u32,
}

impl Claimed<'_> {
    fn decode(&self) -> Result<Vec<u8>, Error> {
        let mut out = match self.compression & 3 {
            1 => inflate(self.data, self.size)?,
            2 => unzstd(self.data, self.size)?,
            _ => self.data.to_vec(),
        };
        if out.len() != self.size {
            return Err(Error::Malformed("archive entry size mismatch"));
        }
        unpredict(&mut out, self.compression)?;
        if crc32fast::hash(&out) != self.crc {
            return Err(Error::Malformed("archive entry checksum mismatch"));
        }
        Ok(out)
    }
}

/// Most threads decoding entries at once; each zstd frame may reserve its declared window.
const MAX_DECODE_THREADS: usize = 8;
/// Fewer entries than this are decoded in place: starting a thread costs more than they do.
const MIN_PARALLEL_ENTRIES: usize = 4;

/// Decode every entry, in parallel when there are enough of them. The results are in order.
fn decode_all(jobs: &[Claimed<'_>]) -> Vec<Result<Vec<u8>, Error>> {
    static THREADS: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    // The browser build has no threads to start.
    let threads = if jobs.len() < MIN_PARALLEL_ENTRIES || cfg!(target_arch = "wasm32") {
        1
    } else {
        (*THREADS.get_or_init(|| std::thread::available_parallelism().map_or(1, usize::from).min(MAX_DECODE_THREADS))).min(jobs.len())
    };
    if threads <= 1 {
        return jobs.iter().map(Claimed::decode).collect();
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    // The workers and this thread take the next undecoded entry until none is left.
    let work = || {
        let mut done = Vec::new();
        loop {
            let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let Some(job) = jobs.get(i) else { break };
            done.push((i, job.decode()));
        }
        done
    };
    let mut slots: Vec<Option<Result<Vec<u8>, Error>>> = jobs.iter().map(|_| None).collect();
    std::thread::scope(|sc| {
        // A thread that cannot be started just leaves its share to the others.
        let workers: Vec<_> = (1..threads).filter_map(|_| std::thread::Builder::new().spawn_scoped(sc, work).ok()).collect();
        let mut finished = vec![work()];
        finished.extend(workers.into_iter().map(|w| w.join().unwrap_or_default()));
        for (i, result) in finished.into_iter().flatten() {
            if let Some(slot) = slots.get_mut(i) {
                *slot = Some(result);
            }
        }
    });
    slots.into_iter().map(|r| r.unwrap_or(Err(Error::Malformed("tile decoder stopped")))).collect()
}

/// Read at most `size + 1` bytes so a stream that decodes past its declared size is caught
/// without unbounded allocation.
fn read_bounded(mut r: impl Read, size: usize) -> Result<Vec<u8>, Error> {
    let mut out = Vec::with_capacity(size.min(64 << 20));
    let cap = u64::try_from(size).unwrap_or(u64::MAX).saturating_add(1);
    r.by_ref().take(cap).read_to_end(&mut out).map_err(|_| Error::Malformed("corrupt compressed entry"))?;
    Ok(out)
}

fn inflate(data: &[u8], size: usize) -> Result<Vec<u8>, Error> {
    read_bounded(flate2::bufread::ZlibDecoder::new(data), size)
}

fn unzstd(data: &[u8], size: usize) -> Result<Vec<u8>, Error> {
    let mut src = data;
    let mut out = Vec::new();
    // Entries may hold several concatenated frames.
    while !src.is_empty() {
        let left = size.checked_sub(out.len()).ok_or(Error::Malformed("archive entry size mismatch"))?;
        let decoder = ruzstd::decoding::StreamingDecoder::new_with_max_window_size(&mut src, MAX_ZSTD_WINDOW)
            .map_err(|_| Error::Malformed("corrupt compressed entry"))?;
        let frame = read_bounded(decoder, left)?;
        if frame.is_empty() {
            return Err(Error::Malformed("corrupt compressed entry"));
        }
        out.extend_from_slice(&frame);
        if out.len() > size {
            return Err(Error::Malformed("archive entry size mismatch"));
        }
    }
    Ok(out)
}

/// Undo the byte predictor selected by the high bits of the compression byte.
fn unpredict(b: &mut [u8], compression: u8) -> Result<(), Error> {
    match (compression & 0xC0, compression & 0x20 != 0) {
        (0x00, _) => {}
        (0x40, _) => byte_delta(b),
        (0x80, false) => {
            let mut prev = 0u16;
            for pair in b.as_chunks_mut::<2>().0.iter_mut() {
                let v = u16::from_le_bytes(*pair).wrapping_add(prev);
                *pair = v.to_le_bytes();
                prev = v;
            }
        }
        (0x80, true) => {
            byte_delta(b);
            // A 64 KiB tile of 16-bit samples is stored as all high bytes, then all low bytes.
            if b.len() == 0x10000 {
                let planes = b.to_vec();
                let (hi, lo) = planes.split_at(0x8000);
                for (i, out) in b.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    let (Some(h), Some(l)) = (hi.get(2 * i..2 * i + 2), lo.get(2 * i..2 * i + 2)) else {
                        return Err(Error::Malformed("tile size"));
                    };
                    *out = [l[0], h[0], l[1], h[1]];
                }
            }
        }
        _ => return Err(Error::Unsupported("archive entry predictor")),
    }
    Ok(())
}

fn byte_delta(b: &mut [u8]) {
    let mut prev = 0u8;
    for v in b {
        *v = v.wrapping_add(prev);
        prev = *v;
    }
}
