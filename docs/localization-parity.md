# Localization parity

> **Last reviewed:** 2026-10-12 · **Last updated:** 2026-10-29 · **Change:** minor (Swedish, Finnish, Danish and Norwegian Bokmål UI catalogs; previously minor: Arabic UI catalog) · **Target:** Adobe Photoshop 2026

Per-language status of PhotoCraft's UI. How catalogs work and how to add one:
[`localization.md`](localization.md). Language notes: [Korean](localization-ko.md), [Spanish](localization-es.md),
[Simplified Chinese](localization-zh-hans.md), [Italian](localization-it.md), [Russian](localization-ru.md).

**How measured.** `cargo xtask i18n-coverage` (`xtask/src/i18n_coverage.rs`) defines the key
universe: `tl!("…")` literals and `trn(` plurals in `ui-egui` (tests excluded), menu-catalog
labels and path segments, `menus.rs` UI commands, and engine `CommandSpec` labels and menu paths:
**1,608 English keys** on 2026-10-10. Each catalog (`crates/ui-egui/src/i18n/<code>.tsv`) was
checked against that universe by a source script replicating the xtask (no build). Catalogs hold
more rows than the universe because they also translate dynamic `tl!(expr)` strings (~159 call
sites the tool can't enumerate). **Not covered anywhere:** engine error and status messages and
filter parameter help, which are English in every language; so no language is `full`. The report
also lists, per language, the keys its catalog lacks; those show in English. `cargo xtask ci` prints
it as its last step and never fails on it.

**Photoshop 2026** ships UI language packs for cs, da, de, en, es, fi, fr (and fr-CA), hu, it,
ja, ko, nb, nl, pl, pt-BR, ru, sk, sv, tr, uk, zh-Hans, zh-Hant (24 `.lproj` markers; one pack is
installed per Creative Cloud install, here `en_US`). It has no Hindi, Arabic, Indonesian or
Vietnamese UI; Arabic and Hebrew users get English "Middle East" builds with the World-Ready
composer, and its text engine handles RTL, Devanagari and Indic scripts.

**Summary.** 7 of the 12 key languages at 100% of measured keys (status `partial` because of the
engine messages), 2 absent (Hindi, Vietnamese), plus 8 other languages. Weighted by
speakers, ~55% ready. Remaining: 80–140 h, plus native-speaker review (recorded only for Italian,
2026-10-12).

## The twelve key languages

| Language | Code | UI strings (measured keys) | Catalog rows | Dialogs / tooltips / help | Script support | Native review | Status | To `full` |
|---|---|---|---:|---|---|---|---|---:|
| English | `en` | 1,608 / 1,608 (100%) | source | all | Latin | – | full | 0 h |
| Simplified Chinese | `zh-hans` | 1,608 / 1,608 (100%) | 2,366 | dialogs, tooltips yes; engine messages English | CJK fonts (lazy loader), IME preedit/commit, vertical type | no | partial | 8–12 h |
| Spanish | `es` | 1,844 / 1,844 (100%) | 4,010 | as above; complete dynamic UI key set | Latin | no | partial | 6–10 h |
| Hindi | `hi` | partial | 448 | Top menus, toolbox tools, camera raw, dialogs, plurals | Devanagari shaping in the text engine (parley/HarfRust); no Devanagari fallback font (TYPE-218-3) | no | partial | 15–20 h |
| Arabic | `ar` | 2,175 / 2,175 (100%, 2026-10-28) | 3,410 | as above; combined from the community catalogs with one term per concept | Arabic shaping and bidi in the UI (egui 0.37) and the text engine, Arabic fallback font; RTL shell layout but not inside panels (DIST-220-4), paragraph direction buttons but no visual RTL caret (TYPE-218-2 partial) | no | partial | 15–25 h |
| French | `fr` | 1,608 / 1,608 (100%) | 2,766 | as above | Latin | no | partial | 6–10 h |
| Portuguese (Brazil) | `pt-br` | 1,608 / 1,608 (100%) | 2,788 | as above | Latin; every `pt` locale uses it | no | partial | 6–10 h |
| Indonesian | `id` | 1,608 / 1,608 (100%) | 2,764 | as above | Latin | no | partial | 6–10 h |
| Japanese | `ja` | 1,608 / 1,608 (100%) | 3,356 | as above | CJK fonts, IME (Windows IME broken, #590), vertical type (tategaki); installed Japanese fonts missing from the list (#2330) | no | partial | 10–16 h |
| German | `de` | 1,608 / 1,608 (100%) | 2,400 | as above | Latin | no | partial | 6–10 h |
| Korean | `ko` | 1,608 / 1,608 (100%) | 2,936 | as above; terminology from Photoshop's Korean UI ([notes](localization-ko.md)) | Hangul fonts, IME | no | partial | 6–10 h |
| Vietnamese | `vi` | 0 (0%) | – | – | Latin with stacked diacritics (shaping supported) | no | none | 12–18 h |

## Other shipped languages

8 more, each at 1,608 / 1,608 measured keys, status `partial` for the same reason: Traditional
Chinese `zh-hant` (2,329 rows), Russian `ru` (4,227 keys; also every dynamic label the
other catalogs translate, [notes](localization-ru.md)), Ukrainian `uk` (3,375), Czech `cs` (3,522),
Polish `pl` (3,366), Greek `el` (3,481), Dutch `nl` (3,568), Italian `it` (4,164; also every dynamic label the
other catalogs translate; reviewed by a native speaker, [notes](localization-it.md)). Turkish `tr`
(3,731 rows) also covers every measured key. Serbian `sr` (4,022 rows, Latin script, Ekavian, added 2026-10-12) covers every
measured key and the union of the other catalogs' dynamic labels; its one/few/many plurals reuse
the Russian rule. Swedish `sv` (4,552 rows, added 2026-10-28) covers every measured key and the
other catalogs' dynamic labels; combined from the community catalogs in #5985 and #9536 with one
term per concept; the contested terms follow a native speaker's questionnaire answers. Finnish
`fi` (4,552 rows, combined from #5666 and #6961), Danish `da` (4,552 rows, based on #7267 and
completed for the current UI) and Norwegian Bokmål `nb` (4,552 rows, combined from #8072 and #9025;
`no` and `nn` locales resolve to it), all added 2026-10-29, cover the same keys with one term per
concept; none has had a native-speaker review yet.

Czech UI follow-up (2026-10-11): status and notice wrappers, hardware labels, brush and
transform accessibility names, rasterization prompts, profile mismatch messages and preset
counts now translate at their render/assignment sites. Navigator's zoom level uses a context
(`Zvětšení`), while the tool remains `Lupa`. Engine errors, profile/file/document names stay
verbatim; copied System Info and control API diagnostics remain English while their on-screen
labels translate. Import warning counts use parentheses in Czech. New dynamic templates fall
back to English in catalogs without entries. This is additional UI coverage, not a claim that
engine localization or native review is complete.

## Cross-language gaps

- Engine error, status and parameter-help messages are English in every language.
- Right-to-left UI layout is partial (2026-10-10): the shell mirrors for Arabic, Hebrew, Persian,
  Urdu and other right-to-left codes (`direction.rs`), but panel interiors, dialogs, menu rows,
  submenus and document tabs stay left to right. Word order within a label is right: egui 0.37
  lays text out with the Unicode Bidirectional Algorithm.
- Web build: no browser-locale detection. It has no system fonts: Arabic, CJK and other non-Latin interface labels are drawn with the host's served script fonts (`packaging/web/README.md` › Fonts), and show as boxes on a host that serves none.
- WebAssembly plug-ins disappear from Filter › Plug-ins when the UI isn't English (#2540).
- A user couldn't find the language option (#2532): it is in Preferences › Interface.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-10 | major | First version, measured from the catalogs with the `i18n-coverage` rules |
| 2026-10-10 | minor | Right-to-left shell layout (menu bar, title, options and status bars, toolbox and dock sides) |
| 2026-10-11 | minor | Czech dynamic status/notice and hardware text, render labels and Navigator zoom context; engine messages remain English |
| 2026-10-11 | minor | Czech review follow-ups: English System Info diagnostics, id-aware shortcut labels, display-only auto translation, warning punctuation and wording; catalog row count refreshed |
| 2026-10-11 | minor | Italian: 1,287 dynamic labels added (filter parameters, Camera Raw, lighting, panels, dialogs) and terminology made consistent |
| 2026-10-12 | minor | Added Turkish (`tr`) catalog with 100% measured UI coverage |
| 2026-10-12 | minor | Spanish: 1,276 runtime-only labels added (filter and adjustment parameters, Camera Raw, panels, dialogs and status wrappers); generated-label coverage test added |
| 2026-10-12 | minor | Italian native-speaker review: mistranslations (Trap, HDR Toning, Display Text) and phrasing fixed |
| 2026-10-12 | minor | Serbian (`sr`, Latin, Ekavian) catalog: every measured key plus the dynamic labels of the other catalogs |
| 2026-10-13 | minor | Italian: the `Screen` blend mode read *Schermo* (the display) and is now *Scherma*; the Dodge/Burn/Sponge tool names were already Adobe's (*Scherma*, *Brucia*, *Spugna*) and stay. Pinned by a test |
| 2026-10-18 | minor | Russian: 1,229 dynamic labels added (filter and adjustment parameters, Camera Raw, lighting, History/Layers/paths menus, metadata, System Info, status templates, dialogs) with a terminology note ([localization-ru](localization-ru.md)) |
| 2026-10-22 | minor | Arabic row: paragraph direction has Auto / Left-to-right / Right-to-left buttons in the Paragraph panel and Type options bar (#6793); the visual RTL caret is still missing (TYPE-218-2 partial) |
| 2026-10-28 | minor | Arabic (`ar`) UI catalog: every measured key, six plural forms; layer names and the title bar no longer reversed on egui 0.37 |
| 2026-10-29 | minor | Swedish (`sv`) UI catalog: every measured key plus the dynamic labels, combined from #5985 and #9536 |
| 2026-10-29 | minor | Finnish (`fi`), Danish (`da`) and Norwegian Bokmål (`nb`) UI catalogs: every measured key plus the dynamic labels, combined from #5666, #6961, #7267, #8072 and #9025 |
