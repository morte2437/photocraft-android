//! Encoded channels parked on the scratch disk while a large PSD/PSB save is assembled (#5333).
//!
//! A save encodes the whole file model before it writes a byte (#8115: it can still be
//! cancelled, and the scratch reads behind it are checked, before the file is replaced). In
//! memory that model is a second, compressed copy of every layer. With
//! [`PsdExportOptions::stage_on_scratch_from`](super::PsdExportOptions::stage_on_scratch_from)
//! set and spilling on, each channel is appended to a [`StageFile`] in the scratch directory as
//! soon as it is encoded, and the model keeps only a [`Stored`] range: the writer then copies it
//! through in short reads (`craft_psd`'s `put_encoded`). A channel that cannot be staged
//! (the disk is full, say) stays in memory: staging only ever saves memory.

use std::sync::Arc;

use craft_psd::source::{ReadAt, Source, Stored};
use craft_psd::{ChannelData, ImageData, PsdError};
use photocraft_raster::stage_file::StageFile;

/// Encoded data shorter than this stays in memory: a range costs more than it saves.
const MIN_STAGED_BYTES: usize = 64 << 10;
/// Raw pixel bytes (layers and composite, as sampled) from which a save made by
/// [`crate::export_with`] stages its channels: about a 24 MP RGBA/16 document with five layers.
pub(crate) const STAGE_DOCUMENTS_FROM: u64 = 1 << 30;

#[cfg(test)]
thread_local! {
    /// Where this thread's exports stage, instead of the process-wide scratch directory.
    pub(crate) static DIR_FOR_TESTS: std::cell::RefCell<Option<std::path::PathBuf>> = const { std::cell::RefCell::new(None) };
}

struct Backing(StageFile);

impl ReadAt for Backing {
    fn len(&self) -> u64 {
        self.0.len()
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> craft_psd::Result<()> {
        self.0.read_exact_at(buf, offset).map_err(|e| PsdError::Read(format!("staged export data, {} bytes at offset {offset}: {e}", buf.len())))
    }
}

/// The stage of one export: a file on the scratch disk shared by every channel.
pub(crate) struct Stage {
    file: Arc<Backing>,
    source: Source,
}

impl Stage {
    /// A stage for a document of `raw_bytes` pixel bytes, when `from` asks for one at that size
    /// and a stage file can be made in the scratch directory.
    pub(crate) fn for_document(raw_bytes: u64, from: Option<u64>) -> Option<Arc<Self>> {
        if raw_bytes < from? {
            return None;
        }
        #[cfg(test)]
        if let Some(dir) = DIR_FOR_TESTS.with(|d| d.borrow().clone()) {
            return StageFile::create(&dir).ok().map(Self::new);
        }
        StageFile::in_scratch().map(Self::new)
    }

    fn new(file: StageFile) -> Arc<Self> {
        let file = Arc::new(Backing(file));
        let source = Source::from_arc(file.clone());
        Arc::new(Self { file, source })
    }

    /// Moves `data` to the stage; `None` (the data kept by the caller) when it is short or the
    /// write fails.
    fn park(&self, data: &[u8]) -> Option<Stored> {
        if data.len() < MIN_STAGED_BYTES {
            return None;
        }
        let offset = self.file.0.append(data).ok()?;
        Some(Stored { source: self.source.clone(), offset, len: data.len() as u64 })
    }

    pub(crate) fn channel(&self, mut c: ChannelData) -> ChannelData {
        if c.stored.is_none()
            && let Some(stored) = self.park(&c.data)
        {
            c.data = Vec::new();
            c.stored = Some(stored);
        }
        c
    }

    pub(crate) fn image(&self, mut image: ImageData) -> ImageData {
        if image.stored.is_none()
            && let Some(stored) = self.park(&image.data)
        {
            image.data = Vec::new();
            image.stored = Some(stored);
        }
        image
    }

    /// Appends one part of a channel written band by band; `None` when the write fails.
    fn append(&self, data: &[u8]) -> Option<u64> {
        self.file.0.append(data).ok()
    }

