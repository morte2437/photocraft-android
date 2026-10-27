// ---------------------------------------------------------------- TIFF writer

/// A TIFF entry value.
#[derive(Debug, Clone)]
pub enum Val {
    Byte(Vec<u8>),
    Ascii(String),
    Short(Vec<u16>),
    Long(Vec<u32>),
    Double(Vec<f64>),
    Rational(Vec<(u32, u32)>),
    SRational(Vec<(i32, i32)>),
    Undefined(Vec<u8>),
    /// Offsets of these blobs (LONG).
    Blobs(Vec<usize>),
    /// Offsets of these IFDs (LONG), for SubIFDs / EXIF.
    Ifds(Vec<usize>),
    /// An UNDEFINED value whose bytes are this IFD (a maker note).
    IfdBytes(usize),
}

/// A TIFF file under construction.
#[derive(Debug, Clone, Default)]
pub struct TiffBuilder {
    pub big_endian: bool,
    /// Entries per IFD (sorted by tag on write).
    pub ifds: Vec<Vec<(u16, Val)>>,
    /// The main IFD chain, by index into `ifds`.
    pub chain: Vec<usize>,
    pub blobs: Vec<Vec<u8>>,
    /// Write a CR2 header pointing at this IFD as the raw IFD.
    pub cr2_raw_ifd: Option<usize>,
}

impl TiffBuilder {
    pub fn ifd(&mut self, entries: Vec<(u16, Val)>) -> usize {
        self.ifds.push(entries);
        self.ifds.len() - 1
    }

    pub fn blob(&mut self, data: Vec<u8>) -> usize {
        self.blobs.push(data);
        self.blobs.len() - 1
    }

    fn val_size(&self, v: &Val) -> usize {
        match v {
            Val::Byte(b) | Val::Undefined(b) => b.len(),
            Val::Ascii(s) => s.len() + 1,
            Val::Short(s) => 2 * s.len(),
            Val::Long(l) => 4 * l.len(),
            Val::Double(d) => 8 * d.len(),
            Val::Rational(r) => 8 * r.len(),
            Val::SRational(r) => 8 * r.len(),
            Val::Blobs(b) => 4 * b.len(),
            Val::Ifds(i) => 4 * i.len(),
            Val::IfdBytes(i) => self.ifd_size(*i),
        }
    }

    fn ifd_size(&self, i: usize) -> usize {
        2 + 12 * self.ifds[i].len() + 4
    }

