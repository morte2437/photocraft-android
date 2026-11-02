//! Minimal ZIP container: we write STORE entries (the payloads are already
//! zstd-compressed) and read STORE or DEFLATE entries (so a bundle that was
//! unzipped and re-zipped by another tool still loads). No ZIP64, no
//! encryption, no multi-disk archives.

use std::borrow::Cow;
use std::collections::HashMap;
use std::io::Read;

use crate::FormatError;

const LOCAL_SIG: u32 = 0x0403_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const EOCD_SIG: u32 = 0x0605_4b50;

/// CRC-32 (IEEE), as stored in ZIP headers.
pub fn crc32(data: &[u8]) -> u32 {
    crc32fast::hash(data)
}

/// Streaming writer for STORE entries.
pub struct ZipWriter {
    out: Vec<u8>,
    central: Vec<u8>,
    count: u16,
}

impl Default for ZipWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl ZipWriter {
    /// Creates an empty archive of STORE entries.
    pub fn new() -> Self {
        ZipWriter { out: Vec::new(), central: Vec::new(), count: 0 }
    }

    /// Reserves the complete archive from its entry names and payload sizes.
    pub(crate) fn with_entries<'a>(entries: impl IntoIterator<Item = (&'a str, usize)>) -> Result<Self, FormatError> {
        let (size, directory) = entries.into_iter().try_fold((22usize, 0usize), |(size, directory), (name, bytes)| {
            let directory = directory.checked_add(46 + name.len());
            let size = size.checked_add(76).and_then(|n| n.checked_add(name.len())).and_then(|n| n.checked_add(name.len())).and_then(|n| n.checked_add(bytes));
            size.zip(directory).ok_or_else(|| FormatError::Unsupported("zip size overflow".into()))
        })?;
        let mut writer = Self::new();
        writer.out.try_reserve_exact(size).map_err(|_| FormatError::LimitExceeded("not enough memory for zip".into()))?;
        writer.central.try_reserve_exact(directory).map_err(|_| FormatError::LimitExceeded("not enough memory for zip directory".into()))?;
        Ok(writer)
    }

    /// Adds a STORE entry, rejecting names or data that exceed ZIP field limits.
    /// The entry path has a 16-bit byte length, independent of filesystem limits.
    pub fn add(&mut self, name: &str, data: &[u8]) -> Result<(), FormatError> {
        if self.count == u16::MAX {
            return Err(FormatError::Unsupported("more than 65535 zip entries (ZIP64 not supported)".into()));
        }
        let size = u32::try_from(data.len()).map_err(|_| FormatError::Unsupported("zip entry over 4 GiB".into()))?;
        let offset = u32::try_from(self.out.len()).map_err(|_| FormatError::Unsupported("zip over 4 GiB".into()))?;
        let name_b = name.as_bytes();
        let name_len = u16::try_from(name_b.len()).map_err(|_| FormatError::Unsupported("zip entry path exceeds the 65535-byte format limit".into()))?;
        let local_len = 30usize
            .checked_add(name_b.len())
            .and_then(|n| n.checked_add(data.len()))
            .ok_or_else(|| FormatError::Unsupported("zip entry size overflow".into()))?;
        self.out.try_reserve(local_len).map_err(|_| FormatError::LimitExceeded("not enough memory for zip entry".into()))?;
        self.central.try_reserve(46 + name_b.len()).map_err(|_| FormatError::LimitExceeded("not enough memory for zip directory".into()))?;
        let crc = crc32(data);
        // Local file header.
        let o = &mut self.out;
        o.extend_from_slice(&LOCAL_SIG.to_le_bytes());
        o.extend_from_slice(&20u16.to_le_bytes()); // version needed
        o.extend_from_slice(&0x0800u16.to_le_bytes()); // flags: UTF-8 names
        o.extend_from_slice(&0u16.to_le_bytes()); // method: store
        o.extend_from_slice(&0u16.to_le_bytes()); // time
        o.extend_from_slice(&0x21u16.to_le_bytes()); // date: 1980-01-01
        o.extend_from_slice(&crc.to_le_bytes());
        o.extend_from_slice(&size.to_le_bytes());
        o.extend_from_slice(&size.to_le_bytes());
        o.extend_from_slice(&name_len.to_le_bytes());
        o.extend_from_slice(&0u16.to_le_bytes());
        o.extend_from_slice(name_b);
        o.extend_from_slice(data);
        // Central directory entry.
        let c = &mut self.central;
        c.extend_from_slice(&CENTRAL_SIG.to_le_bytes());
        c.extend_from_slice(&20u16.to_le_bytes()); // made by
        c.extend_from_slice(&20u16.to_le_bytes()); // needed
        c.extend_from_slice(&0x0800u16.to_le_bytes());
        c.extend_from_slice(&0u16.to_le_bytes());
        c.extend_from_slice(&0u16.to_le_bytes());
        c.extend_from_slice(&0x21u16.to_le_bytes());
        c.extend_from_slice(&crc.to_le_bytes());
        c.extend_from_slice(&size.to_le_bytes());
        c.extend_from_slice(&size.to_le_bytes());
        c.extend_from_slice(&name_len.to_le_bytes());
        c.extend_from_slice(&[0; 8]); // extra len, comment len, disk, internal attrs
        c.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        c.extend_from_slice(&offset.to_le_bytes());
        c.extend_from_slice(name_b);
        self.count += 1;
        Ok(())
    }

    /// Appends the central directory and returns the complete archive.
    pub fn finish(mut self) -> Result<Vec<u8>, FormatError> {
        let cd_offset = u32::try_from(self.out.len()).map_err(|_| FormatError::Unsupported("zip over 4 GiB".into()))?;
        let cd_size = u32::try_from(self.central.len()).map_err(|_| FormatError::Unsupported("zip directory over 4 GiB".into()))?;
        self.out.try_reserve(self.central.len().saturating_add(22)).map_err(|_| FormatError::LimitExceeded("not enough memory to finish zip".into()))?;
        self.out.extend_from_slice(&self.central);
        self.out.extend_from_slice(&EOCD_SIG.to_le_bytes());
        self.out.extend_from_slice(&[0; 4]); // disk numbers
        self.out.extend_from_slice(&self.count.to_le_bytes());
        self.out.extend_from_slice(&self.count.to_le_bytes());
        self.out.extend_from_slice(&cd_size.to_le_bytes());
        self.out.extend_from_slice(&cd_offset.to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes());
        Ok(self.out)
    }
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    method: u16,
    crc: u32,
    compressed: usize,
    uncompressed: usize,
    data_offset: usize,
}

