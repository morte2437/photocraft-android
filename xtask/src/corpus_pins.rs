//! **Every test-corpus pin lives here.** Real-file corpora are never committed to this
//! repository: `cargo xtask corpus --all` fetches them into `corpus/` (gitignored) at the commits
//! below and verifies each file against the `xtask/*.sha256` manifests. The CI corpus job's cache
//! key hashes this file and the manifests, so changing a pin is the only thing that refetches.
//!
//! - Our own oracles: https://github.com/storytold/photocraft-corpus (README, AGENTS.md).
//! - Third-party sets come from their upstreams (licences in ATTRIBUTION.md).
//!
//! Moving a pin: change the commit, run `cargo xtask corpus --<name> --update-manifest`, review the
//! manifest diff, and re-run `cargo xtask test-corpus` (adjust floors only upwards).

use crate::pinned::{PinnedCorpus, Upstream};

/// https://github.com/storytold/photocraft-corpus: our Photoshop-authored oracle PSDs
/// (`photoshop/`). Bump through a PR after committing there.
pub const PHOTOCRAFT_CORPUS_COMMIT: &str = "f5b1178cab15309e05b7b504545df3c701f6d8fd";
/// https://github.com/storytold/photocraft-corpus: our GIMP-authored XCF oracles (`gimp/`). Its own
/// pin, so that set can move without refetching `photoshop/`. Bump through a PR after committing there.
pub const PHOTOCRAFT_CORPUS_GIMP_COMMIT: &str = "fbe9227a48d06de87061e527541209a178fd685c";
/// https://github.com/psd-tools/psd-tools (MIT): `tests/psd_files` (main, 2026-10-05).
pub const PSD_TOOLS_COMMIT: &str = "96eb134c17b2c65edf4c4151c0f00b802ada86c2";
/// https://github.com/Agamnentzar/ag-psd (MIT): `test/` (master, 2026-07-02).
pub const AG_PSD_COMMIT: &str = "387049670cb89b88fb8fe1b7c01aeacf98dd2e3b";
/// https://github.com/tbraun96/heic-rs (MIT OR Apache-2.0): `tests/fixtures` at the v0.1.1 tag, the
/// version photocraft-heif pins (2026-09-12).
pub const HEIC_RS_COMMIT: &str = "4f0d4df474c773dfc3b14fa80e7219be6898866e";
/// https://github.com/bigcat88/pillow_heif (BSD-3-Clause): `tests/images` (master, 2026-09-25).
pub const PILLOW_HEIF_COMMIT: &str = "b16be1196dfa465a342d68894696e685ca3655cb";
/// https://github.com/AcademySoftwareFoundation/openexr (BSD-3-Clause): `src/test/bin/test_images`
/// at the v3.5.2 tag (2026-10-03).
pub const OPENEXR_COMMIT: &str = "69b2604fc76e370615438bdc8d2cd95b9349c12e";
/// https://github.com/samuel-etver/vector-art (CC0-1.0): `simple/`, four Affinity 3 `.af` files and
/// Designer `.afdesign` files (main, 2026-04-22).
pub const VECTOR_ART_COMMIT: &str = "255f8add3c8f0740196e22bd59502b811b532f0b";
/// https://github.com/NickBeeuwsaert/AFDesignLoad (MIT): `testDesigns/`, purpose-made Designer files.
pub const AFDESIGNLOAD_COMMIT: &str = "a18dd50a7079fb861a28835eeefa0d015f03f4a1";
/// https://github.com/Jac21/Branding (MIT): Designer logos with artboards and placed images.
pub const JAC21_BRANDING_COMMIT: &str = "57ae3f45bf4637f6927c0b07de25b922c55f944f";
/// https://github.com/eviltwo/AssetStoreTemplate (MIT): an Affinity template with artboards.
pub const ASSET_STORE_TEMPLATE_COMMIT: &str = "f671981ee89d15526285b3cc87b4393d3defe644";
/// https://github.com/satoshoe-dev/affinity-samples (CC0-1.0): purpose-made Affinity 3 documents
/// and exported PNG references for the gaps tracked in #1606.
pub const AFFINITY_SAMPLES_COMMIT: &str = "96aca6bd86bbe7a677f1cf0630bd832d23943be8";
/// PngSuite (public domain), a fixed release archive.
pub const PNGSUITE_URL: &str = "http://www.schaik.com/pngsuite/PngSuite-2017jul19.tgz";
/// Fixed archive, corroborated by HTTPS records in opencodecs at 6f070029 and rivet-png at
/// 134faad8 (corpus/checksums.json and tests/pngsuite/SHA256SUMS). Check the legacy HTTP
/// download against this committed digest before extraction.
pub const PNGSUITE_SHA256: &str = "0294b244c95a8342c01b00010cf34abdcabc7c6a34fd0fe1bd963917537bfdc8";

