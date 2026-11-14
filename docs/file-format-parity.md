# File-format parity

> **Last reviewed:** 2026-10-12 · **Last updated:** 2026-11-08 · **Change:** minor (JPEG Options shows the size the file will have, FILE-215-19 partial; PDF: spot plates described in XMP for converters, document XMP embedded; PDF: spot channels as overprinting `/Separation` plates, a Multichannel document as plates alone, FILE-215-17's PDF part; Fujifilm compressed RAF (lossless and lossy, X-Trans and Bayer) decodes instead of falling back to the embedded JPEG preview, #7377; Camera Raw: an `.xmp` sidecar's crop crops the opened raw, and `crs:RawFileName` is no longer named as unsupported, #9985; CLI `--metadata none|all` for `convert`, `run --out` and `batch`: `none` leaves the document XMP and free-form text out of flat exports, as Export As's Metadata: None, `all` the default, #9466; Hasselblad 3FR/FFF, the X2D II 100C, decodes; scripted PSD/PSB saves take `maximizeCompatibility`, and the CLI `--no-maximize-compatibility`, #8064; TIFF Options on Save As and Save a Copy: Image Compression None/LZW/ZIP, Save Image Pyramid, Save Transparency and Discard Layers, also `tiffTransparency`/`tiffPyramid` for scripts, the CLI and MCP, #8827; `.atn`: 145 supported events including Shear after Actions batch 12 (Groups A–I, command options, version 12 files, reference forms); the supported table and measured reference are in [atn.md](atn.md); TIFF: YCbCr images open as RGB, including JPEG-compressed scans with subsampled chroma, #10316; TIFFs up to 300,000 px a side open, decoded a group of rows at a time into the layer, #8169; embedded AI/PDF/EPS save-back retains native format with a source-bound SVG display preview; PSD Filter Gallery: Halftone Pattern dot/circle/line settings survive PSD/PSB saves (#8940); Save for Web writes WebP, lossy or lossless, #6391; Photoshop's `MTrn` merged-transparency marker, from a layered TIFF saved with Save Transparency, marks the merged transparency, #8877; layered TIFF writes the TySh text version and descriptor in PSD big-endian order while the header, matrix, warp and bounds keep container order, #9098; layered TIFF writes Photoshop's 14-byte global layer mask, so Photoshop opens the layers instead of only the composite, #9095; legacy PSD point text placed from the layout ink box after font resolution, #6094; `.atn`: a recorded Save As imports with its format and file (`file.saveAs`, `file.saveACopy` for As a Copy) instead of as a plain Save that wrote over the played file, #7102; folder destinations stop playback until #5430; `.asl` layer style libraries read and written, #1051; JPEG Options on File › Save, Save As and Save a Copy, FILE-215-19 partial (no Matte, progressive Scans or size preview); 32-bit Hue/Saturation unclipped and Colorize as Photoshop does them, Photoshop-oracle floor 190/258, #5781; JPEG Options on Save As and Save a Copy, FILE-215-19; TIFF compression for scripted saves, `tiffCompression`, #5301; the Nikon D4 white level follows the sample depth (12 and 14 bit), #5978; Apply Layer Comp action mapping, #7101; Cineon write: 10-bit gray/RGB; animated WebP frames load into video layers, #1198; Photoshop mixed-order TIFF type geometry, #4758; DDS textures save: uncompressed or BC1–BC3, optional mip chain, DDS Options in Save As (#1327); PSD smart filters: 53 filters (39 more: Pinch, Polar Coordinates, Ripple, Spherize, Twirl, Wave, ZigZag, Invert, Blur, Surface Blur, Smart Sharpen, Diffuse, Wind, Offset, Mezzotint, Clouds, Difference Clouds, Fibers, Lens Flare and others) plus Hue/Saturation and Perspective Warp mapped both ways, #6296, #6298, #6300, #6302, #6304, #6314, #6316, #6267, #6268, FILE-215-2; large-file fixtures: a 20 MP layered PSD, a 150-layer layout, a 2 GB PSD and a 4.4 GB PSB generated at test time and round-tripped, FILE-216-2 done, #5107; earlier: smart objects keep Photoshop's Cylinder warp through re-renders and saves, #4112; PNG `sTER` stereo pairing kept on save, #6063; PDF write one flattened page via Save As / Save a Copy / CLI, #3401; CMYK Invert, Posterize and Threshold on the inks; smart filter masks on PSD open and save; Photoshop-oracle floor 188/258; lossy DNG, compression 34892, decodes with its levels and LinearizationTable applied to the stored 8-bit samples, FILE-215-11; CMYK Hue/Saturation and Color Balance keep the black ink; CMYK Posterize on the stored values, psd-tools oracle floor 242/309; CMYK adjustment layers composite on the document's inks; Photoshop-oracle floor 165/258; CMYK Brightness/Contrast on the inks, CMYK Threshold's luma; modern Brightness/Contrast's brightness curve; Photoshop-oracle floor 151/258; Lab documents composite on their stored L, a, b; Photoshop-oracle floor 152/258; Levels on whole levels at every step with Photoshop's shadow toe; Photoshop-oracle floor 146/258, psd-tools 238/309; 32-bit adjustments unclipped; Photoshop-oracle floor 149/258; mask feathers blur with their exact variance; Threshold splits on Photoshop's luminosity; a pass-through clipping base with an adjustment renders isolated; psd-tools oracle floor 240/309; 16-bit Posterize as Photoshop bins it; psd-tools oracle floor 241/309; Gradient Map luminosity; Photoshop-oracle floor 147/258; PSD: `Mt16`/`Mt32` merged transparency read; palette PNGs open in Indexed Color mode, #4804; 16-bit Lab Invert; Photoshop-oracle floor 145/258; `.atn` import maps 112 events; DDS textures open, #1327; DPX read and write, #1049; #2465 fixed: CS2 placed layers with only a `PlLd` block open their contents; Photoshop Actions `.atn` read and write; `.shc` contour files read and write; earlier the same day: indexed BMP palette mode; Save keeps a JPEG's quality and a WebP's encoding; Amiga IFF write: 24-plane ILBM, Save As `.iff`; PSD stores spot channels as Photoshop does, ink 0 = solid, and numbers them in 1053, #7415 split; TIFF writes and reads alpha and spot channels as extra samples with Photoshop's channel resources, PSD output unchanged, FILE-215-7, FILE-215-17's TIFF part, #7415; Photoshop's `MTrn` merged-transparency marker, from a layered TIFF saved with Save Transparency, marks the merged transparency) · **Target:** Adobe Photoshop 2026

**PSD Filter Gallery (#8940, 2026-10-23):** Halftone Pattern maps its size, contrast and dot/circle/line pattern as editable settings on import and export, with the reference application's descriptor keys (`HlSz`, `Cntr`, `ScrT`: `ScrD`/`ScrC`/`ScrL`). PSD/PSB round trips pass at 8/16/32 bits. This does not claim pixel equivalence for the existing renderer. Unknown or malformed settings remain verbatim on import.

Every format Photoshop 2026 (27.11.0) reads or writes, with PhotoCraft's support, fidelity and
tests. Photoshop's list comes from its `Info.plist` `CFBundleDocumentTypes`, the PiPL read/write
flags of `Contents/PlugIns/Required/File Formats`, and format names in the main binary (Open/Save
As behaviour from Adobe's documentation where the bundle doesn't say). Ours comes from
`crates/codecs/src/format.rs` (capability table), `crates/io/src/lib.rs` (import/export
dispatch), `crates/psd`, `crates/raw`, `crates/heif`, `crates/affinity`. More detail on our side:
[raster formats](../book/src/formats/raster-formats.md), [PSD/PSB](../book/src/formats/psd-psb.md),
[OpenRaster](ora.md), [Paint.NET](pdn.md).

**Summary.** By count, 13 of Photoshop's 31 formats are fully supported in the directions Photoshop supports them, 4 partially (48%,
measured). Weighted by use (PSD/PSB 40%, JPEG 15%, PNG 15%, TIFF 10%, camera raw 8%, WebP/AVIF/
HEIF/JXL 6%, PDF 3%, everything else 3%), presence is ~80% and ready-for-real-work ~60%: PSD
is broad but not yet reliable against Photoshop (gap G1), and Photoshop PDF, AVIF read and JPEG
XL are missing (gap G10). Remaining: 300–500 h including PSD fidelity.

## Photoshop's formats

| Format | Photoshop | PhotoCraft read | PhotoCraft write | Fidelity and tests |
|---|---|---|---|---|
| PSD | R/W | yes | yes | 1/8/16/32-bit; Bitmap, Gray, Indexed, RGB, CMYK, Multichannel, Lab, Duotone; layers, groups, masks, vector masks, adjustment and fill layers, effects, text, shapes, smart objects (SoLd/lnk2 with their warp: the bend presets, a custom mesh and, since #4112, Cylinder; CS2 PlLd/lnkD read with their warp, kept as is or re-saved as SoLd once edited), smart filters (53 mapped, among them Invert, the Distort filters but Shear, Clouds, Difference Clouds, Fibers and Lens Flare, plus Hue/Saturation, Perspective Warp, Filter Gallery with Chrome and a Camera Raw subset, rest verbatim), slices, comps, Advanced Blending, each layer's XMP packet (`layer.xmpMetadata`); alpha and spot channels as Photoshop stores them (spot ink 0 = solid, channel identifiers in 1053), and PSDs with spot channels from PhotoCraft 0.1.0–0.16.0, which stored the ink the other way round, keep their ink. Oracle floors: Photoshop-authored 190/258, io 148/170, psd-tools 249/309; round trip ≥ 169/170, 308/309, 258/258. Known failures: #1281, #2469 (#2374, text layers, closed 2026-10-10; #2465, CS2 `PlLd`-only smart objects, fixed 2026-10-12) |
| PSB | R/W | yes | yes | Auto-switches above 2 GB (REL-212-2); generated 20 MP, 150-layer, 2 GB PSD and 4.4 GB PSB fixtures opened, saved and re-opened by io tests/it/large_files.rs (FILE-216-2, 2026-10-13; the 4.4 GB PSB opens on Windows); > 4 GB from the app's File › Open on Windows: #375 |
| Cloud PSD (.psdc) | R/W | no | no | Out of scope (Adobe cloud) |
| TIFF | R/W | yes | yes | 8/16/32f, Gray/RGB/CMYK, YCbCr read as RGB (JPEG-compressed with subsampled chroma, or uncompressed without subsampling; #10316), BigTIFF, every IFD, banded parallel decode; flat pages up to 300,000 px a side open a group of rows at a time into the layer, with no full-size copy, and are refused with an error when the machine lacks the memory (Linux: free RAM and swap, cgroup limits; #8169); layered TIFF both byte orders, including Photoshop mixed-order type blocks on read (#4758/#4771) and TySh text descriptors written in PSD order (#9098) and Photoshop's 14-byte `LMsk` block (#9095); TIFF Options on Save As and Save a Copy (Image Compression None/LZW/ZIP, Save Image Pyramid, Save Transparency, Discard Layers and Save a Copy; scripts pass `tiffCompression`, `tiffTransparency`, `tiffPyramid`, `tiffLayers`; #8827), no JPEG compression, Pixel Order or Byte Order (the encoder writes interleaved samples in the machine's byte order); undecodable type keeps saved pixels (FILE-215-7); alpha and spot channels as extra samples (at most 16) with Photoshop's names, ink colours and solidity, spot ink stored 0 = solid (#7415); Save Transparency on keeps the transparency as the first extra sample ahead of the channels (a layered file marks it with `MTrn`, as Photoshop does), off mattes on white; unset, it is kept only without alpha or spot channels |
| JPEG | R/W | yes | yes | 8-bit Gray/RGB/CMYK, ICC/EXIF/XMP (the CLI's `--metadata none` leaves the document XMP out, #9466); Save keeps the opened file's quality (estimated from its quantisation tables, #2399); File › Save, Save As and Save a Copy ask JPEG Options: Quality 0–12, starting at the opened file's quality or else the Export preference, and Baseline ("Standard") / Baseline Optimized / Progressive; scripts pass `quality`/`jpegFormat`, the CLI `--jpeg-format` (FILE-215-19). Baseline Optimized fits the Huffman tables only at 4:4:4 (Gray, CMYK) or progressive: RGB is written 4:2:0, which zune-jpeg 0.5.15 decodes wrongly with fitted tables, as in Save for Web. The dialog shows the size the file will have ("152.3K", "1.5M"), encoded in memory as OK would write it, on a worker thread in the desktop app and in the frame on the web, for documents up to 40 MP (4 MP on the web; larger ones read "—"). No Matte or progressive Scans (3–5) |
| PNG | R/W | yes | yes | 8/16-bit, ICC/text (the CLI's `--metadata none` leaves the document XMP and text out, #9466); APNG first frame only; alpha channels on open (#2225); palette PNGs open in Indexed Color mode with their transparent entry (#4804); Save keeps a stereo PNG's `sTER` pairing (#6063) |
| GIF | R/W | yes | yes | File › Open keeps the first frame; New Video Layer from File and Video Frames to Layers take every frame, with the timeline rate from the delays (#572). Per-frame delays and keeping the frames in `.pcraft` are not done |
| BMP | R/W | yes | yes | 8-bit; palette BMPs open in Indexed Color mode and Indexed Color documents save as palette BMPs (no transparency) |
| WebP | R/W | yes | yes | Lossless and our own lossy VP8 encoder; ICC/EXIF/XMP (the CLI's `--metadata none` leaves the document XMP out, #9466); Save keeps an opened file lossy (at its quality) or lossless (#2399); Save for Web (Legacy) writes it too, lossy at its Quality or lossless, a format Photoshop's legacy dialog lacks (#6391). File › Open keeps an animation's first frame; New Video Layer from File and Video Frames to Layers take every frame, with the timeline rate from the durations (#1198) |
| AVIF | R/W | no (#463) | optional feature, off by default | |
| JPEG XL | R/W | inside DNG only | no | |
| HEIF/HEIC | R/W(HEIF) | yes (pure-Rust heic-rs, `heif` feature, on in releases) | no | 8/16-bit |
| OpenEXR | R/W | yes | yes | f16/f32; deep EXR and Cryptomatte readers (`corpus/exr`) |
| Radiance HDR | R/W | yes | yes | f32 |
| Portable Bit Map (PBM/PGM/PPM/PNM/PFM/PAM) | R/W | yes | yes | 8/16/32f |
| FITS | R/W (FITS Liberator plug-in) | yes | yes | 8/16/32f gray or RGB, header cards kept; no fpack, no debayer |
| Targa | R/W | yes | yes | 8-bit |
| Photoshop PDF | R/W | yes, raster pages (craft-pdf; up to 16 pages, 8192 px per side, 64M px per page) | yes, one flattened page (Save As / Save a Copy / CLI, #3401; also print and artboards-to-PDF); spot channels as overprinting `/Separation` plates after the image, a Multichannel document as its plates alone (FILE-215-17's PDF part) | Read renders each page as a raster artboard: no editable text/vector layers yet (craft-pdf `interpret` is the follow-up), no PDF forms or annotations; write is still one flattened page, no multi-page save, no PDF presentation, layers are not preserved (FILE-215-10). Plates are 8-bit, named after the channel, 1 = solid ink, painted in channel order with `/OP true`; an artboard page carries its own region of each plate; the tint transform's alternate is the ink colour separated to CMYK (preview only); solidity does not change the plate image or tint transform and is carried only in the XMP description; no printer marks, labels or halftone screens, as Photoshop's PDF has none either (scripts: `pdfSpotPlates: false` leaves the plates out and refuses a Multichannel document, which has nothing else to write); each plate's ink colour (Lab D50, sRGB) and solidity are described in the catalog's XMP packet (see Spot-plate PDF metadata below), and the document's own XMP travels in the same packet. Ghostscript `tiffsep`/`psdcmyk` split a page into the named plates |
| Photoshop EPS, DCS 1.0/2.0, EPS TIFF/PICT preview | R/W | no | no | A spot-plate PDF (above) carries what a DCS 2.0 file carried to a RIP: one named plate per spot channel |
| Generic EPS / AI | R (rasterize) | no | no | |
| SVG | R (rasterize) | yes, as editable shape layers or a vector smart object | no | Better than Photoshop for editing; gradients approximated (FILE-215-9) |
| JPEG 2000 | R/W | no (#1047) | no | |
| Cineon / DPX | R (DPX), R/W (CIN) | DPX: luma, RGB, RGBA at 8/10/12/16 bits, both byte orders (#1049); Cineon: 10-bit gray/RGB | DPX: 8/10/12/16 bits (`dpxBits`, `--dpx-bits`); Cineon: 10-bit log gray/RGB (Save As `.cin`) | Writes DPX, which Photoshop only reads; header fields (creator, project, copyright, transfer, frame rate, time code…) round-trip; a Cineon frame saved as Cineon keeps its 10-bit code values; no RLE |
| DICOM | R/W | yes (#1048) | no | Read-only, uncompressed syntaxes: MONOCHROME1/2, RGB, YBR_FULL |
| PCX | R/W | no | no | |
| Pixar | R/W | no | no | |
| Amiga IFF | R/W | yes | yes (Save As `.iff`) | Read: 1–8 planes with CMAP and 24-plane RGB, stored and ByteRun1; write: 24-plane ILBM, ByteRun1, alpha as a 1-bit mask plane, 8-bit only |
| Wireless Bitmap | R/W | no | no | |
| MPO / JPS (stereo) | R/W | no | no | |
| PICT | R | no | no | Legacy |
| Scitex CT | R/W | no | no | Legacy |
| Photoshop Raw (.raw) | R/W | no | no | |
| Camera raw (via Camera Raw 18.7) | R | partial | – | See below |

## Camera raw

Photoshop opens raws through Camera Raw 18.7 (the bundle registers cr2, crw, dcr, dng, erf, mos,
fff, 3fr, mef, mfw, iiq, nrw, rwl, srw, mrw, nef, orf, pef, raf, srf, x3f; Camera Raw itself
supports many more, including CR3).

| Camera format | PhotoCraft | Evidence |
|---|---|---|
| DNG (incl. LJPEG, lossy JPEG and JPEG XL tiles) | yes | 2026-10-13: lossy DNG (DNG 1.4 compression 34892: 8-bit baseline JPEG tiles or strips, CFA and LinearRaw, partial edge tiles) decodes to the stored samples, which go through LinearizationTable / BlackLevel / WhiteLevel (default 255) like every other depth; `crates/raw/tests/it/dng.rs` `lossy_jpeg_*` (FILE-215-11, lossy DNG part; no lossy file in the raw corpus yet) |
| Canon CR2 | yes | |
| Canon CR3 | detected, unsupported | `crates/raw/src/lib.rs:219` |
| Nikon NEF/NRW (lossless, lossy) | yes, including "lossy after split" (2026-10-31: the 11 CC0 raw.pixls.us split files, D3400 / D5 / D5000 / D5200 / D5500 / D5600 / D7000 / 1 J5 / Z 6_2) | #50; D5100 and D90 image areas measured (14-bit NEFs); D4 profile levels follow the sample depth (12 and 14 bit); [validation](nikon-nef-development.md) |
| Sony ARW/SR2/SRF (incl. compressed) | yes | |
| Panasonic RW2, Pentax PEF | yes | |
| Olympus ORF | uncompressed only | |
| Fujifilm RAF (Bayer and X-Trans) | uncompressed, lossless and lossy compressed; no Fuji colour calibration | `crates/raw/tests/it/raf.rs` |
| Hasselblad 3FR/FFF | uncompressed only; compressed (lossless-JPEG) files: embedded preview only | X2D II 100C uses a measured Bayer phase; other bodies assume it with a warning (`crates/raw/src/hasselblad.rs`) |
| Others (CRW, IIQ, X3F, SRW, MRW, ERF…) | embedded preview only | |
| .xmp sidecars, highlight modes, raw preferences | partial (sidecars and preferences; no highlight modes) | 2026-10-28: a sidecar's crop (`crs:HasCrop` with `CropTop` / `CropLeft` / `CropBottom` / `CropRight`, fractions of the developed frame as stored, mapped through its Orientation) crops the opened raw: Open re-develops and crops before the Camera Raw step, so the post-crop vignette centres on the crop; the dialog previews the whole frame. A straightened crop (`CropAngle` not 0) isn't applied and is named, as are unreadable or empty ones; `HasCrop` missing or `False` means no crop; `RawFileName` is bookkeeping (#9985; `crates/io/src/camera_raw_xmp.rs` tests, `crates/io/tests/it/raw.rs` `camera_raw_tuning_crops_in_the_stored_orientation`, `camera_raw_ui/tests.rs` `a_sidecar_crop_crops_the_opened_raw`). 2026-10-13: sidecars fill the open-time dialog and Open writes them back; Camera Raw Defaults wired. FILE-215-11 |
| Camera Raw as a PSD smart filter | partial | 2026-10-07: WB, Light/Presence, curves, HSL, grading, detail, grain, vignette mapped (FILE-215-12) |

2026-10-11: on #2135's embedded-JPEG rendering work, open-time Camera Raw tuning
preserves the fitted rendering while changing Exposure, Temperature and Tint. Default
tuned import matches normal import exactly; output remains full-resolution 16-bit ProPhoto.
This improves appearance when a usable preview is present, without adding a measured
D5100 colour profile. Counts and the G14 estimate are unchanged.

## Formats Photoshop doesn't have (ours only)

| Format | Read | Write | Notes |
|---|---|---|---|
| `.pcraft` (native) | yes | yes | Incremental, autosave, crash recovery, atomic save |
| Affinity `.af`, `.afphoto`, `.afdesign`, `.afpub` | yes | no | See below |
| Paint.NET `.pdn` | yes | no | [pdn.md](pdn.md) |
| OpenRaster `.ora` | yes | yes | [ora.md](ora.md) |
| ICO, QOI | yes | yes | |
| DirectDraw Surface `.dds` | yes | yes | Opens the top mip level of a 2D texture: uncompressed RGB, RGBA or luminance, BC1–BC3 (DXT1/3/5), legacy or DX10 header. Saves a 2D texture uncompressed (BGRA, BGRX, luminance with or without alpha) or BC1–BC3 with the legacy header, optionally with its full mip chain (box filter); Save As and Save a Copy ask (DDS Options), scripts pass `ddsCompression` and `ddsMipmaps` (`--dds-compression`, `--dds-mipmaps` on the CLI). Cubemaps, arrays, volumes and BC4–BC7 are refused (#1327). Photoshop needs a plug-in |
| Krita `.kra`, GIMP `.xcf`, Procreate | no | no | Not in Photoshop either |

### Affinity import notes (moved from `roadmap.md`)

2026-10-08: Affinity `.af`, `.afdesign`, `.afphoto` and `.afpub` documents (container versions
8–12) open **natively** ([format notes](../crates/affinity/README.md)): pages and artboards, layers
and groups, curves and geometric shapes with fills, gradients and strokes, text as type layers,
placed images as smart objects, pixel layers and masks, as an 8-bit RGB document without a source
save path. Measured against the thumbnail Affinity embeds in each file, 21 pinned public documents
(`cargo xtask corpus --affinity`) differ by 0–4.6 of 255 on average. Layer effects, adjustments,
live filters, brush strokes, special shapes, master pages and CMYK/Lab/16-bit document colour are
approximated or left out, each with a warning; damaged or unknown files fall back to the embedded
preview. Affinity writing is not implemented: no Affinity installation was available to check
written files, so `.af` export stays unsupported.

2026-10-10: current `.af` artboard properties (`phrp`/`aprp`) are recognized alongside the
legacy flag, including converted curve boards. Synthetic two-board regressions check separate
bounds, overflow clipping, `.pcraft`/PSD board and pixel round trips, and moving one board with its
children through undo/redo. Nested boards remain masked groups; rotated/curved board outlines
still import at their bounding rectangle.

2026-10-09: `corpus/affinity/` now has 39 pinned documents and 21 PNGs (20 rendered references and
one bitmap-fill texture): the prior 21 public documents plus 18 CC0 samples for #1606. The new
samples compare against their exported PNGs;
mean differences range from 0.44/255 (conical gradients) to 86.05/255 (RGB/32 reduced to 8-bit).
Clouds, hearts, cogs, callouts, arrows, double stars, tears, crescents, diamonds and circular segments
now import as parametric shapes; the special-shapes sample measures 5.68/255 against its PNG. The
remaining gaps in #1606 stay tracked with per-sample corpus ceilings.

## Presets and data files

| Format | Photoshop | PhotoCraft | Notes |
|---|---|---|---|
| Brushes `.abr` | R/W | R/W | `psd/src/abr.rs` (v6/v12) |
| Gradients `.grd` | R/W | R/W | |
| Patterns `.pat` | R/W | R/W | |
| Swatches `.aco`, `.act`, `.ase` | R/W | R/W | FILE-215-14; `.act` also from Image › Mode › Color Table Load…/Save… (`psd/src/act.rs`) |
| LUTs `.cube` / `.3dl` / `.look` | R/W (Export Color Lookup) | R/W / R / R | `craft-cms` `src/lutfile.rs` |
| Keyboard shortcuts `.kys` | R/W | R/W | `engine/src/kys.rs` (`write`: Edit › Keyboard Shortcuts › Export…, rows carry our id; Photoshop reading our files is unverified) |
| ICC profiles | R | R | Own CMS |
| Actions `.atn` ([reference](atn.md)) | R/W | R/W | `psd/src/atn.rs` reads version 12 and 16 sets and writes version 16 (exact round trip for version 16; version 12 legacy bare identifiers, reference classes and object-array values keep their meaning; trailing bytes remain a warning, #10699). `engine/src/cmd/atn_cmds/` maps the reference application's events to commands (supported event IDs listed in [atn.md](atn.md); one recognized-but-disabled event), including Apply Layer Comp by name (#7101), recorded layer indexes/ordinals and indexed moves, opening-order document indexes and comp index/ID/ordinal references (#10701), recorded classic `GrMp` with fractional RGB stops and `RplC` with explicit Lab sample bounds (#10607); other steps kept verbatim. Save As maps to `file.saveAs`/`file.saveACopy` for a file destination (#7102). Panel menu, CLI `batch --actions set.atn`, `actions.load`/`actions.save`. Version 7 and MacBinary wrappers remain unsupported; version 12 coverage uses synthetic fixtures. |
| Styles `.asl` | R/W | R/W | `psd/src/asl.rs`, `io/src/asl_map.rs`; Styles panel Import/Export Styles…, `style.presets.importAsl`/`exportAsl` (#1051) |
| Contours `.shc` | R/W | R/W | `psd/src/shc.rs`; Contour Editor Load…/Save… (FILE-215-15). No user contour library; not yet checked against a Photoshop-written file |
| Custom shapes `.csh`, tool presets `.tpl` | R/W | no | |
| Curves `.acv`, Levels `.alv`, other adjustment presets | R/W | not audited | |
| C2PA Content Credentials | R/W | no | #1050 |

## Spot-plate PDF metadata

A PDF with spot plates (see Photoshop PDF above) carries, in the catalog's `/Metadata` XMP packet
(the document's own packet when it has one), one `rdf:Description` under the namespace
`http://ns.getartcraft.com/photocraft/spot-plates/1.0/` (prefix `pcsp`), so a converter can
rebuild each spot channel with its real ink colour, which the `/Separation` colour space's CMYK
alternate cannot carry. `pcsp:plates` is an `rdf:Seq` in paint order, one `rdf:li`
(`rdf:parseType="Resource"`) per plate:

| Property | Value |
|---|---|
| `pcsp:index` | 0-based position in the sequence, the order the plates are painted (the first is underneath) |
| `pcsp:name` | the channel name, exactly the Separation colorant name (XML-escaped, UTF-8) |
| `pcsp:labL`, `pcsp:labA`, `pcsp:labB` | the ink colour as CIE L\*a\*b\*, D50, two decimals |
| `pcsp:srgb` | the same colour as encoded sRGB, `#RRGGBB` |
| `pcsp:solidity` | 0–100, Photoshop's Spot Channel Options › Solidity (0 for an alpha-type channel of a Multichannel document, which lists as black) |

The packet is unfiltered, so it can be found by scanning for `<?xpacket begin`. Readers such as
pdf.js, pdf-lib, pikepdf and exiftool expose it as the document XMP. The plates themselves are
unchanged by the description; it is PDF 1.4 metadata that PDF/X-1a allows.

## Embedded vectors in PSD smart objects (2026-10-22)

Embedded SVG and PDF-compatible AI/PDF/EPS payloads can use an explicit VectorCraft editor choice, including nested PSB contents. AI/PDF/EPS working copies retain their format on save-back; a separate source-bound SVG display preview does not replace the native payload. Placement, shared instances, masks and Undo/Redo use existing smart-object transactions, with native bytes and previews persisted through .pcraft and PSD/PSB saves. Unsupported editor features may still change; standalone EPS/DCS import and lossless Adobe round-trip compatibility remain unverified. See [workflow, safety and validation limits](embedded-vector-editing.md). No corpus floor or parity percentage is changed.

## Revision history

The original reference row retains its source date and historical totals; current totals are in the status and Actions row above.
The original reference entry retains its source date and historical totals; current totals are in the status above.

| Date | Change | Summary |
|---|---|---|
| 2026-11-07 | minor | Fujifilm compressed RAF (lossless and lossy, X-Trans and Bayer) decodes instead of falling back to the embedded JPEG preview (#7377) |
| 2026-11-07 | minor | Photoshop PDF read: pages import as raster artboards through the shared craft-pdf (Hayro) renderer, up to 16 pages, 8192 px per side and 64M px per page; editable text/vectors and PDFium are gone. Write is unchanged. |
| 2026-11-06 | minor | Scripted PSD/PSB saves take `maximizeCompatibility` (Save, Save As, Save a Copy, `app.save`, serve `doc.save`, MCP `doc_save`/`doc_export`, Batch, Image Processor, Layers to Files) over Preferences › File Handling for one save, and the CLI `--no-maximize-compatibility` (#8064). Layer channels and the merged image were already RLE (ZIP with prediction at 32-bit), as Photoshop writes them (#8645) |
| 2026-10-24 | minor | Hasselblad 3FR/FFF: the X2D II 100C's 16-bit uncompressed CFA decodes; compressed 3FR still opens as the embedded preview |
| 2026-11-02 | minor | Actions Shear (`Shr `) loads, plays and saves the measured curve points and edge mode; unsafe subranges and unrepresentable forms remain kept steps (#10605). |
| 2026-11-03 | minor | CLI `--metadata` (`none` or `all`) for `convert`, `run --out` and `batch`: `none` leaves the document's XMP packet and free-form text out of flat exports (PNG, JPEG, WebP, TIFF, …) through Export As's Metadata: None path; `all`, which keeps them as Save As does, stays the default. The ICC profile, EXIF and resolution are written as they are without the option (a palette PNG carries only its resolution, either way); PSD, PSB and `.pcraft` always keep the XMP (#9466) |
| 2026-11-03 | minor | Camera Raw `.xmp` sidecars: the crop (`crs:HasCrop` with `CropTop` / `CropLeft` / `CropBottom` / `CropRight`, fractions of the raw as stored, mapped through its Orientation) crops the opened raw, with the dialog's Open and without the dialog; out-of-range fractions are clamped, and inverted, unreadable or straightened (`CropAngle` not 0) crops are named as not supported instead of applied; PhotoCraft's own sidecar keeps the crop when it is rewritten; `crs:RawFileName` is no longer named as unsupported (#9985) |
| 2026-11-01 | minor | `.atn`: 23 newer measured forms mapped, including explicit Link/Unlink, Smart Object file operations, Make Type to Shape, radius-only Shape Blur, Artboard from Group and the Crop Photos parent; supported in batch 13; Link Layers keeps the measured toggle. Artboard forms with unmeasured settings and generated Crop helper steps remain kept. |
| 2026-11-01 | minor | Recorded direct Gradient Map and Lab-range Replace Color load, play and retain their measured settings on save; unsupported forms remain preserved (part of #10607). |
| 2026-11-02 | minor | TIFF Options on Save As and Save a Copy (#8827): Image Compression (None, LZW, ZIP), Save Image Pyramid (half-size levels marked reduced-resolution, down to 64 px, alpha-weighted; +45 ms on a 30 MP RGB/8 save), Save Transparency (on: transparency kept ahead of alpha and spot channels, `MTrn` in a layered file; off: matted on white) and Discard Layers and Save a Copy. Scripts, `doc.save`, MCP `save` and the CLI (`--tiff-transparency`, `--tiff-pyramid`) take the same answers. JPEG compression, Pixel Order and Byte Order not offered |
| 2026-10-30 | minor | PDF: the spot plates are described in the catalog's XMP packet (`pcsp` namespace: index, name, Lab D50, sRGB, solidity per plate), merged into the document's own packet, so a converter can rebuild the spot channels with their real ink colours; a document's XMP now travels into its PDF as into the other flat formats (`XmpEmbed`) |
| 2026-10-29 | minor | PDF: spot channels are written as overprinting `/Separation` plates after the flattened image (one 8-bit Flate image XObject per channel, named after it, 1 = solid ink, `/OP true`, channel order, DeviceCMYK alternate from the ink colour), and a Multichannel document saves as its plates alone; `pdfSpotPlates: false` on scripted saves writes the image only. FILE-215-17's PDF Separation part; its RGB/CMYK spot preview stays open |
| 2026-11-01 | minor | `.atn`: event reference [atn.md](atn.md): the 117 supported events, and measured keys, defaults, ranges and enum values for 101 events not mapped yet |
| 2026-10-31 | minor | `.atn` Group B: four Render events load, play and save; 120 supported and 97 unmapped. Supported-table columns corrected against the event registry; the original 117/101 reference entry remains historical. |
| 2026-10-31 | minor | `.atn`: Pinch (`Pnch`), Polar Coordinates (`Plr `), Spherize (`Sphr`), Twirl (`Twrl`), Wave (`Wave`) and ZigZag (`ZgZg`) map to `filter.distort.*` with every key in recorded order; out-of-range values clamped and missing keys given Photoshop's defaults on import; a Wave scaled below 100 % stays a Photoshop step (the command has no scale); Shear still kept verbatim (#10605) |
| 2026-10-31 | minor | `.atn`: De-Interlace (`Dntr`), NTSC Colors (`NTSC`), HSB/HSL (`HsbP`), Custom (`Cstm`, a 5 × 5 `Mtrx`), Average (`Avrg`) and Remove Black / White Matte (`RmvB`/`RmvW`) map to their commands with Photoshop's defaults and save back with the same keys; Replace Color and the direct Gradient Map (`GrMp`) still kept verbatim (#10607) |
| 2026-11-01 | minor | `.atn` version 12 import uses the existing descriptor model and limits, preserves trailing-byte warnings and writes version 16. Synthetic fixtures cover legacy values, references, truncations and command-import equivalence (#10699). |
| 2026-10-31 | minor | `.atn`: Oil Paint (`oilPaint`), Smart Sharpen (`smartSharpen`), Focus Area (`focusMask`), Select Sky (`selectSky`) and Remove Background (`removeBackground`) map to their commands with the keys and defaults Photoshop records (Oil Paint's and Select Sky's defaults written out where they differ from the commands'); Smart Sharpen maps the newer filter's Gaussian form, as PSD smart filters do, and keeps the legacy filter, Lens Blur and Motion Blur as Photoshop steps; out-of-range Oil Paint and Smart Sharpen settings are clamped on import (#10610) |
| 2026-11-01 | minor | `.atn`: layer indexes/ordinals include collapsed children and group ends; indexed moves use pre-move boundaries; document indexes use opening order; comp index/ID/ordinal forms retain their reference kind (#10701). |
| 2026-11-01 | minor | `.atn`: original reference snapshot, before the Group F mappings: [atn.md](atn.md) documented 117 supported events and measured keys, defaults, ranges and enum values for 101 events not mapped yet |
| 2026-10-31 | minor | `.atn` Group F: selection combinations, Paste Into/Outside, Fade, Link, Distribute, Define Pattern and older mask/merge events load, play and save; 127 supported event IDs, 89 unmapped reference entries; AUTO-219-2 remains partial |
| 2026-10-31 | minor | `.atn` Group D: twelve Stylize/Pixelate events load, play and save; 128 supported and 89 unmapped. Supported-table columns corrected against the event registry; the original 117/101 reference entry remains historical. |
| 2026-10-31 | minor | `.atn`: Split Channels (`SplC`, its recorded `documentIDs` not replayed or written), Merge Spot Channel (`MSpt`), Trap (`Trap`, `Wdth` clamped to 32767 px) and Match Color without a source (`matchColor`, `noReference`) map to `channel.split`, `channel.mergeSpot`, `image.trap` and `image.adjustments.matchColor`; a Match Color step that matches another image stays a Photoshop step (#10609) |
| 2026-10-31 | minor | `.atn`: Filter Gallery (`GEfc` with the effect in `GEfk`, or the effect's own event) maps all 47 effects to `filter.gallery.<effect>` with every key in recorded order, out-of-range values clamped and missing keys given Photoshop's defaults (was Spatter only); Glass maps the Frosted and Canvas textures (#10592) |
| 2026-10-30 | patch | TIFF: YCbCr (PhotometricInterpretation 6) opens as RGB instead of failing with "colour type YCbCr(8)": JPEG-compressed with subsampled chroma (as scanners write it) through the JPEG conversion, uncompressed with ReferenceBlackWhite and YCbCrCoefficients; 8 and 16-bit. Uncompressed subsampled YCbCr and YCbCr with extra samples still fail with a clear error (#10316) |
| 2026-10-28 | minor | JPEG Options shows the size the file will have (FILE-215-19 partial: no Matte or progressive Scans) |
| 2026-10-26 | minor | Embedded AI/PDF/EPS save-back retains native format and a separate source-bound display preview; original sources remain undoable. Native Adobe interoperability and standalone EPS/DCS remain unverified; no corpus floor or percentage changed. |
| 2026-10-22 | minor | Embedded PSD vectors gain explicit bounded SVG editor/save-back; standalone EPS/DCS and native-format fidelity remain open. |
| 2026-10-22 | minor | TIFF: flat pages up to 300,000 px a side open (was 2^30 px, about 32,768²): `codecs::TiffBands` decodes a group of rows at a time into the layer's tiles. The 117,504 × 22,464 px banner from #8456 opens in 3.3 s at 11.45 GB peak for a 10.56 GB layer; 30,000² 6.8 s / 6.43 GB → 1.1 s / 3.86 GB; 36 MP 220 → 52 ms. Layered TIFFs still decode whole (#8169) |
| 2026-10-24 | minor | Layered TIFF: the `LMsk` block in tag 37724 carries Photoshop's default global layer mask (14 bytes) instead of none; with an empty block Photoshop read only the composite (#9095). PSD output unchanged |
| 2026-10-24 | minor | Layered TIFF writes the TySh text version and descriptor in PSD big-endian order while retaining container order for the block header, transform matrix, warp section and bounds (#9098); the mixed-order read path remains covered by #4758/#4771. |
| 2026-10-23 | minor | PSD: spot channels are stored and read as Photoshop 27.11's PSDs store them (ink 0 = solid; all 7 of its spot PSDs store the 100/25/75/0 % columns as 0/191/64/255), and resource 1053 numbers the extra channels 3, 4, 5, … after 0 for the merged transparency. A PSD with spot channels whose 1057 is exactly what PhotoCraft 0.1.0–0.16.0 wrote, and with no 1053 or one kept from an opened Photoshop file that doesn't list every channel, is read as those versions stored it (straight past the channels that 1053 lists), so it keeps its ink. Multichannel is unchanged (#7415 split, part 4) |
| 2026-10-23 | minor | TIFF: alpha and spot channels are written and read as extra samples (at most 16) with Photoshop's channel resources (1006, 1045, 1077, 1053), spot ink stored 0 = solid as Photoshop 27.11's TIFFs store it; PSD output is unchanged. FILE-215-7 done, FILE-215-17 partial (its TIFF part) (#7415, supersedes #3121) |
| 2026-10-23 | patch | PSD and layered TIFF: the `MTrn` block Photoshop writes in a layered TIFF saved with Save Transparency (a positive layer count, `nrTM` in an `II` file) marks the merged image's first extra channel as its transparency, like `Mtrn`/`Mt16`/`Mt32`: such a file no longer opens with its transparency also as an alpha channel (named "Transparency" in Photoshop 27.11's file), and a PSD save no longer writes it twice. A PSD save with an opaque composite drops any of these markers kept from the opened file, so its first alpha channel no longer reopens as transparency and vanishes (split from #7415; credit #3121) |
| 2026-10-23 | minor | Amiga IFF write: Save As `.iff` stores a 24-plane ILBM (ByteRun1, alpha as a 1-bit mask plane), and 24-plane ILBMs open |
| 2026-10-16 | minor | Save for Web (Legacy) offers WebP, lossy or lossless, with its own preview and size estimate (#2415, #6391) |
| 2026-10-23 | minor | #8940: Filter Gallery Halftone Pattern maps size, contrast and dot/circle/line both ways with the reference application's descriptor keys (`HlSz`, `Cntr`, `ScrT`); PSD/PSB round trips at 8/16/32 bits, malformed descriptors preserved verbatim. Other gallery effects remain partial; no floor or estimate changed. |
| 2026-10-22 | minor | 32-bit Hue/Saturation as Photoshop does it: the raised saturation isn't clipped (saturated colours leave 0..1), the chroma is scaled around the lightness by Photoshop's saturation factor (measured at every whole percent), so colours at or above lightness 1 keep their hue, and Colorize builds its colour with the saturation times 1 − \|Lightness\| (8 and 16 bits unchanged): photoshop rgb32 hue-saturation and hue-saturation-colorize pass; Photoshop-oracle floor 188 → 190 of 258 (#5781) |
| 2026-10-19 | minor | JPEG Options on File › Save, Save As and Save a Copy (FILE-215-19 partial: no Matte, progressive Scans or size preview) |
| 2026-10-18 | minor | Scripted TIFF saves take `tiffCompression` (none, LZW, ZIP), and the CLI `--tiff-compression` (#5301) |
| 2026-10-17 | minor | `.atn`: a recorded Save As imports with its format and file (`file.saveAs`, `file.saveACopy` for As a Copy) instead of as a plain Save that wrote over the played file, #7102; folder destinations stop playback until #5430 |
| 2026-10-17 | minor | `.atn`: Apply Layer Comp by name maps to `layerComp.apply` and exports as `applyComp` (#7101) |
| 2026-10-15 | minor | Legacy PSD point-text placement is refined after font resolution using the rendering engine; centered/right alignment keeps its layout offset, while valid `Txt2` frames remain authoritative. Synthetic alignment, font-context and round-trip regressions cover this fallback. |
| 2026-10-15 | minor | Cineon write: Save As `.cin` stores 10-bit log gray/RGB frames (Cineon 4.5 header, three samples to a 32-bit word); a Cineon frame saved back keeps its code values |
| 2026-10-14 | minor | 32-bit Hue/Saturation as Photoshop does it: the raised saturation isn't clipped (saturated colours leave 0..1) and the slider is cut to whole 8-bit levels, and Colorize builds its colour with the saturation times 1 − \|Lightness\| (8 and 16 bits unchanged): photoshop rgb32 hue-saturation and hue-saturation-colorize pass |
| 2026-10-14 | minor | Lab documents round each adjustment layer's result on the stored L, a, b to whole samples, as Photoshop stores them (the native Lab path): photoshop lab8 curves.psd passes |
| 2026-10-15 | minor | PSD smart filters: Clouds, Difference Clouds, Fibers and Lens Flare open as editable filters with their seeds, colours and lens, and save like Photoshop (checked against a Photoshop 27 file); render filters on a smart object work within the object, so a flare sits where Photoshop puts it |
| 2026-10-15 | minor | DDS textures save (#1327): uncompressed or BC1/BC2/BC3 (DXT1/3/5) from a clean-room block encoder, optional box-filtered mip chain, legacy header; DDS Options in Save As and Save a Copy, `ddsCompression`/`ddsMipmaps` for scripts, the CLI and MCP |
| 2026-10-16 | minor | PSD smart filters: Perspective Warp (`perspectiveWarpTransform`: `vertices`, `warpedVertices`, `quads` of vertex indices, document pixels) opens as `layer.smartObjects.perspectiveWarp`, one plane per quad, and re-renders warped; save writes the original item with only the geometry rewritten, or a new one in Photoshop's layout; inconsistent descriptors stay verbatim (FILE-215-2, #6268) |
| 2026-10-16 | minor | PSD smart filters: Hue/Saturation (`HStr`: `Clrz`, `Adjs` = master `Hst2` plus one per edited range with `LclR` and its four range sliders, all longs, measured on a Photoshop-saved file) opens as `image.adjustments.hueSaturation`, re-renders with the smart object and saves in Photoshop's layout; layouts outside it stay verbatim (FILE-215-2, #6267) |
| 2026-10-15 | minor | PSD smart filters without options (Blur, Blur More, Average, Despeckle, Find Edges, Facet, Fragment, Sharpen, Sharpen More, Sharpen Edges, Solarize) map both ways as Photoshop stores them: `hasoptions` false, no `Fltr`, the four-char `filterID` (FILE-215-2) |
| 2026-10-15 | minor | PSD smart filters: Surface Blur (filterID 1135, `surfaceBlur`), Smart Blur (`SmrB`) and Radial Blur (`RdlB`) open as editable filters and save with the keys, types and enums measured in Photoshop; out-of-range, unknown-enum or extra-key entries (an off-centre Radial Blur) stay verbatim (#6298, FILE-215-2) |
| 2026-10-15 | minor | PSD smart filters: Smart Sharpen (filterID 1131, Gaussian removal; Lens/Motion Blur and legacy entries stay verbatim) and Dust & Scratches (`DstS`) open as editable filters and save in Photoshop's measured shape (#6300) |
| 2026-10-15 | minor | PSD smart filters: Diffuse (`Dfs `, `DfsM` mode), Trace Contour (`TrcC`, level and `CntE` edge), Crystallize (`Crst`) and Pointillize (`Pntl`, background from `BckC`) open as editable smart filters and save in the shape measured in Photoshop (#6302, FILE-215-2) |
| 2026-10-15 | minor | PSD smart filters: Wind (`Wnd `, `WndM` method, `Drct` direction), Extrude (`Extr`, `ExtT` type, size, depth, `ExtR` depth mode, solid front faces, mask incomplete blocks) and Tiles (`Tls `, count, offset, `FlCl` fill with the entry's `FrgC`/`BckC`) open as editable smart filters and save in the shape measured in Photoshop (#6314, FILE-215-2) |
| 2026-10-15 | minor | PSD smart filters: Offset (`Ofst`: `Hrzn`/`Vrtc` long, `Fl  ` `FlMd` Wrap/Background/Repeat) and Custom (`Cstm`: `Scl `, `Ofst` long, 25-long `Mtrx`) map both ways in `filterFX`, matching descriptors measured in Photoshop (FILE-215-2, #6304) |
| 2026-10-15 | minor | PSD smart filters: Mezzotint (`Mztn`, `MztT` enum, all 10 types) and Color Halftone (`ClrH`, `Rds `/`Ang1`–`Ang4` longs) open as editable filters and save in Photoshop's shape, measured in Photoshop (FILE-215-2); Mezzotint's PhotoCraft-only seed is not saved |
| 2026-10-15 | minor | PNG: a stereo PNG's `sTER` pairing (PNG Extensions 1.4.0 §3.6) is read on open and written back on save, so the file still says it is a stereo pair (#6063) |
| 2026-10-15 | minor | Indexed Color documents save as 8-bit palette BMPs with their colour table instead of 24-bit RGB; a palette BMP opens with its declared number of colours, not 256 |
| 2026-10-15 | minor | PSD guides: image resource 1092 (guides v2) gives guides their own colours on import; 1092 and 1097 (guide GUIDs) are written back only while they still describe the document's guides, so an edit no longer leaves Photoshop a stale second guide list (#5966) |
| 2026-10-14 | minor | PSD smart objects warped with Transform Warp's Cylinder (`warpCylinder`, seven `warpValues`) are read and written: Replace Contents, Relink and Edit Contents re-render them wrapped instead of flat, and a regenerated `SoLd` no longer drops the warp (#4112). Vertical cylinders and ones with perspective are still unread |
| 2026-10-15 | minor | Bevel & Emboss Gloss Contour › Anti-aliased (`antialiasGloss`) is read and written, set by `glossAntiAlias` and shown in the Layer Style dialog; a PSD whose bevel is edited no longer loses it. Rendering still ignores contour anti-aliasing, as for the other effects' contours |
| 2026-10-15 | minor | Stroke › Overprint (`overprint`) is read and written, set by the stroke command's `overprint` and shown in the Layer Style dialog; an edited stroke no longer drops it. Rendering ignores it for now |
| 2026-10-14 | minor | Nikon NEF: the D4 profile's saturation level follows the sample depth (15520 at 14 bits, 3880 at 12), so 12-bit D4 files open as bright as their 14-bit twins instead of 2.6 stops darker (#5978) |
| 2026-10-14 | minor | Animated WebP: New Video Layer from File, Replace Footage, Reload Frame and Video Frames to Layers take every frame, composited, with the timeline rate from the durations (#1198) |
| 2026-10-14 | minor | PSD smart filter masks: Photoshop stores a smart object's filtered pixels without its filter mask and mixes the unfiltered ones (`FEid`) back in; import now mixes them and export writes them that way, so sf-filter-mask-hard, -gradient and sf-stack-blend-and-mask pass; Photoshop-oracle floor 185 → 188 of 258 |
| 2026-10-13 | minor | Lossy DNG (compression 34892) decodes: 8-bit baseline JPEG tiles and strips, CFA and LinearRaw, partial edge tiles; the stored values take the normal levels and LinearizationTable (FILE-215-11, lossy DNG part) |
| 2026-10-14 | minor | Corpus floors raised to the measured results of the CMYK, Lab, 32-bit, Levels, Threshold, Posterize, feather and pass-through fixes landed together: Photoshop-authored 165 → 185 of 258, io 146 → 148 of 170, psd-tools 242 → 249 of 309 |
| 2026-10-13 | minor | Large-file fixtures generated at test time (20 MP layered, 150-layer layout, 2.02 GB PSD, 4.38 GB PSB), opened, saved and re-opened by io tests/it/large_files.rs; FILE-216-2 done; P20/P63 open times measurable (#5107) |
| 2026-10-13 | minor | PSD smart filters: Pinch, Polar Coordinates, Ripple, Spherize, Twirl, Wave, ZigZag and Invert open as editable filters and are saved instead of dropped (checked against a Photoshop 27 file); a scaled Wave, a Wave with more than 64 generators and Shear stay verbatim |
| 2026-10-13 | minor | CMYK Hue/Saturation (not Colorize) and Color Balance work on C, M and Y as inverted RGB with black kept, on the inks for display, export and destructive edits; cmyk8/16 hue-saturation 100% → 0.4–0.7% bad pixels, color-balance 74% → 25% |
| 2026-10-13 | minor | CMYK Posterize works on the stored values (1 − ink), as Photoshop does: psd-tools posterize_cmyk passes (13.8 % → 0 of pixels off on the ink path); psd-tools floor → 242 of 309 |
| 2026-10-13 | minor | CMYK documents whose layers are plain raster layers plus Invert, Posterize, Threshold, Brightness/Contrast, Levels or Curves layers composite on their own inks for display, export and flattening: the cmyk8/16 oracles for those seven adjustments pass; Photoshop-oracle floor 151 → 165 of 258 |
| 2026-10-13 | minor | CMYK Threshold compares the Rec. 601 luma of the inks' plain complement, as Photoshop does (every pixel of cmyk8/16 threshold-128.psd); oracle bad pixels 1.6% → 0.2% |
| 2026-10-13 | minor | CMYK Brightness/Contrast (modern and legacy) curves each ink's stored value (1 − ink), exact on destructive edits; cmyk8/16 oracle bad pixels 71% → 48–57% (the RGB composite's separation still differs) |
| 2026-10-13 | minor | Modern Brightness/Contrast: the brightness curve refit to Photoshop, so the rgb and gray 8/16/32 brightness-contrast oracles pass; Photoshop-oracle floor 145 → 151 of 258 |
| 2026-10-13 | minor | Lab documents of plain layers with Invert, Posterize, Threshold, Brightness/Contrast or Lab Levels/Curves adjustment layers composite on the stored L, a, b instead of clipped display RGB, and flattening keeps them: lab16 invert, posterize-4, levels, curves and legacy Brightness/Contrast and lab8 invert and posterize-4 pass; Photoshop-oracle floor 145 → 152 of 258 |
| 2026-10-13 | minor | Levels round the gamma and output steps to whole levels too (8-bit), and the gamma above 1 starts along a toe of slope 2^gamma that joins the power curve: psd-tools levels_rgb and rgb8 stack-levels-curves-hue-posterize pass, rgb8/gray8 levels.psd exact; Photoshop-oracle floor 145 → 146 of 258, psd-tools 236 → 238 of 309 |
| 2026-10-13 | minor | 32-bit documents: legacy Brightness/Contrast, Channel Mixer, Photo Filter and the Black & White tint keep values above 1 and below 0 as Photoshop does (rgb32/gray32 brightness-contrast-legacy, rgb32 channel-mixer and black-white-tint pass); Photoshop-oracle floor 145 → 149 of 258 |
| 2026-10-13 | minor | Mask feathers blur with exactly the feather's variance (three extended boxes; whole-pixel boxes rounded 6 px to 6.48): psd-tools mask-parameters-no-real-channel passes, layer_mask_data 27 % → 4 % off; psd-tools floor → 240 of 309 (main 239) |
| 2026-10-13 | minor | Threshold splits on the luminosity 0.30 R + 0.59 G + 0.11 B of whole samples, as Photoshop does (8-bit: rounded to a level, halves down; 16-bit: against the level in 16 bits), not the Rec. 601 grey: psd-tools threshold_rgb and threshold_16bits_rgb pass; psd-tools floor → 241 of 309 (main 239) |
| 2026-10-13 | minor | A pass-through group that is the base of a clipping group and holds an adjustment layer renders isolated, as Photoshop does (its blend modes still reach the backdrop): psd-tools passthrough_clipping_mask_adjustment passes (was 50 % of pixels off); psd-tools floor → 240 of 309 (main 239) |
| 2026-10-13 | minor | 16-bit Posterize bins the 16-bit sample and lands on 8-bit levels through a 16-bit step, as Photoshop does (RGB, Grayscale and the CMYK/Lab channel path): psd-tools posterize_16bits_rgb and posterize_16bits_grayscale pass; psd-tools floor → 241 of 309 (main 239) |
| 2026-10-13 | minor | Gradient Map adjustment layers are driven by the W3C luminosity (0.3, 0.59, 0.11), not the Rec. 601 grey: rgb8 and rgb16 gradient-map.psd pass; Photoshop-oracle floor 145 → 147 of 258 |
| 2026-10-13 | minor | PSD: a "Saving Merged Transparency" global block (`Mtrn`, `Mt16`, `Mt32`; Photoshop's 32-bit documents keep a positive layer count) marks the merged image's first extra channel as its transparency: opening such a file no longer adds a "Transparency" alpha channel, and the oracle compares psd-tools transparentbg.psd's alpha (83.7 % → 0.9 % of pixels off; its shape edges remain) |
| 2026-10-13 | minor | Palette PNGs open in Indexed Color mode (#4804) |
| 2026-10-12 | minor | `.atn`: 112 events mapped (Select and Mask, Filter Gallery Spatter, Ripple, Crystallize, Pattern Fill layers, mask targeting, Step Backward, points, Bitmap) |
| 2026-10-12 | minor | DDS textures open, read-only (#1327) |
| 2026-10-12 | minor | #2465 fixed: Photoshop CS2 placed layers (`PlLd` only, file in `lnkD`) open as editable smart objects and save their edits |
| 2026-10-12 | minor | CMYK Invert, Posterize and Threshold work on the inks and 16-bit Lab Invert mirrors a and b around neutral (oracle bad pixels: CMYK 71–77% → 9–10%, 37% → 1.6%; lab16 invert 52% → 1.2%); Photoshop-oracle floor 133 → 145 of 258 |
| 2026-10-12 | minor | DPX read and write (#1049); Cineon still missing |
| 2026-10-12 | minor | Photoshop Actions `.atn`: read and write, 90 events mapped, the rest kept verbatim (AUTO-219-2) |
| 2026-10-12 | minor | Presets table: Contours `.shc` read and write through the Contour Editor (FILE-215-15); `.csh` and `.tpl` stay missing |
| 2026-10-12 | minor | Palette BMPs open in Indexed Color mode (#3802) |
| 2026-10-20 | minor | `.asl` layer style libraries: import and export with embedded patterns and blending options |
| 2026-10-12 | minor | Save writes a JPEG at its own estimated quality and a WebP lossy or lossless as opened (#2399) |
| 2026-10-11 | minor | PSD smart filters: Filter Gallery (`GEfc`) maps to `filter.filterGallery` when every effect is modelled (Chrome so far), editable from the Layers panel |
| 2026-10-11 | minor | D5100 image area and fitted-look preservation in Camera Raw tuning |
| 2026-10-11 | minor | PDF write now covers Save As / Save a Copy / CLI (`io::pdf::export`, #3401): one flattened RGB/Gray/CMYK page at the document's print size with the embedded profile. Read (#2541), multi-page save, PDF presentation, vector/text output still open
| 2026-10-11 | minor | Sony encrypted applied WB/black and measured ILCE-6000 crop; camera calibration remains partial |
| 2026-10-10 | major | First version: Photoshop 2026 27.11.0's formats from its bundle, against our codecs; Affinity notes moved from `roadmap.md` |