/// Read-only view of an archive in memory.
pub struct ZipReader<'a> {
    bytes: &'a [u8],
    /// All entries, in central-directory order. Treat as read-only: `find`
    /// looks names up through an index built from this list in `new`.
    pub entries: Vec<Entry>,
    /// Name -> index of the first entry with that name, so `find` is O(1)
    /// instead of a scan per lookup (a bundle reads one entry per tile).
    by_name: HashMap<Cow<'a, str>, usize>,
}

fn u16_at(b: &[u8], o: usize) -> Result<u16, FormatError> {
    o.checked_add(2).and_then(|end| b.get(o..end)).map(|s| u16::from_le_bytes([s[0], s[1]])).ok_or_else(|| FormatError::corrupt("truncated zip"))
}
fn u32_at(b: &[u8], o: usize) -> Result<u32, FormatError> {
    o.checked_add(4).and_then(|end| b.get(o..end)).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).ok_or_else(|| FormatError::corrupt("truncated zip"))
}

impl<'a> ZipReader<'a> {
    /// Checks ZIP record bounds and indexes the first entry for each name.
    pub fn new(bytes: &'a [u8]) -> Result<Self, FormatError> {
        if bytes.len() < 22 {
            return Err(FormatError::corrupt("not a zip archive (too short)"));
        }
        // A comment may itself contain the signature. Only accept a record whose
        // declared comment reaches the end of the archive (APPNOTE 4.3.16).
        let min = bytes.len().saturating_sub(22 + 65535);
        let eocd = (min..=bytes.len() - 22)
            .rev()
            .find(|&i| {
                if u32_at(bytes, i).ok() != Some(EOCD_SIG) {
                    return false;
                }
                let comment_end = u16_at(bytes, i + 20).ok().and_then(|length| i.checked_add(22)?.checked_add(usize::from(length)));
                let directory_end = u32_at(bytes, i + 16)
                    .ok()
                    .zip(u32_at(bytes, i + 12).ok())
                    .and_then(|(offset, size)| usize::try_from(offset).ok()?.checked_add(usize::try_from(size).ok()?));
                // In the supported non-ZIP64 layout, the central directory immediately
                // precedes this record. A zeroed signature in a comment is not an empty ZIP.
                comment_end == Some(bytes.len()) && directory_end == Some(i)
            })
            .ok_or_else(|| FormatError::corrupt("zip end-of-central-directory not found"))?;
        let count = u16_at(bytes, eocd + 10)? as usize;
        let cd_size = u32_at(bytes, eocd + 12)? as usize;
        let cd_off = u32_at(bytes, eocd + 16)? as usize;
        let cd_end = cd_off.checked_add(cd_size).filter(|end| *end <= eocd).ok_or_else(|| FormatError::corrupt("zip central directory out of range"))?;
        let mut entries = Vec::with_capacity(count.min(cd_size / 46));
        let mut by_name = HashMap::with_capacity(entries.capacity());
        let mut p = cd_off;
        for _ in 0..count {
            let header_end = p.checked_add(46).filter(|end| *end <= cd_end).ok_or_else(|| FormatError::corrupt("truncated zip central directory entry"))?;
            if u32_at(bytes, p)? != CENTRAL_SIG {
                return Err(FormatError::corrupt("bad zip central directory entry"));
            }
            let flags = u16_at(bytes, p + 8)?;
            let method = u16_at(bytes, p + 10)?;
            let crc = u32_at(bytes, p + 16)?;
            let compressed = u32_at(bytes, p + 20)? as usize;
            let uncompressed = u32_at(bytes, p + 24)? as usize;
            let nlen = u16_at(bytes, p + 28)? as usize;
            let elen = u16_at(bytes, p + 30)? as usize;
            let clen = u16_at(bytes, p + 32)? as usize;
            let local = u32_at(bytes, p + 42)? as usize;
            let name_end = header_end.checked_add(nlen).filter(|end| *end <= cd_end).ok_or_else(|| FormatError::corrupt("zip entry name out of range"))?;
            let name = bytes.get(header_end..name_end).ok_or_else(|| FormatError::corrupt("truncated zip name"))?;
            let name = String::from_utf8_lossy(name);
            if flags & 1 != 0 {
                return Err(FormatError::Unsupported("encrypted zip entries".into()));
            }
            // Bound each entry by the declared central-directory extent before advancing.
            let entry_end = name_end
                .checked_add(elen)
                .and_then(|end| end.checked_add(clen))
                .filter(|end| *end <= cd_end)
                .ok_or_else(|| FormatError::corrupt("zip central directory entry out of range"))?;
            // Local header gives the real data offset.
            if u32_at(bytes, local)? != LOCAL_SIG {
                return Err(FormatError::corrupt("bad zip local header"));
            }
            let local_name_len = local.checked_add(26).ok_or_else(|| FormatError::corrupt("zip local header offset overflow"))?;
            let local_extra_len = local.checked_add(28).ok_or_else(|| FormatError::corrupt("zip local header offset overflow"))?;
            let lnlen = u16_at(bytes, local_name_len)? as usize;
            let lelen = u16_at(bytes, local_extra_len)? as usize;
            let data_offset = local
                .checked_add(30)
                .and_then(|offset| offset.checked_add(lnlen))
                .and_then(|offset| offset.checked_add(lelen))
                .ok_or_else(|| FormatError::corrupt("zip data offset overflow"))?;
            if data_offset.checked_add(compressed).is_none_or(|end| end > bytes.len()) {
                return Err(FormatError::corrupt(format!("zip entry `{name}` out of range")));
            }
            // The index borrows valid UTF-8 names; duplicate names keep the first entry.
            by_name.entry(name.clone()).or_insert(entries.len());
            entries.push(Entry { name: name.into_owned(), method, crc, compressed, uncompressed, data_offset });
            p = entry_end;
        }
        Ok(ZipReader { bytes, entries, by_name })
    }

