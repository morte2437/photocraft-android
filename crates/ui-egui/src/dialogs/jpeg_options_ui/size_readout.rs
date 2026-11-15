//! JPEG Options' file size: while choosing a quality, the dialog shows how big the JPEG will be
//! ("152.3K"), and updates it as Quality and Format Options change. The dialog's document is
//! encoded in memory with the settings OK would write it with (the same encoder and
//! [`ExportSettings`] as the save, so the size is the file's, metadata and profile included).
//!
//! The encode runs on a worker thread in the native app, so the dialog never waits for it; the web
//! build has no threads and encodes in the frame, so its limit is lower. One encode runs at a time,
//! and the last few answers are kept for their settings: a frame that changes nothing encodes
//! nothing, and going back to settings already worked out shows their size at once. While a new
//! size is worked out, the last one for the same document stays up marked as pending ("152.3K…").
//! Documents over [`MAX_PX`] aren't encoded and read "—".

use std::collections::VecDeque;

use std::sync::{Arc, Mutex, PoisonError};

use photocraft_codecs::JpegFormat;
use photocraft_doc::{DocId, Document};

use crate::{ExportSettings, PhotocraftApp};

/// The largest document, in pixels, whose size is worked out. Flattening and encoding 40 MP takes
/// a second or two on a worker and about 160 MB at 8 bits, as Export As's exact size does (#6595).
#[cfg(not(target_arch = "wasm32"))]
const MAX_PX: u64 = 40_000_000;
/// The web build encodes in the frame: up to 4 MP keeps a settings change under a frame or so.
#[cfg(target_arch = "wasm32")]
const MAX_PX: u64 = 4_000_000;

/// Whether a document of `size` gets a size readout (see [`MAX_PX`]).
pub(super) fn estimable(size: photocraft_doc::Size) -> bool {
    size.area() <= MAX_PX
}

/// A size as the readout shows it: kilobytes with one decimal under a megabyte
/// ("152.3K"), megabytes above ("1.5M"), in units of 1024.
pub(super) fn size_label(bytes: u64) -> String {
    const K: f64 = 1024.0;
    let kilo = (bytes as f64 / K * 10.0).round() / 10.0;
    if kilo < K { format!("{kilo:.1}K") } else { format!("{:.1}M", bytes as f64 / (K * K)) }
}

/// The bytes `doc` takes written as a JPEG with `settings`, as the save writes it; `None` when it
/// can't be encoded.
pub(super) fn encoded_size(doc: &Document, settings: &ExportSettings) -> Option<u64> {
    let bytes = photocraft_io::export(doc, "size.jpg", &settings.io_options()).ok()?.bytes;
    u64::try_from(bytes.len()).ok()
}

/// What one size is for: the document as it is, and the encoder quality (1–100) and format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Key {
    doc: DocId,
    revision: u64,
    quality: u8,
    format: JpegFormat,
}

/// How many worked-out sizes are kept, so stepping between a few qualities and formats doesn't
/// encode again.
const KEPT: usize = 8;

/// What the readout says for one set of settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Estimate {
    /// The size for these settings (`None`: they couldn't be encoded).
    Known(Option<u64>),
    /// Still being worked out, with the last size of the same document as it is now, if any.
    Pending(Option<u64>),
}

impl Estimate {
    /// "152.3K" when known, "152.3K…" while the last size stands in, "…" with none yet, and "—"
    /// when the settings couldn't be encoded.
    pub(super) fn label(self) -> String {
        match self {
            Estimate::Known(Some(bytes)) => size_label(bytes),
            Estimate::Known(None) => "—".into(),
            Estimate::Pending(Some(bytes)) => format!("{}…", size_label(bytes)),
            Estimate::Pending(None) => "…".into(),
        }
    }
}

/// Where a worker leaves its answer: `Some` once done (`Some(None)`: couldn't be encoded).
type Slot = Arc<Mutex<Option<Option<u64>>>>;

/// The readout between frames (kept in egui's temporary data).
#[derive(Default)]
pub(super) struct Readout {
    /// The last [`KEPT`] sizes worked out and what each is for, newest last.
    known: VecDeque<(Key, Option<u64>)>,
    /// The encode running, and what it is for.
    running: Option<(Key, Slot)>,
}

