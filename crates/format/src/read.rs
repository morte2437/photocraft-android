//! Reading a whole file to open it: the read side of [`crate::atomic`].
//!
//! What it adds over `std::fs::read` for multi-gigabyte PSB files:
//!
//! - **Bounded reads.** Each read asks for at most [`READ_CHUNK`] bytes. `std` asks Windows for up
//!   to 4 GiB in one `ReadFile`, and some file systems (exFAT, network redirectors) reject a read
//!   that large with `ERROR_INVALID_PARAMETER` ("The parameter is incorrect", #375).
//! - **A clear out-of-memory error.** The buffer is reserved up front with `try_reserve_exact`,
//!   and a file larger than the memory available says how much it needed. A file that grows past
//!   its stat length while being read grows the buffer with `try_reserve` too.
//! - **A total limit.** No read returns more than [`MAX_OPEN_BYTES`], so a file that keeps growing
//!   while it is read cannot keep an open or a save from finishing.
//!
//! On `wasm32` there is no file system and every call returns the `std` "unsupported" error.

use std::io::{self, Read};
use std::path::Path;

/// Largest single read: big enough that a 5 GB file takes under a hundred reads.
pub const READ_CHUNK: usize = 64 << 20;

/// Most bytes [`read_file`] and [`read_all`] return. Far above any document memory can hold (a
/// PSB can run to tens of gigabytes), it only bounds a file that never stops growing; a file over it
/// is a `FileTooLarge` error, never a truncated read.
pub const MAX_OPEN_BYTES: u64 = 1 << 38;

/// Read all of `path`, in reads of at most [`READ_CHUNK`] bytes and at most [`MAX_OPEN_BYTES`] in
/// all. Anything but a regular file is an `InvalidInput` error.
pub fn read_file(path: &Path) -> io::Result<Vec<u8>> {
    read_file_with(path, READ_CHUNK)
}

/// [`read_file`] with a chosen read size (tests use small ones to exercise the loop).
pub fn read_file_with(path: &Path, chunk: usize) -> io::Result<Vec<u8>> {
    read_path(path, chunk, MAX_OPEN_BYTES)
}

/// [`read_file_with`] with a chosen total limit (tests use small ones).
fn read_path(path: &Path, chunk: usize, max: u64) -> io::Result<Vec<u8>> {
    #[cfg(unix)]
    let (mut f, metadata) = {
        use rustix::fs::{Mode, OFlags, open};
        let fd = open(path, OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC, Mode::empty()).map_err(io::Error::from)?;
        let f = std::fs::File::from(fd);
        let metadata = f.metadata()?;
        if !metadata.is_file() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "path is not a regular file"));
        }
        rustix::fs::fcntl_setfl(&f, OFlags::empty()).map_err(io::Error::from)?;
        (f, metadata)
    };
    #[cfg(not(unix))]
    let (mut f, metadata) = {
        let f = std::fs::File::open(path)?;
        let metadata = f.metadata()?;
        if !metadata.is_file() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "path is not a regular file"));
        }
        (f, metadata)
    };
    let len = metadata.len();
    read_all_with(&mut f, len, chunk, max)
}

/// Read `reader` to its end in reads of at most [`READ_CHUNK`] bytes, for an already open file
/// (a capability-scoped handle, say). `size_hint` (the file's length) sizes the buffer up front.
/// More than [`MAX_OPEN_BYTES`] in all is a `FileTooLarge` error.
pub fn read_all(reader: &mut impl Read, size_hint: u64) -> io::Result<Vec<u8>> {
    read_all_with(reader, size_hint, READ_CHUNK, MAX_OPEN_BYTES)
}

/// Read `reader` to its end in reads of at most `chunk` bytes, failing once it has given more
/// than `max` bytes. The hint only sizes the buffer: a file that grows or shrinks while being
/// read is still read to its end, within `max`. Past the hint the buffer grows only after a probe
/// read has found more bytes, and fallibly, so a file read exactly to its stat length is never
/// over-allocated and one that outgrows memory is an `OutOfMemory` error, not an abort.
fn read_all_with(reader: &mut impl Read, size_hint: u64, chunk: usize, max: u64) -> io::Result<Vec<u8>> {
    if size_hint > max {
        return Err(too_large(max));
    }
    let mut buf = buffer_for(size_hint)?;
    let chunk = chunk.max(1);
    // One byte over `max` is enough to tell a file over the limit from one exactly at it.
    let mut reader = reader.take(max.saturating_add(1));
    loop {
        if buf.len() == buf.capacity() {
            let mut probe = [0; 4096];
            let n = loop {
                match reader.read(&mut probe) {
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    r => break r?,
                }
            };
            if n == 0 {
                break;
            }
            let needed = buf.len() as u64 + n as u64;
            buf.try_reserve(n.max(chunk)).map_err(|_| out_of_memory(needed))?;
            buf.extend_from_slice(&probe[..n]);
        }
        let room = u64::try_from((buf.capacity() - buf.len()).min(chunk)).unwrap_or(u64::MAX);
        if (&mut reader).take(room).read_to_end(&mut buf)? == 0 && buf.len() < buf.capacity() {
            break;
        }
    }
    if buf.len() as u64 > max {
        return Err(too_large(max));
    }
    Ok(buf)
}