/// `corpus/photoshop`: 258 PSDs authored with Photoshop by the photocraft-corpus generator.
pub const PHOTOSHOP: PinnedCorpus = PinnedCorpus {
    name: "photoshop",
    dest: "photoshop",
    upstreams: &[Upstream {
        prefix: "",
        repo: "storytold/photocraft-corpus",
        commit: PHOTOCRAFT_CORPUS_COMMIT,
        subdir: "photoshop",
        extras: &[("README.md", "README.md"), ("LICENSE-MIT", "LICENSE-MIT"), ("LICENSE-APACHE", "LICENSE-APACHE")],
    }],
    manifest: "xtask/photoshop-corpus.sha256",
    exts: &["psd", "psb"],
    subset: false,
    sources_md: |c| {
        format!(
            "# Photoshop oracle corpus\n\n`photoshop/` of https://github.com/storytold/photocraft-corpus at commit {PHOTOCRAFT_CORPUS_COMMIT} \
             (see `README.md`). MIT OR Apache-2.0, authored by the PhotoCraft contributors. Fetched and sha256-verified by \
             `cargo xtask corpus --photoshop` against `{}`. Gitignored; never commit these files.\n",
            c.manifest
        )
    },
};

/// `corpus/gimp`: 14 XCF files written by GIMP 3.2.6 by the photocraft-corpus generator, each with
/// GIMP's merged rendering as a PNG (28 files), the oracles of `photocraft-io`'s XCF reader.
pub const GIMP: PinnedCorpus = PinnedCorpus {
    name: "gimp",
    dest: "gimp",
    upstreams: &[Upstream {
        prefix: "",
        repo: "storytold/photocraft-corpus",
        commit: PHOTOCRAFT_CORPUS_GIMP_COMMIT,
        subdir: "gimp",
        extras: &[("README.md", "README.md"), ("LICENSE-MIT", "LICENSE-MIT"), ("LICENSE-APACHE", "LICENSE-APACHE")],
    }],
    manifest: "xtask/gimp-corpus.sha256",
    exts: &["xcf", "png"],
    subset: false,
    sources_md: |c| {
        format!(
            "# GIMP XCF oracle corpus\n\n`gimp/` of https://github.com/storytold/photocraft-corpus at commit {PHOTOCRAFT_CORPUS_GIMP_COMMIT} \
             (see `README.md`). MIT OR Apache-2.0, authored by the PhotoCraft contributors with GIMP 3.2.6. Fetched and sha256-verified by \
             `cargo xtask corpus --gimp` against `{}`. Gitignored; never commit these files.\n",
            c.manifest
        )
    },
};