    pub fn find(&self, name: &str) -> Option<&Entry> {
        self.by_name.get(name).and_then(|&i| self.entries.get(i))
    }

    /// Entry payload (decompressed), at most `max` bytes, CRC-checked.
    pub fn read(&self, e: &Entry, max: usize) -> Result<Vec<u8>, FormatError> {
        if e.uncompressed > max {
            return Err(FormatError::LimitExceeded(format!("zip entry `{}` is {} bytes (max {max})", e.name, e.uncompressed)));
        }
        // `Entry` is public and cloneable: one from another archive must not index out of range.
        let raw = e
            .data_offset
            .checked_add(e.compressed)
            .and_then(|end| self.bytes.get(e.data_offset..end))
            .ok_or_else(|| FormatError::corrupt(format!("zip entry `{}` out of range", e.name)))?;
        let data = match e.method {
            0 => raw.to_vec(),
            8 => {
                // The uncompressed length comes from an untrusted ZIP header: a
                // small forged archive must not reserve gigabytes up front, so the
                // preallocation is capped (DEFLATE expands at most ~1032x).
                let mut out = Vec::with_capacity(e.uncompressed.min(raw.len().saturating_mul(1032)).min(64 << 20));
                // One surplus byte proves a false size without expanding to the caller's larger budget.
                flate2::bufread::DeflateDecoder::new(raw)
                    .take((e.uncompressed as u64).saturating_add(1))
                    .read_to_end(&mut out)
                    .map_err(|err| FormatError::corrupt(format!("zip entry `{}`: {err}", e.name)))?;
                out
            }
            m => {
                return Err(FormatError::Unsupported(format!("zip compression method {m} for `{}`", e.name)));
            }
        };
        if data.len() != e.uncompressed || data.len() > max {
            return Err(FormatError::corrupt(format!("zip entry `{}` has wrong size", e.name)));
        }
        if crc32(&data) != e.crc {
            return Err(FormatError::corrupt(format!("zip entry `{}` failed its CRC check", e.name)));
        }
        Ok(data)
    }