    pub fn build(&self) -> Vec<u8> {
        let header = if self.cr2_raw_ifd.is_some() { 16 } else { 8 };
        // Layout: each IFD, then its overflow values; then the blobs.
        let mut pos = header;
        let mut ifd_pos = vec![0usize; self.ifds.len()];
        let mut val_pos: Vec<Vec<usize>> = vec![Vec::new(); self.ifds.len()];
        // Maker-note IFDs are written inside their owner's value area.
        let embedded: Vec<usize> = self.ifds.iter().flatten().filter_map(|(_, v)| if let Val::IfdBytes(i) = v { Some(*i) } else { None }).collect();
        for i in 0..self.ifds.len() {
            if embedded.contains(&i) {
                continue;
            }
            ifd_pos[i] = pos;
            pos += self.ifd_size(i);
            let mut vp = Vec::new();
            for (_, v) in self.sorted(i) {
                let size = self.val_size(&v);
                if size > 4 {
                    vp.push(pos);
                    if let Val::IfdBytes(e) = v {
                        ifd_pos[e] = pos;
                        // The embedded IFD's own values follow it.
                        let inner = pos + self.ifd_size(e);
                        let mut ip = Vec::new();
                        let mut q = inner;
                        for (_, iv) in self.sorted(e) {
                            let s = self.val_size(&iv);
                            if s > 4 {
                                ip.push(q);
                                q += s + (s & 1);
                            } else {
                                ip.push(0);
                            }
                        }
                        val_pos[e] = ip;
                        pos = q;
                    } else {
                        pos += size + (size & 1);
                    }
                } else {
                    vp.push(0);
                }
            }
            val_pos[i] = vp;
        }
        let mut blob_pos = Vec::new();
        for b in &self.blobs {
            blob_pos.push(pos);
            pos += b.len() + (b.len() & 1);
        }
        let mut out = vec![0u8; pos];
        let be = self.big_endian;
        let w16 = |o: &mut [u8], at: usize, v: u16| o[at..at + 2].copy_from_slice(&if be { v.to_be_bytes() } else { v.to_le_bytes() });
        let w32 = |o: &mut [u8], at: usize, v: u32| o[at..at + 4].copy_from_slice(&if be { v.to_be_bytes() } else { v.to_le_bytes() });
        out[0..4].copy_from_slice(if be { b"MM\0*" } else { b"II*\0" });
        w32(&mut out, 4, ifd_pos[self.chain[0]] as u32);
        if let Some(r) = self.cr2_raw_ifd {
            out[8..12].copy_from_slice(b"CR\x02\0");
            w32(&mut out, 12, ifd_pos[r] as u32);
        }
        for i in 0..self.ifds.len() {
            let at = ifd_pos[i];
            let entries = self.sorted(i);
            w16(&mut out, at, entries.len() as u16);
            for (k, (tag, v)) in entries.iter().enumerate() {
                let e = at + 2 + 12 * k;
                let (typ, count, bytes) = self.encode(v, &blob_pos, &ifd_pos);
                w16(&mut out, e, *tag);
                w16(&mut out, e + 2, typ);
                w32(&mut out, e + 4, count);
                if bytes.len() <= 4 {
                    out[e + 8..e + 8 + bytes.len()].copy_from_slice(&bytes);
                } else {
                    let vp = val_pos[i][k];
                    w32(&mut out, e + 8, vp as u32);
                    if !matches!(v, Val::IfdBytes(_)) {
                        out[vp..vp + bytes.len()].copy_from_slice(&bytes);
                    }
                }
            }
            let next = self.chain.iter().position(|&c| c == i).and_then(|p| self.chain.get(p + 1)).map(|&n| ifd_pos[n]).unwrap_or(0);
            w32(&mut out, at + 2 + 12 * entries.len(), next as u32);
        }
        for (b, &p) in self.blobs.iter().zip(&blob_pos) {
            out[p..p + b.len()].copy_from_slice(b);
        }
        out
    }

    fn sorted(&self, i: usize) -> Vec<(u16, Val)> {
        let mut e = self.ifds[i].clone();
        e.sort_by_key(|(t, _)| *t);
        e
    }

    fn encode(&self, v: &Val, blob_pos: &[usize], ifd_pos: &[usize]) -> (u16, u32, Vec<u8>) {
        let be = self.big_endian;
        let b16 = |v: u16| if be { v.to_be_bytes() } else { v.to_le_bytes() };
        let b32 = |v: u32| if be { v.to_be_bytes() } else { v.to_le_bytes() };
        let b64 = |v: u64| if be { v.to_be_bytes() } else { v.to_le_bytes() };
        match v {
            Val::Byte(b) => (1, b.len() as u32, b.clone()),
            Val::Undefined(b) => (7, b.len() as u32, b.clone()),
            Val::Ascii(s) => {
                let mut b = s.as_bytes().to_vec();
                b.push(0);
                (2, b.len() as u32, b)
            }
            Val::Short(s) => (3, s.len() as u32, s.iter().flat_map(|&v| b16(v)).collect()),
            Val::Long(l) => (4, l.len() as u32, l.iter().flat_map(|&v| b32(v)).collect()),
            Val::Double(d) => (12, d.len() as u32, d.iter().flat_map(|v| b64(v.to_bits())).collect()),
            Val::Rational(r) => (5, r.len() as u32, r.iter().flat_map(|&(n, d)| [b32(n), b32(d)].concat()).collect()),
            Val::SRational(r) => (10, r.len() as u32, r.iter().flat_map(|&(n, d)| [b32(n as u32), b32(d as u32)].concat()).collect()),
            Val::Blobs(b) => (4, b.len() as u32, b.iter().flat_map(|&i| b32(blob_pos[i] as u32)).collect()),
            Val::Ifds(i) => (4, i.len() as u32, i.iter().flat_map(|&k| b32(ifd_pos[k] as u32)).collect()),
            Val::IfdBytes(i) => (7, self.ifd_size(*i) as u32, vec![0; 8]),
        }
    }
}

pub(super) fn srat(v: f64) -> (i32, i32) {
    ((v * 10000.0).round() as i32, 10000)
}

pub(super) fn urat(v: f64) -> (u32, u32) {
    ((v * 10000.0).round() as u32, 10000)
}