/// `corpus/psd-tools`: the complete psd-tools test set.
pub const PSD_TOOLS: PinnedCorpus = PinnedCorpus {
    name: "psd-tools",
    dest: "psd-tools",
    upstreams: &[Upstream { prefix: "", repo: "psd-tools/psd-tools", commit: PSD_TOOLS_COMMIT, subdir: "tests/psd_files", extras: &[("LICENSE", "LICENSE")] }],
    manifest: "xtask/psd-tools-corpus.sha256",
    exts: &["psd", "psb"],
    subset: false,
    sources_md: |c| {
        format!(
            "# psd-tools test corpus\n\nThe PSD/PSB files of https://github.com/psd-tools/psd-tools/tree/{PSD_TOOLS_COMMIT}/tests/psd_files, \
             unmodified. MIT licence, Copyright (c) 2019 Kota Yamaguchi (see `LICENSE`). Fetched and sha256-verified by \
             `cargo xtask corpus --psd-tools` against `{}`. Gitignored; never commit these files.\n",
            c.manifest
        )
    },
};

/// `corpus/psd`: the hand-picked mix of small psd-tools and ag-psd files (170) that most PSD
/// tests use. The manifest selects the files.
pub const PSD_MIXED: PinnedCorpus = PinnedCorpus {
    name: "psd",
    dest: "psd",
    upstreams: &[
        Upstream {
            prefix: "psd-tools/",
            repo: "psd-tools/psd-tools",
            commit: PSD_TOOLS_COMMIT,
            subdir: "tests/psd_files",
            extras: &[("LICENSE", "psd-tools/LICENSE")],
        },
        Upstream { prefix: "ag-psd/", repo: "Agamnentzar/ag-psd", commit: AG_PSD_COMMIT, subdir: "test", extras: &[("LICENSE", "ag-psd/LICENSE")] },
    ],
    manifest: "xtask/psd-corpus.sha256",
    exts: &["psd", "psb"],
    subset: true,
    sources_md: |c| {
        format!(
            "# PSD test corpus (mixed)\n\nSmall PSD/PSB files selected from https://github.com/psd-tools/psd-tools/tree/{PSD_TOOLS_COMMIT}/tests/psd_files \
             (`psd-tools/`, MIT, Copyright (c) 2019 Kota Yamaguchi) and https://github.com/Agamnentzar/ag-psd/tree/{AG_PSD_COMMIT}/test \
             (`ag-psd/`, MIT, Copyright (c) 2016 Agamnentzar), unmodified; licences next to them. Selected by and sha256-verified \
             against `{}` by `cargo xtask corpus --psd`. Gitignored; never commit these files.\n",
            c.manifest
        )
    },
};

/// `corpus/heif`: a few small HEIC/HEIF files for the `heif` feature's tests. heic-rs's
/// checkerboards, RGB strips and a grid-tiled photo with EXIF and XMP (synthetic pixels encoded by
/// macOS `sips`, each `.ref.png` Apple's own decode), and pillow-heif's 10-bit RGBA file with the
/// 16-bit PNG it was encoded from. The manifest selects the files.
pub const HEIF: PinnedCorpus = PinnedCorpus {
    name: "heif",
    dest: "heif",
    upstreams: &[
        Upstream {
            prefix: "heic-rs/",
            repo: "tbraun96/heic-rs",
            commit: HEIC_RS_COMMIT,
            subdir: "tests/fixtures",
            extras: &[("LICENSE-MIT", "heic-rs/LICENSE-MIT"), ("LICENSE-APACHE", "heic-rs/LICENSE-APACHE")],
        },
        Upstream {
            prefix: "pillow-heif/",
            repo: "bigcat88/pillow_heif",
            commit: PILLOW_HEIF_COMMIT,
            subdir: "tests/images",
            extras: &[("LICENSE.txt", "pillow-heif/LICENSE.txt")],
        },
    ],
    manifest: "xtask/heif-corpus.sha256",
    exts: &["heic", "heif", "png"],
    subset: true,
    sources_md: |c| {
        format!(
            "# HEIF test corpus\n\nSmall files selected from https://github.com/tbraun96/heic-rs/tree/{HEIC_RS_COMMIT}/tests/fixtures \
             (`heic-rs/`, MIT OR Apache-2.0, Thomas Braun) and https://github.com/bigcat88/pillow_heif/tree/{PILLOW_HEIF_COMMIT}/tests/images \
             (`pillow-heif/`, BSD-3-Clause, Pillow-Heif contributors), unmodified; licences next to them. Selected by and sha256-verified \
             against `{}` by `cargo xtask corpus --heif`. Gitignored; never commit these files.\n",
            c.manifest
        )
    },
};