    /// Bytes on the scratch disk.
    #[cfg(test)]
    pub(crate) fn len(&self) -> u64 {
        self.file.0.len()
    }
}

/// One channel of a layer encoded band by band (#5333): its head (an RLE row-count table, filled
/// in when the last band is encoded) and the bands' encoded parts in order. With a stage every
/// part goes to the scratch disk as it is made, so a layer of any size is saved holding about one
/// band; without one the parts are joined in memory, as a channel encoded whole would be.
pub(crate) struct Parts {
    stage: Option<Arc<Stage>>,
    head_len: usize,
    /// Without a stage: the head's room and the parts so far.
    joined: Vec<u8>,
    /// With a stage: the parts, staged or (when a write failed) kept.
    parts: Vec<Part>,
    len: u64,
}

enum Part {
    Kept(Vec<u8>),
    Staged { offset: u64, len: u64 },
}

impl Part {
    fn len(&self) -> u64 {
        match self {
            Part::Kept(v) => v.len() as u64,
            Part::Staged { len, .. } => *len,
        }
    }
}

impl Parts {
    pub(crate) fn new(stage: Option<Arc<Stage>>, head_len: usize) -> Self {
        let joined = if stage.is_none() { vec![0; head_len] } else { Vec::new() };
        Parts { stage, head_len, joined, parts: Vec::new(), len: head_len as u64 }
    }

    /// Adds the next encoded part.
    pub(crate) fn push(&mut self, data: Vec<u8>) {
        if data.is_empty() {
            return;
        }
        self.len += data.len() as u64;
        match &self.stage {
            None => self.joined.extend_from_slice(&data),
            Some(stage) => self.parts.push(match stage.append(&data) {
                Some(offset) => Part::Staged { offset, len: data.len() as u64 },
                None => Part::Kept(data),
            }),
        }
    }

    /// The channel: `head` (of the length given to [`Parts::new`]) and then the parts.
    pub(crate) fn finish(mut self, id: i16, compression: craft_psd::Compression, head: &[u8]) -> craft_psd::Result<ChannelData> {
        if head.len() != self.head_len {
            return Err(PsdError::Invalid("banded channel head has the wrong length".into()));
        }
        let Some(stage) = self.stage else {
            self.joined[..head.len()].copy_from_slice(head);
            self.joined.shrink_to_fit();
            return Ok(ChannelData::new(id, Some(compression), self.joined));
        };
        let mut parts = Vec::with_capacity(self.parts.len() + 1);
        if !head.is_empty() {
            parts.push(Part::Kept(head.to_vec()));
        }
        parts.append(&mut self.parts);
        let mut starts = Vec::with_capacity(parts.len());
        let mut at = 0;
        for p in &parts {
            starts.push(at);
            at += p.len();
        }
        let joined = Joined { file: stage.file.clone(), parts, starts, len: self.len };
        let stored = Stored { source: Source::from_arc(Arc::new(joined)), offset: 0, len: self.len };
        Ok(ChannelData { id, compression: Some(compression), data: Vec::new(), stored: Some(stored) })
    }
}

/// A banded channel's bytes, in memory and in its staged parts. A read is split at the parts, so
/// the writer's consecutive pass reads each staged append from its start to its end, and the stage
/// file checks it.
struct Joined {
    file: Arc<Backing>,
    parts: Vec<Part>,
    starts: Vec<u64>,
    len: u64,
}

impl ReadAt for Joined {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> craft_psd::Result<()> {
        if offset.checked_add(buf.len() as u64).is_none_or(|end| end > self.len) {
            return Err(PsdError::Read(format!("staged export data, {} bytes at offset {offset} past its {} bytes", buf.len(), self.len)));
        }
        let mut pos = offset;
        let mut buf = buf;
        let mut i = self.starts.partition_point(|&s| s <= pos).saturating_sub(1);
        while !buf.is_empty() {
            let (part, start) = (&self.parts[i], self.starts[i]);
            let within = pos - start;
            let n = usize::try_from((part.len() - within).min(buf.len() as u64)).unwrap_or(buf.len());
            let (now, rest) = std::mem::take(&mut buf).split_at_mut(n);
            match part {
                Part::Kept(v) => now.copy_from_slice(&v[within as usize..within as usize + n]),
                Part::Staged { offset, .. } => self.file.read_at(offset + within, now)?,
            }
            buf = rest;
            pos += n as u64;
            i += 1;
        }
        Ok(())
    }
}