impl Readout {
    /// What to show for `key`. Unless `key`'s size is known or being worked out, starts the job
    /// `make` returns, which calls `done` when it finishes; a change made while another encode
    /// runs starts once that one is done.
    pub(super) fn update<J>(&mut self, key: Key, make: impl FnOnce() -> J, done: impl FnOnce() + Send + 'static) -> Estimate
    where
        J: FnOnce() -> Option<u64> + Send + 'static,
    {
        self.collect();
        if self.running.is_none() && self.size_of(key).is_none() {
            let slot = Slot::default();
            run(make(), Arc::clone(&slot), done);
            self.running = Some((key, slot));
            // The web build encodes in the frame: its answer is already there.
            self.collect();
        }
        if let Some(size) = self.size_of(key) {
            return Estimate::Known(size);
        }
        let same_doc = self.known.iter().rev().find(|(k, _)| k.doc == key.doc && k.revision == key.revision);
        Estimate::Pending(same_doc.and_then(|&(_, size)| size))
    }

    /// `key`'s size if it has been worked out (`Some(None)`: it couldn't be).
    fn size_of(&self, key: Key) -> Option<Option<u64>> {
        self.known.iter().find(|(k, _)| *k == key).map(|&(_, size)| size)
    }

    /// Takes a finished encode's answer.
    fn collect(&mut self) {
        let Some((key, slot)) = &self.running else { return };
        let Some(size) = slot.lock().unwrap_or_else(PoisonError::into_inner).take() else { return };
        let key = *key;
        self.running = None;
        self.known.retain(|(k, _)| *k != key);
        if self.known.len() == KEPT {
            self.known.pop_front();
        }
        self.known.push_back((key, size));
    }
}

/// Runs `job` on a worker (in the web build, right here), leaves its answer in `slot`, then calls
/// `done`. A job that panics answers `None`.
fn run(job: impl FnOnce() -> Option<u64> + Send + 'static, slot: Slot, done: impl FnOnce() + Send + 'static) {
    let answer = Arc::clone(&slot);
    let work = move || {
        let size = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)).unwrap_or(None);
        *answer.lock().unwrap_or_else(PoisonError::into_inner) = Some(size);
        done();
    };
    #[cfg(not(target_arch = "wasm32"))]
    if std::thread::Builder::new().name("jpeg-options-size".into()).spawn(work).is_err() {
        *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(None);
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = slot;
        work();
    }
}

/// What the readout encodes: the waiting save's document, with the settings its save starts from.
/// Taken before the dialog is drawn, which borrows the prompt.
pub(super) struct Source {
    doc: Arc<Document>,
    revision: u64,
    base: ExportSettings,
    own_quality: Option<u8>,
}

impl Source {
    /// For the JPEG Options prompt waiting, if any.
    pub(super) fn of(app: &PhotocraftApp) -> Option<Self> {
        let p = app.jpeg_options.as_ref()?;
        let st = app.session.documents().iter().find(|st| st.doc.id == p.doc)?;
        Some(Source { doc: Arc::clone(&st.doc), revision: st.revision, base: app.save_settings(), own_quality: p.own_quality })
    }

    /// The encoder quality (1–100) OK writes for `quality` (0–12): the opened file's own when left
    /// on its step, as `confirm` keeps it.
    fn encoder_quality(&self, quality: u8) -> u8 {
        let own = self.own_quality.filter(|&q| super::quality_from_percent(u32::from(q)) == quality);
        own.unwrap_or_else(|| photocraft_engine::file_cmds::jpeg_quality(f64::from(quality)))
    }

    /// The settings OK writes with for `quality` (0–12) and `format`.
    fn settings(&self, quality: u8, format: JpegFormat) -> ExportSettings {
        ExportSettings { jpeg_quality: Some(self.encoder_quality(quality)), jpeg_format: format, ..self.base.clone() }
    }

    /// The readout for `quality` (0–12) and `format` (see [`Estimate::label`]).
    fn text(&self, ctx: &egui::Context, quality: u8, format: JpegFormat) -> String {
        if !estimable(self.doc.size) {
            return "—".into();
        }
        let key = Key { doc: self.doc.id, revision: self.revision, quality: self.encoder_quality(quality), format };
        let state = ctx.data_mut(|d| Arc::clone(d.get_temp_mut_or_default::<Arc<Mutex<Readout>>>(egui::Id::unique("jpeg-options-size"))));
        let mut readout = state.lock().unwrap_or_else(PoisonError::into_inner);
        let make = || {
            let (doc, settings) = (Arc::clone(&self.doc), self.settings(quality, format));
            move || encoded_size(&doc, &settings)
        };
        let repaint = ctx.clone();
        readout.update(key, make, move || repaint.request_repaint()).label()
    }
}

/// Draws the readout for the dialog's current `quality` (0–12) and `format`.
pub(super) fn show(ui: &mut egui::Ui, source: Option<&Source>, quality: u8, format: JpegFormat) {
    let text = source.map_or_else(|| "—".into(), |s| s.text(ui.ctx(), quality, format));
    let weak = ui.visuals().weak_text_color();
    ui.label(egui::RichText::new(text).color(weak)).on_hover_text(tl!("Estimated file size"));
}

#[cfg(test)]
mod tests;