/// `corpus/exr`: the deep OpenEXR test images (scanline and tiled deep data, several channel
/// types) the deep decoder was verified against. The manifest selects the files.
pub const EXR: PinnedCorpus = PinnedCorpus {
    name: "exr",
    dest: "exr",
    upstreams: &[Upstream {
        prefix: "",
        repo: "AcademySoftwareFoundation/openexr",
        commit: OPENEXR_COMMIT,
        subdir: "src/test/bin/test_images",
        extras: &[("LICENSE.md", "LICENSE.md")],
    }],
    manifest: "xtask/exr-corpus.sha256",
    exts: &["exr"],
    subset: true,
    sources_md: |c| {
        format!(
            "# Deep OpenEXR test corpus

The `*.deep.exr` files of https://github.com/AcademySoftwareFoundation/openexr/tree/{OPENEXR_COMMIT}/src/test/bin/test_images,              unmodified. BSD-3-Clause, Copyright Contributors to the OpenEXR Project (see `LICENSE.md`). Selected by and sha256-verified              against `{}` by `cargo xtask corpus --exr`. Gitignored; never commit these files.
",
            c.manifest
        )
    },
};

/// `corpus/affinity`: public Affinity documents saved by Affinity 1.x to 3.x, chosen for having no
/// personal paths in their metadata. Each embeds Affinity's own render of itself (its thumbnail),
/// the oracle `photocraft-io`'s corpus test compares the imported document against. The manifest
/// selects the files.
pub const AFFINITY: PinnedCorpus = PinnedCorpus {
    name: "affinity",
    dest: "affinity",
    upstreams: &[
        Upstream {
            prefix: "vector-art/",
            repo: "samuel-etver/vector-art",
            commit: VECTOR_ART_COMMIT,
            subdir: "simple",
            extras: &[("LICENSE", "vector-art/LICENSE")],
        },
        Upstream {
            prefix: "afdesignload/",
            repo: "NickBeeuwsaert/AFDesignLoad",
            commit: AFDESIGNLOAD_COMMIT,
            subdir: "testDesigns",
            extras: &[("LICENSE", "afdesignload/LICENSE")],
        },
        Upstream {
            prefix: "jac21/",
            repo: "Jac21/Branding",
            commit: JAC21_BRANDING_COMMIT,
            subdir: "Logos/JC/DesignerFiles/Affinity",
            extras: &[("LICENSE.md", "jac21/LICENSE.md")],
        },
        Upstream {
            prefix: "asset-store-template/",
            repo: "eviltwo/AssetStoreTemplate",
            commit: ASSET_STORE_TEMPLATE_COMMIT,
            subdir: "AssetStoreTemplate",
            extras: &[("LICENSE", "asset-store-template/LICENSE")],
        },
        Upstream {
            prefix: "affinity-samples/",
            repo: "satoshoe-dev/affinity-samples",
            commit: AFFINITY_SAMPLES_COMMIT,
            subdir: "",
            extras: &[("LICENSE", "affinity-samples/LICENSE"), ("README.md", "affinity-samples/README.md")],
        },
    ],
    manifest: "xtask/affinity-corpus.sha256",
    exts: &["af", "afdesign", "aftemplate", "png"],
    subset: true,
    sources_md: |c| {
        format!(
            "# Affinity test corpus\n\nPublic Affinity documents, unmodified, with their licences next to them: \
             https://github.com/samuel-etver/vector-art/tree/{VECTOR_ART_COMMIT}/simple (`vector-art/`, CC0-1.0), \
             https://github.com/NickBeeuwsaert/AFDesignLoad/tree/{AFDESIGNLOAD_COMMIT}/testDesigns (`afdesignload/`, MIT, Copyright (c) 2015 Nick Beeuwsaert), \
             https://github.com/Jac21/Branding/tree/{JAC21_BRANDING_COMMIT}/Logos/JC/DesignerFiles/Affinity (`jac21/`, MIT, Copyright (c) 2018 Jeremy Cantu) and \
             https://github.com/eviltwo/AssetStoreTemplate/tree/{ASSET_STORE_TEMPLATE_COMMIT}/AssetStoreTemplate (`asset-store-template/`, MIT) and \
             https://github.com/satoshoe-dev/affinity-samples/tree/{AFFINITY_SAMPLES_COMMIT} (`affinity-samples/`, CC0-1.0; includes PNG references for issue #1606). \
             Selected by and sha256-verified against `{}` by `cargo xtask corpus --affinity`. Gitignored; never commit these files.\n",
            c.manifest
        )
    },
};
/// Photoshop Actions files (`.atn`) from public repositories under permissive licences, the
/// real-file oracles of `photocraft-psd::atn` and the engine's `.atn` import (2026-10-12).
/// https://github.com/sbaril/Photoshop-Scripts (MIT): `Actions/`.
pub const SBARIL_COMMIT: &str = "622493c9c617c9d2ff38500e8e512c0ad421eed9";
/// https://github.com/bjango/Bjango-Actions (BSD, `Help/License.md`): three sets at the root.
pub const BJANGO_COMMIT: &str = "0fdc790b4931fd36816e6056dd84c2aa0d3ff054";
/// https://github.com/BKWLD/photoshop-actions (MIT).
pub const BKWLD_COMMIT: &str = "d1a5d24e02bf23f78659106473ef7d8c747bea00";
/// https://github.com/mrijk/atntools (MIT): the fixture of a Rust `.atn` reader.
pub const ATNTOOLS_COMMIT: &str = "529db30b631f32bc7970121af0393853c2154fa0";
/// https://github.com/damsmcfly/photoshop-cleaner-trimmer (Unlicense).
pub const TRIMMER_COMMIT: &str = "fd3814df2a625ed7a015633eb91d8fe2f5bbe9a0";
/// https://github.com/uclagamelab/AlphaUtilityAdditions (BSD-2-Clause).
pub const ALPHA_UTILITY_COMMIT: &str = "7bdae950a7db4c6c2224bcaf08f4c17ec18958cb";
/// https://github.com/overbyte/photoshop-gif-creator-script (MIT).
pub const GIF_CREATOR_COMMIT: &str = "9fb78e8c09a21678afc84ccb31a435bd242894df";