    pub fn read_by_name(&self, name: &str, max: usize) -> Result<Vec<u8>, FormatError> {
        let e = self.find(name).ok_or_else(|| FormatError::corrupt(format!("missing `{name}`")))?;
        self.read(e, max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_known_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    /// ZIP entry paths have a format limit even when each component is short.
    #[test]
    fn entry_path_length_is_checked_before_writing() {
        let mut writer = ZipWriter::new();
        for name in ["dir/".repeat(16384), "é/".repeat(21846)] {
            assert!(matches!(writer.add(&name, b"invalid"), Err(FormatError::Unsupported(_))));
            assert!(writer.out.is_empty() && writer.central.is_empty());
        }
        let name = "dir/".repeat(16383) + "end";
        writer.add(&name, b"valid").unwrap();
        let bytes = writer.finish().unwrap();
        assert_eq!(ZipReader::new(&bytes).unwrap().read_by_name(&name, 5).unwrap(), b"valid");
    }

    #[test]
    fn roundtrip() {
        let mut w = ZipWriter::new();
        w.add("a.txt", b"hello").unwrap();
        w.add("dir/b.bin", &[0, 1, 2, 3]).unwrap();
        w.add("empty", &[]).unwrap();
        let bytes = w.finish().unwrap();
        let r = ZipReader::new(&bytes).unwrap();
        assert_eq!(r.entries.len(), 3);
        assert_eq!(r.read_by_name("a.txt", 100).unwrap(), b"hello");
        assert_eq!(r.read_by_name("dir/b.bin", 100).unwrap(), [0, 1, 2, 3]);
        assert!(r.read_by_name("empty", 0).unwrap().is_empty());
        assert!(matches!(r.read_by_name("a.txt", 3), Err(FormatError::LimitExceeded(_))));
        assert!(r.by_name.keys().all(|name| matches!(name, Cow::Borrowed(_))));
    }

    fn archive_with_comment(comment: &[u8]) -> Vec<u8> {
        let mut writer = ZipWriter::new();
        writer.add("manifest.json", b"project content").unwrap();
        let mut bytes = writer.finish().unwrap();
        let length_offset = bytes.len() - 2;
        bytes[length_offset..].copy_from_slice(&u16::try_from(comment.len()).unwrap().to_le_bytes());
        bytes.extend_from_slice(comment);
        bytes
    }

    #[test]
    fn directory_signature_inside_comment_does_not_hide_entries() {
        for length in [22, 32, usize::from(u16::MAX)] {
            let mut comment = vec![0; length];
            comment[..4].copy_from_slice(&EOCD_SIG.to_le_bytes());
            let bytes = archive_with_comment(&comment);
            let reader = ZipReader::new(&bytes).unwrap();
            assert_eq!(reader.entries.len(), 1, "comment length {length}");
            assert_eq!(reader.read_by_name("manifest.json", 100).unwrap(), b"project content");
        }
    }

    #[test]
    fn archive_comments_support_empty_short_and_maximum_lengths() {
        for length in [0, 1, 21, 22, 100, usize::from(u16::MAX)] {
            let bytes = archive_with_comment(&vec![b'x'; length]);
            let reader = ZipReader::new(&bytes).unwrap();
            assert_eq!(reader.read_by_name("manifest.json", 100).unwrap(), b"project content", "comment length {length}");
        }
    }

    #[test]
    fn central_directory_digital_signature_remains_supported() {
        let mut bytes = archive_with_comment(b"signed archive");
        let eocd = bytes.len() - 22 - b"signed archive".len();
        let signature = [0x50, 0x4b, 0x05, 0x05, 3, 0, 1, 2, 3];
        let size = u32_at(&bytes, eocd + 12).unwrap() + signature.len() as u32;
        bytes[eocd + 12..eocd + 16].copy_from_slice(&size.to_le_bytes());
        bytes.splice(eocd..eocd, signature);
        let reader = ZipReader::new(&bytes).unwrap();
        assert_eq!(reader.read_by_name("manifest.json", 100).unwrap(), b"project content");
    }

    #[test]
    fn truncated_archive_comment_is_rejected() {
        let mut bytes = archive_with_comment(b"comment");
        bytes.pop();
        assert!(ZipReader::new(&bytes).is_err());
    }

    #[test]
    fn crc_matches_bitwise_reference() {
        // Bit-at-a-time IEEE CRC-32 as the oracle, over lengths that cross
        // any SIMD / slicing block size.
        fn reference(data: &[u8]) -> u32 {
            let mut c = 0xFFFF_FFFFu32;
            for &b in data {
                c ^= u32::from(b);
                for _ in 0..8 {
                    c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
                }
            }
            !c
        }
        let data: Vec<u8> = (0..1000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
        for len in [0, 1, 7, 15, 16, 17, 63, 64, 65, 255, 256, 1000] {
            assert_eq!(crc32(&data[..len]), reference(&data[..len]), "len {len}");
        }
    }

    #[test]
    fn find_locates_every_entry_in_a_large_archive() {
        let mut w = ZipWriter::new();
        let n = 3000u32;
        for i in 0..n {
            w.add(&format!("tiles/{i:05}.zst"), &i.to_le_bytes()).unwrap();
        }
        let bytes = w.finish().unwrap();
        let r = ZipReader::new(&bytes).unwrap();
        assert_eq!(r.entries.len(), n as usize);
        for i in (0..n).rev() {
            let name = format!("tiles/{i:05}.zst");
            let e = r.find(&name).unwrap();
            assert_eq!(e.name, name);
            assert_eq!(r.read(e, 4).unwrap(), i.to_le_bytes());
        }
        assert!(r.find("tiles/missing.zst").is_none());
        assert!(r.find("").is_none());
    }

    #[test]
    fn duplicate_names_resolve_to_the_first_entry() {
        let mut w = ZipWriter::new();
        w.add("a", b"first").unwrap();
        w.add("b", b"other").unwrap();
        w.add("a", b"second").unwrap();
        let bytes = w.finish().unwrap();
        let r = ZipReader::new(&bytes).unwrap();
        assert_eq!(r.entries.len(), 3);
        assert_eq!(r.read_by_name("a", 100).unwrap(), b"first");
        assert_eq!(r.read_by_name("b", 100).unwrap(), b"other");
    }

    /// Reservation changes neither entry bytes nor final output, and avoids growth.
    #[test]
    fn whole_archive_reservation_preserves_output() {
        let entries = [("层/é.png", &b"image"[..]), ("empty", &b""[..]), ("empty", &b"duplicate"[..])];
        for entries in [&entries[..0], &entries[..]] {
            let mut previous = ZipWriter::new();
            let mut reserved = ZipWriter::with_entries(entries.iter().map(|(name, data)| (*name, data.len()))).unwrap();
            let (capacity, directory) = (reserved.out.capacity(), reserved.central.capacity());
            for (name, data) in entries {
                previous.add(name, data).unwrap();
                reserved.add(name, data).unwrap();
            }
            assert_eq!(reserved.central.capacity(), directory);
            let bytes = reserved.finish().unwrap();
            assert_eq!(bytes.capacity(), capacity);
            assert_eq!(bytes, previous.finish().unwrap());
        }
        assert!(matches!(ZipWriter::with_entries([("x", usize::MAX)]), Err(FormatError::Unsupported(_))));
        assert!(matches!(ZipWriter::with_entries([("x", isize::MAX as usize)]), Err(FormatError::LimitExceeded(_))));
    }

    /// Invalid UTF-8 names retain their lossy spelling and first-entry lookup.
    #[test]
    fn name_index_borrows_unicode_and_owns_lossy_names() {
        let mut writer = ZipWriter::new();
        for name in ["层/é.png", "a", "b"] {
            writer.add(name, name.as_bytes()).unwrap();
        }
        let mut bytes = writer.finish().unwrap();
        let mut p = u32_at(&bytes, bytes.len() - 6).unwrap() as usize;
        p += 46 + "层/é.png".len();
        bytes[p + 46] = 0xff;
        bytes[p + 47 + 46] = 0xfe;
        let reader = ZipReader::new(&bytes).unwrap();
        assert_eq!(reader.read_by_name("层/é.png", 100).unwrap(), "层/é.png".as_bytes());
        assert!(matches!(reader.by_name.get_key_value("层/é.png").unwrap().0, Cow::Borrowed(_)));
        assert!(matches!(reader.by_name.get_key_value("�").unwrap().0, Cow::Owned(_)));
        assert_eq!(reader.read_by_name("�", 1).unwrap(), b"a");
    }

    #[test]
    fn crc_mismatch_detected() {
        let mut w = ZipWriter::new();
        w.add("a", b"hello").unwrap();
        let mut bytes = w.finish().unwrap();
        let pos = bytes.windows(5).position(|x| x == b"hello").unwrap();
        bytes[pos] = b'j';
        let r = ZipReader::new(&bytes).unwrap();
        assert!(r.read_by_name("a", 100).is_err());
    }

    #[test]
    fn slice_deflate_matches_buffered_reader_and_preserves_checks() {
        use std::io::Write as _;

        let content = b"PhotoCraft ZIP payload".repeat(5000);
        let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::none());
        encoder.write_all(&content).unwrap();
        let compressed = encoder.finish().unwrap();
        assert!(compressed.len() > 32 * 1024);
        let mut previous = Vec::new();
        flate2::read::DeflateDecoder::new(compressed.as_slice()).read_to_end(&mut previous).unwrap();
        let mut writer = ZipWriter::new();
        writer.add("payload", &compressed).unwrap();
        let bytes = writer.finish().unwrap();
        let reader = ZipReader::new(&bytes).unwrap();
        let mut entry = reader.entries[0].clone();
        entry.method = 8;
        entry.uncompressed = content.len();
        entry.crc = crc32(&content);
        assert_eq!(reader.read(&entry, content.len()).unwrap(), previous);
        assert!(matches!(reader.read(&entry, content.len() - 1), Err(FormatError::LimitExceeded(_))));
        entry.crc ^= 1;
        assert!(matches!(reader.read(&entry, content.len()), Err(FormatError::Corrupt(_))));
        entry.crc ^= 1;
        entry.compressed /= 2;
        assert!(matches!(reader.read(&entry, content.len()), Err(FormatError::Corrupt(_))));
    }

    /// A false small size must stop expansion before decoding the rest of the stream.
    #[test]
    fn deflate_surplus_stops_before_a_corrupt_suffix() {
        // Exceed miniz's internal 32 KiB output buffer before the corrupt block.
        let raw = [&[0, 0xff, 0xff, 0, 0][..], &[b'x'; 65535], &[7]].concat();
        let reader = ZipReader { bytes: &raw, entries: Vec::new(), by_name: HashMap::new() };
        for declared in [0, 7] {
            let entry = Entry { name: "surplus".into(), method: 8, crc: 0, compressed: raw.len(), uncompressed: declared, data_offset: 0 };
            let err = reader.read(&entry, usize::MAX).unwrap_err();
            assert!(err.to_string().contains("wrong size"), "{err}");
        }
    }

    /// Correct output length and CRC do not replace the final DEFLATE block.
    #[test]
    fn deflate_requires_a_complete_stream_including_empty_entries() {
        for (raw, content) in [(&[0, 1, 0, 0xfe, 0xff, b'x', 1, 0, 0, 0xff, 0xff][..], &b"x"[..]), (&[3, 0][..], &b""[..])] {
            let reader = ZipReader { bytes: raw, entries: Vec::new(), by_name: HashMap::new() };
            let mut entry =
                Entry { name: "complete".into(), method: 8, crc: crc32(content), compressed: raw.len(), uncompressed: content.len(), data_offset: 0 };
            assert_eq!(reader.read(&entry, content.len()).unwrap(), content);
            for cut in 0..raw.len() {
                entry.compressed = cut;
                assert!(matches!(reader.read(&entry, content.len()), Err(FormatError::Corrupt(_))), "cut at {cut}");
            }
        }
    }

    /// An unlimited caller budget still checks a DEFLATE entry's declared size.
    #[test]
    fn deflated_entry_with_unbounded_limit_round_trips() {
        use std::io::Write as _;

        let content = b"DEFLATE-compressed PhotoCraft project content, with a valid CRC.";
        let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(content).unwrap();
        let compressed = encoder.finish().unwrap();

        // Construct a standards-conforming DEFLATE entry from the store-only
        // writer: keep its actual compressed bytes, but set the ZIP method,
        // original length and CRC in *both* the local and central headers.
        let mut writer = ZipWriter::new();
        writer.add("manifest.json", &compressed).unwrap();
        let mut bytes = writer.finish().unwrap();
        let eocd = bytes.len() - 22;
        let central = u32_at(&bytes, eocd + 16).unwrap() as usize;
        let crc = crc32(content).to_le_bytes();
        let length = (content.len() as u32).to_le_bytes();

        bytes[8..10].copy_from_slice(&8u16.to_le_bytes());
        bytes[14..18].copy_from_slice(&crc);
        bytes[22..26].copy_from_slice(&length);
        bytes[central + 10..central + 12].copy_from_slice(&8u16.to_le_bytes());
        bytes[central + 16..central + 20].copy_from_slice(&crc);
        bytes[central + 24..central + 28].copy_from_slice(&length);

        let archive = ZipReader::new(&bytes).unwrap();
        assert_eq!(archive.read_by_name("manifest.json", usize::MAX).unwrap(), content);
        assert_eq!(archive.read_by_name("manifest.json", content.len()).unwrap(), content);
        assert!(matches!(archive.read_by_name("manifest.json", content.len() - 1), Err(FormatError::LimitExceeded(_))));

        // A tiny archive with a forged 4 GiB size must be rejected without
        // trying to allocate its claimed output capacity in advance.
        let mut forged = bytes;
        forged[22..26].copy_from_slice(&u32::MAX.to_le_bytes());
        forged[central + 24..central + 28].copy_from_slice(&u32::MAX.to_le_bytes());
        let archive = ZipReader::new(&forged).unwrap();
        assert!(matches!(archive.read_by_name("manifest.json", usize::MAX), Err(FormatError::Corrupt(_))));
    }

    #[test]
    fn an_entry_from_another_archive_is_an_error_not_a_panic() {
        let mut big = ZipWriter::new();
        big.add("a", &[7; 4096]).unwrap();
        let big = big.finish().unwrap();
        let foreign = ZipReader::new(&big).unwrap().entries[0].clone();
        let mut small = ZipWriter::new();
        small.add("a", b"x").unwrap();
        let small = small.finish().unwrap();
        let r = ZipReader::new(&small).unwrap();
        assert!(matches!(r.read(&foreign, usize::MAX), Err(FormatError::Corrupt(_))));
    }

    #[test]
    fn offset_access_rejects_overflow() {
        assert!(u16_at(&[], usize::MAX).is_err());
        assert!(u32_at(&[], usize::MAX - 3).is_err());
    }
}