/// The error for a file over the `max` bytes a read may return.
fn too_large(max: u64) -> io::Error {
    io::Error::new(io::ErrorKind::FileTooLarge, format!("the file is larger than {} and was not read", size_text(max)))
}

/// The `OutOfMemory` error for a read that needed `len` bytes.
fn out_of_memory(len: u64) -> io::Error {
    io::Error::new(io::ErrorKind::OutOfMemory, format!("not enough memory to read the whole file ({})", size_text(len)))
}

/// An empty buffer with room for `len` bytes, or an `OutOfMemory` error saying how much was needed.
pub fn buffer_for(len: u64) -> io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    let reserved = usize::try_from(len).ok().and_then(|n| buf.try_reserve_exact(n).ok());
    match reserved {
        Some(()) => Ok(buf),
        None => Err(out_of_memory(len)),
    }
}

/// `len` bytes for people: "4.75 GB", "759 MB", "12 KB".
fn size_text(len: u64) -> String {
    const UNITS: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];
    let mut v = len as f64;
    let mut unit = 0;
    while v >= 1000.0 && unit + 1 < UNITS.len() {
        v /= 1000.0;
        unit += 1;
    }
    match UNITS.get(unit) {
        Some(name) if unit > 0 => format!("{v:.2} {name}"),
        _ => format!("{len} bytes"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("photocraft-read-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("f.bin");
        std::fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn reads_whole_file_across_many_chunks() {
        let data: Vec<u8> = (0..100_003u32).map(|i| (i * 7 % 251) as u8).collect();
        let p = temp("chunks", &data);
        for chunk in [1, 7, 4096, 100_003, 1 << 20] {
            assert_eq!(read_file_with(&p, chunk).unwrap(), data, "chunk {chunk}");
        }
        assert_eq!(read_file(&p).unwrap(), data);
        assert_eq!(read_file_with(&p, 0).unwrap(), data, "a zero chunk still makes progress");
        // From an open reader, whatever the hint says.
        for hint in [0, 10, data.len() as u64, 1 << 20] {
            assert_eq!(read_all(&mut std::io::Cursor::new(&data), hint).unwrap(), data, "hint {hint}");
        }
    }

    /// Nothing over the total limit is returned (#8601): a file larger than it, a reader that
    /// gives more than it whatever its hint says, and one that never ends are `FileTooLarge`
    /// errors, while a file exactly at the limit is read whole.
    #[test]
    fn reads_stop_at_the_total_limit() {
        let data: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();
        let p = temp("limit", &data);
        assert_eq!(read_path(&p, 7, data.len() as u64).unwrap(), data, "exactly at the limit");
        assert_eq!(read_path(&p, 7, data.len() as u64 - 1).unwrap_err().kind(), io::ErrorKind::FileTooLarge);
        for hint in [0, 100, data.len() as u64] {
            let e = read_all_with(&mut std::io::Cursor::new(&data), hint, 64, 5_000).unwrap_err();
            assert_eq!(e.kind(), io::ErrorKind::FileTooLarge, "hint {hint}");
        }
        let e = read_all_with(&mut std::io::Cursor::new(&data), 1 << 40, 64, 5_000).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::FileTooLarge, "a hint over the limit fails before reading");
        let e = read_all_with(&mut std::io::repeat(1), 0, 1 << 20, 3 << 20).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::FileTooLarge, "a reader that never ends");
        assert!(e.to_string().contains("3.15 MB"), "{e}");
    }

    /// A file read exactly to its stat length keeps the buffer it reserved: the end is found by a
    /// probe read, without growing the buffer first.
    #[test]
    fn an_exact_hint_is_not_over_allocated() {
        let data = vec![7u8; 100_000];
        let buf = read_all_with(&mut std::io::Cursor::new(&data), data.len() as u64, 4096, MAX_OPEN_BYTES).unwrap();
        assert_eq!(buf, data);
        assert_eq!(buf.capacity(), data.len());
    }

    #[test]
    fn empty_and_missing_files() {
        assert!(read_file(&temp("empty", b"")).unwrap().is_empty());
        let missing = std::env::temp_dir().join("photocraft-read-does-not-exist.psb");
        assert_eq!(read_file(&missing).unwrap_err().kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn a_file_too_large_for_memory_is_a_clear_error() {
        // More than any address space can hold.
        let e = buffer_for(u64::MAX).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::OutOfMemory);
        assert!(e.to_string().contains("18446744.07 TB"), "{e}");
        let e = buffer_for(1 << 62).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::OutOfMemory);
        assert_eq!(buffer_for(1024).unwrap().capacity(), 1024);
    }

    #[test]
    fn sizes_read_like_the_os_shows_them() {
        assert_eq!(size_text(512), "512 bytes");
        assert_eq!(size_text(4_750_000_000), "4.75 GB");
        assert_eq!(size_text(759_000_000), "759.00 MB");
    }
}