/// `corpus/atn`: Photoshop Actions sets from seven public repositories, one folder each.
pub const ATN: PinnedCorpus = PinnedCorpus {
    name: "atn",
    dest: "atn",
    upstreams: &[
        Upstream { prefix: "sbaril/", repo: "sbaril/Photoshop-Scripts", commit: SBARIL_COMMIT, subdir: "Actions", extras: &[("LICENSE", "sbaril/LICENSE")] },
        Upstream { prefix: "bjango/", repo: "bjango/Bjango-Actions", commit: BJANGO_COMMIT, subdir: "", extras: &[("Help/License.md", "bjango/License.md")] },
        Upstream { prefix: "bkwld/", repo: "BKWLD/photoshop-actions", commit: BKWLD_COMMIT, subdir: "", extras: &[("LICENSE", "bkwld/LICENSE")] },
        Upstream { prefix: "atntools/", repo: "mrijk/atntools", commit: ATNTOOLS_COMMIT, subdir: "", extras: &[("LICENSE", "atntools/LICENSE")] },
        Upstream {
            prefix: "trimmer/",
            repo: "damsmcfly/photoshop-cleaner-trimmer",
            commit: TRIMMER_COMMIT,
            subdir: "",
            extras: &[("LICENSE", "trimmer/LICENSE")],
        },
        Upstream {
            prefix: "alpha-utility/",
            repo: "uclagamelab/AlphaUtilityAdditions",
            commit: ALPHA_UTILITY_COMMIT,
            subdir: "",
            extras: &[("LICENSE", "alpha-utility/LICENSE")],
        },
        Upstream {
            prefix: "gif-creator/",
            repo: "overbyte/photoshop-gif-creator-script",
            commit: GIF_CREATOR_COMMIT,
            subdir: "",
            extras: &[("LICENSE", "gif-creator/LICENSE")],
        },
    ],
    manifest: "xtask/atn-corpus.sha256",
    exts: &["atn"],
    subset: false,
    sources_md: |c| {
        format!(
            "# Photoshop Actions (.atn) corpus\n\nAction sets from public repositories, unmodified, each in its own folder with its licence: \
             `sbaril/` (https://github.com/sbaril/Photoshop-Scripts/tree/{SBARIL_COMMIT}/Actions, MIT), \
             `bjango/` (https://github.com/bjango/Bjango-Actions/tree/{BJANGO_COMMIT}, BSD), \
             `bkwld/` (https://github.com/BKWLD/photoshop-actions/tree/{BKWLD_COMMIT}, MIT), \
             `atntools/` (https://github.com/mrijk/atntools/tree/{ATNTOOLS_COMMIT}, MIT), \
             `trimmer/` (https://github.com/damsmcfly/photoshop-cleaner-trimmer/tree/{TRIMMER_COMMIT}, Unlicense), \
             `alpha-utility/` (https://github.com/uclagamelab/AlphaUtilityAdditions/tree/{ALPHA_UTILITY_COMMIT}, BSD-2-Clause), \
             `gif-creator/` (https://github.com/overbyte/photoshop-gif-creator-script/tree/{GIF_CREATOR_COMMIT}, MIT). \
             Fetched and sha256-verified by `cargo xtask corpus --atn` against `{}`. Gitignored; never commit these files.\n",
            c.manifest
        )
    },
};

