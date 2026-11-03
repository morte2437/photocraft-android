//! Encoded channels parked on the scratch disk while a large PSD/PSB save is assembled (#5333).
//!
//! A save encodes the whole file model before it writes a byte (#8115: it can still be
//! cancelled, and the scratch reads behind it are checked, before the file is replaced). In
//! memory that model is a second, compressed copy of every layer. With
//! [`PsdExportOptions::stage_on_scratch_from`](super::PsdExportOptions::stage_on_scratch_from)
//! set and spilling on, each channel is appended to a [`StageFile`] in the scratch directory as
//! soon as it is encoded, and the model keeps only a [`Stored`] range: the writer then copies it
//! through in short reads (`photocraft_psd`'s `put_encoded`). A channel that cannot be staged
//! (the disk is full, say) stays in memory: staging only ever saves memory.

use std::sync::Arc;

use photocraft_psd::source::{ReadAt, Source, Stored};
use photocraft_psd::{ChannelData, ImageData, PsdError};
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

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> photocraft_psd::Result<()> {
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

    /// Bytes on the scratch disk.
    #[cfg(test)]
    pub(crate) fn len(&self) -> u64 {
        self.file.0.len()
    }
}