/// Every pinned corpus, in fetch order.
pub const ALL: &[&PinnedCorpus] = &[&PSD_MIXED, &PSD_TOOLS, &PHOTOSHOP, &GIMP, &HEIF, &EXR, &AFFINITY, &ATN];

/// `corpus/pixls`: real camera raws from <https://raw.pixls.us> (released into the public
/// domain / CC0 by their photographers; see ATTRIBUTION.md). One file per decode path
/// `photocraft-raw` supports, plus the packed-ORF known-unsupported oracle. Fetched as single
/// files (the server 301-redirects `/data/` to `/download/`), verified against
/// `xtask/pixls.sha256`.
pub const PIXLS_BASE: &str = "https://raw.pixls.us/data/";
/// (url path, file name in `corpus/pixls`)
pub const PIXLS_FILES: &[(&str, &str)] = &[
    ("Canon/PowerShot%20SX50%20HS/IMG_4059.CR2", "IMG_4059.CR2"),
    ("Canon/PowerShot%20SX50%20HS/CRW_4061.DNG", "CRW_4061.DNG"),
    ("Nikon/D3/JD1_8203.NEF", "JD1_8203.NEF"),
    // Modern lossy NEF curve: the encoded range is two bits smaller than the sensor ADUs.
    ("Nikon/Z%206/DSC_0751.NEF", "DSC_0751.NEF"),
    ("Nikon/Z%206/DSC_0754.NEF", "DSC_0754.NEF"),
    // Uncompressed packed NEF selected by maker-note mode, despite TIFF Compression 34713.
    ("Nikon/Z%206/DSC_0750.NEF", "DSC_0750.NEF"),
    ("Nikon/Z%206/DSC_0753.NEF", "DSC_0753.NEF"),
    ("Sony/DSC-RX0/DSC00009.ARW", "DSC00009.ARW"),
    ("Panasonic/DC-G9/P1000475.RW2", "P1000475.RW2"),
    ("Olympus/E-1/E_1__C106743_gredos.ORF", "E_1__C106743_gredos.ORF"),
    ("Olympus/E-5/_7061961_copy.ORF", "_7061961_copy.ORF"),
];
