# Localization parity

> **Last reviewed:** 2026-11-08 · **Last updated:** 2026-11-08 · **Change:** minor (the catalogs moved to storytold/craft-i18n and are measured with `cargo xtask i18n-report`; Serbian is `sr-latn`; coverage re-measured on the merged tree, 2,449 keys; Arabic 92.1%; 35 registered languages; Hindi partial; Georgian, Azerbaijani, Hebrew, Romanian, Tamil, Thai and Urdu measured; previously minor: Swedish, Finnish, Danish and Norwegian Bokmål UI catalogs) · **Target:** Adobe Photoshop 2026

Per-language status of PhotoCraft's UI. How catalogs work and how to add one:
[`localization.md`](localization.md). Language notes: [Korean](localization-ko.md), [Spanish](localization-es.md),
[Simplified Chinese](localization-zh-hans.md), [Italian](localization-it.md), [Russian](localization-ru.md), [Georgian](localization-ka.md).

**How measured.** `cargo xtask i18n-coverage` (now `cargo xtask i18n-report`, `xtask/src/i18n_report.rs`, which reads the craft-i18n catalogs in `CRAFT_I18N_DIR`) defines the key
universe: `tl!("…")` literals and `trn(` plurals in `ui-egui` (tests excluded), menu-catalog
labels and path segments, `menus.rs` UI commands, and engine `CommandSpec` labels and menu paths:
**2,449 English keys** on 2026-11-08 after merging main (2,431 earlier that day, 2,428 on 2026-11-07, 2,400 on this branch before that day's merge, 2,389 earlier, 2,369 on main earlier that day, 2,272 on 2026-11-05, 2,199 on 2026-10-31, 2,202 on 2026-10-29, 1,982 on 2026-10-22, 1,918 on 2026-10-21, 1,772 on 2026-10-12, 1,608 on 2026-10-10). Each catalog (`locales/<code>/photocraft.tsv` in craft-i18n)
is checked against that universe by the xtask itself; "rows" are the non-comment, non-blank lines
of the `.tsv`. Catalogs hold more rows than the universe because they also translate dynamic `tl!(expr)` strings (~159 call
sites the tool can't enumerate). **Not covered anywhere:** engine error and status messages and
filter parameter help, which are English in every language; so no translated language is `full`. The report
also lists, per language, the keys its catalog lacks; those show in English. `cargo xtask ci` prints
it as its last step and never fails on it.

**Photoshop 2026** ships UI language packs for cs, da, de, en, es, fi, fr (and fr-CA), hu, it,
ja, ko, nb, nl, pl, pt-BR, ru, sk, sv, tr, uk, zh-Hans, zh-Hant (24 `.lproj` markers; one pack is
installed per Creative Cloud install, here `en_US`). It has no Hindi, Arabic, Indonesian or
Vietnamese UI; Arabic and Hebrew users get English "Middle East" builds with the World-Ready
composer, and its text engine handles RTL, Devanagari and Indic scripts.

**Summary.** 1 of the 12 key languages at 100% of measured keys (English), 10 at 92.1–93.7%
(Arabic at 92.1%, 194 keys in English; Simplified Chinese, Spanish, French, Portuguese (Brazil), Indonesian, Japanese,
German, Korean, Vietnamese at 93.3–93.7%, 155–164 keys each still show in English; status `partial` also
because of the engine messages), Hindi partial (400 of 2,449, 16.3%), plus the other shipped languages below. Weighted by
speakers, ~55% ready. Remaining: 90–143 h, plus native-speaker review (recorded only for Italian,
2026-10-12).

## The twelve key languages

| Language | Code | UI strings (measured keys) | Catalog rows | Dialogs / tooltips / help | Script support | Native review | Status | To `full` |
|---|---|---|---:|---|---|---|---|---:|
| English | `en` | 2,449 / 2,449 (100%) | source | all | Latin | – | full | 0 h |
| Simplified Chinese | `zh-hans` | 2,285 / 2,449 (93.3%) | 3,188 | dialogs, tooltips yes; engine messages English | CJK fonts (lazy loader), IME preedit/commit, vertical type | no | partial | 8–12 h |
| Spanish | `es` | 2,293 / 2,449 (93.6%) | 4,430 | as above; complete dynamic UI key set | Latin | no | partial | 6–10 h |
| Hindi | `hi` | 400 / 2,449 (16.3%) | 629 | Top menus, toolbox tools, Camera Raw, dialogs, plurals | Devanagari shaping in the text engine (parley/HarfRust); no Devanagari fallback font (TYPE-218-3) | no | partial | 15–20 h |
| Arabic | `ar` | 2,255 / 2,449 (92.1%) | 3,487 | as above; combined from the community catalogs with one term per concept | Arabic shaping and bidi in the UI (egui 0.37) and the text engine, Arabic fallback font; RTL shell layout but not inside panels (DIST-220-4), paragraph direction buttons but no visual RTL caret (TYPE-218-2 partial) | no | partial | 15–25 h |
| French | `fr` | 2,292 / 2,449 (93.6%) | 4,517 | as above | Latin | no | partial | 6–10 h |
| Portuguese (Brazil) | `pt-br` | 2,294 / 2,449 (93.7%) | 4,411 | as above | Latin; every `pt` locale uses it | no | partial | 6–10 h |
| Indonesian | `id` | 2,288 / 2,449 (93.4%) | 3,524 | as above | Latin | no | partial | 6–10 h |
| Japanese | `ja` | 2,291 / 2,449 (93.5%) | 4,141 | as above | CJK fonts, IME (Windows IME broken, #590), vertical type (tategaki) | no | partial | 10–16 h |
| German | `de` | 2,287 / 2,449 (93.4%) | 3,532 | as above | Latin | no | partial | 6–10 h |
| Korean | `ko` | 2,291 / 2,449 (93.5%) | 4,070 | as above; terminology from Photoshop's Korean UI ([notes](localization-ko.md)) | Hangul fonts, IME | no | partial | 6–10 h |
| Vietnamese | `vi` | 2,291 / 2,449 (93.5%) | 3,960 | as above | Latin with stacked diacritics (shaping supported) | no | partial | 6–10 h |

## Other shipped languages

23 more registered languages (35 with English), measured 2026-11-08, status `partial` for the same reason. Swedish, Finnish, Danish and Norwegian Bokmål miss 178 of the 2,449 measured keys, Bosnian 183, Georgian 176; the other eleven below miss 155–164:
Traditional Chinese `zh-hant` (3,143 rows), Russian `ru` (4,580; also every dynamic label the
other catalogs translate, [notes](localization-ru.md)), Ukrainian `uk` (4,122), Czech `cs` (4,205),
Polish `pl` (4,108), Greek `el` (4,216), Dutch `nl` (4,290), Hungarian `hu` (4,438), Italian `it` (4,580;
also every dynamic label the other catalogs translate; reviewed by a native speaker,
[notes](localization-it.md)), Turkish `tr` (4,294 rows). Georgian `ka` (4,642 rows, 2,273 of 2,449 measured keys; Georgian UI fallback font: Noto Sans Georgian, Segoe UI, Sylfaen, SF Georgian, DejaVu Sans; on the web the first served font for `Geor`; [notes](localization-ka.md)). Serbian `sr` (4,527 rows, Latin script,
Ekavian, added 2026-10-12) covers the union of the other catalogs' dynamic labels; its one/few/many plurals reuse
the Russian rule. Swedish `sv` (4,603 rows, added 2026-10-28) covers the
other catalogs' dynamic labels; combined from the community catalogs in #5985 and #9536 with one
term per concept; the contested terms follow a native speaker's questionnaire answers. Finnish
`fi` (4,603 rows, combined from #5666 and #6961), Danish `da` (4,603 rows, based on #7267 and
completed for the current UI) and Norwegian Bokmål `nb` (4,603 rows, combined from #8072 and #9025;
`no` and `nn` locales resolve to it), all added 2026-10-29, cover the same keys with one term per
concept; none has had a native-speaker review yet. Bosnian `bs` (4,622 rows, Latin script,
Ijekavian, added 2026-10-30) covers the same dynamic labels; its one/few/other plurals reuse the Russian
rule, and Cyrillic `bs` locales read it too. It is AI-assisted, shares its wording with
storytold/craft-i18n `locales/bs/`, and has not had a native-speaker review yet.

Azerbaijani `az` (Latin script, 2,269 of 2,449 measured keys) brought the runtime registry to 29 languages including
English when it landed; it now has 35. Its catalog has 4,623 entries, including the initial dynamic-label key union, current OpenType menus and five frame labels
and 32 plural messages. It uses one/other integer forms, with singular counted nouns.
Complete menu keys, regional locale matching and English fallback are covered by tests.
Native-speaker review remains pending; validation results and limits are recorded in the
[language notes](localization-az.md). Readiness estimates and G13 remain unchanged.

Hebrew `he`, Romanian `ro` and Urdu `ur` are measured at 2,334 of 2,449 keys (95.3%), Thai `th` at 2,211 (90.3%) and Tamil `ta` at 2,133 (87.1%).

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
| 2026-11-08 | trivial | Catalogs moved to storytold/craft-i18n (`locales/<code>/photocraft.tsv`), looked up through the `craft-i18n` crate; Serbian is `sr-latn`; `cargo xtask i18n-report` replaces `i18n-coverage`; coverage unchanged |
| 2026-11-06 | minor | Azerbaijani/Turkish UI search, dropdown prefixes and access keys use locale-aware I casing; 4,606 Azerbaijani entries and current menu coverage; native review requested and pending |
| 2026-11-05 | minor | Preserved Bosnian while merging current main; 29 registered languages, 17 outside the table; Azerbaijani has 4,606 entries, including 29 current OpenType menu keys and checks for all eight command-ID overrides; native review pending |
| 2026-11-01 | minor | Azerbaijani UI catalog, 4,566 entries; regional tags and one/other plurals; PR narrowed to Azerbaijani, 28 registered languages and 16 outside the table; native review pending; validation limits in the language notes; readiness estimates unchanged |
| 2026-10-10 | major | First version, measured from the catalogs with the `i18n-coverage` rules |
| 2026-10-10 | minor | Right-to-left shell layout (menu bar, title, options and status bars, toolbox and dock sides) |
| 2026-10-11 | minor | Czech dynamic status/notice and hardware text, render labels and Navigator zoom context; engine messages remain English |
| 2026-10-11 | minor | Czech review follow-ups: English System Info diagnostics, id-aware shortcut labels, display-only auto translation, warning punctuation and wording; catalog row count refreshed |
| 2026-10-11 | minor | Italian: 1,287 dynamic labels added (filter parameters, Camera Raw, lighting, panels, dialogs) and terminology made consistent |
| 2026-10-12 | minor | Added Turkish (`tr`) catalog with 100% measured UI coverage |
| 2026-10-12 | minor | Spanish: 1,276 runtime-only labels added (filter and adjustment parameters, Camera Raw, panels, dialogs and status wrappers); generated-label coverage test added |
| 2026-10-12 | minor | Italian native-speaker review: mistranslations (Trap, HDR Toning, Display Text) and phrasing fixed |
| 2026-10-12 | minor | Serbian (`sr`, Latin, Ekavian) catalog: every measured key plus the dynamic labels of the other catalogs |
| 2026-10-12 | minor | Re-measured with `cargo xtask i18n-coverage`: 1,772 keys; Vietnamese catalog shipped (100%, replaces "none"); Simplified Chinese, French, Indonesian and Korean at 1,758 of 1,772; plug-in menu rows stay in Filter › Plug-ins in every language (#2540, #2627) and installed Japanese fonts are listed on Linux (#2330, #2961) |
| 2026-10-13 | minor | Italian: the `Screen` blend mode read *Schermo* (the display) and is now *Scherma*; the Dodge/Burn/Sponge tool names were already Adobe's (*Scherma*, *Brucia*, *Spugna*) and stay. Pinned by a test |
| 2026-10-14 | minor | Re-measured with `cargo xtask i18n-coverage` after merging main: 1,871 keys; Simplified Chinese, French, Indonesian and Korean at 1,857, the other key languages at 100%; catalog rows re-counted; other shipped languages refreshed (Hungarian listed, Polish at 1,904) |
| 2026-10-18 | minor | Russian: 1,229 dynamic labels added (filter and adjustment parameters, Camera Raw, lighting, History/Layers/paths menus, metadata, System Info, status templates, dialogs) with a terminology note ([localization-ru](localization-ru.md)) |
| 2026-10-21 | minor | Re-measured current main: 1,918 keys, 1,904 for French, Indonesian, Korean, Polish and Simplified Chinese; catalog rows recounted |
| 2026-10-22 | minor | Arabic row: paragraph direction has Auto / Left-to-right / Right-to-left buttons in the Paragraph panel and Type options bar (#6793); the visual RTL caret is still missing (TYPE-218-2 partial) |
| 2026-10-22 | minor | Georgian (`ka`) catalog |
| 2026-10-28 | minor | Arabic (`ar`) UI catalog: every measured key, six plural forms; layer names and the title bar no longer reversed on egui 0.37 |
| 2026-10-29 | minor | Swedish (`sv`) UI catalog: every measured key plus the dynamic labels, combined from #5985 and #9536 |
| 2026-10-29 | minor | Finnish (`fi`), Danish (`da`) and Norwegian Bokmål (`nb`) UI catalogs: every measured key plus the dynamic labels, combined from #5666, #6961, #7267, #8072 and #9025 |
| 2026-10-30 | minor | Bosnian (`bs`, Latin, Ijekavian) catalog: the same 4,552 keys as the newest catalogs, `bs-*` locales (Cyrillic included) and one/few/other plurals |
| 2026-11-05 | minor | Re-measured the merged tree with the `xtask/src/i18n_coverage.rs` reporter: 2,272 keys; Arabic at 96.9% (2,202); Bosnian, Swedish, Finnish, Danish and Norwegian Bokmål at 2,216 (Bosnian) and 2,212; Simplified Chinese, Spanish, French, Portuguese (Brazil), Indonesian, Japanese, German, Korean and Vietnamese at 2,195–2,203 (69–77 keys still in English); Hindi 328; catalog rows recounted |
| 2026-11-07 | minor | Re-measured current main with `cargo xtask i18n-coverage`: 2,369 keys; Arabic 2,241 (94.6%); Simplified Chinese, Spanish, French, Portuguese (Brazil), Indonesian, Japanese, German, Korean and Vietnamese at 2,264–2,272 (97–105 keys still in English); Hindi 387; Azerbaijani, Hebrew, Romanian, Tamil, Thai and Urdu added to the measurements; catalog rows recounted |
| 2026-11-07 | minor | Re-measured the tree merged with main using `cargo xtask i18n-coverage`: 2,389 keys; Arabic 2,252 (94.3%); Simplified Chinese, Spanish, French, Portuguese (Brazil), Indonesian, Japanese, German, Korean and Vietnamese at 2,275–2,284 (105–114 keys still in English); Hindi 398; Hebrew, Romanian and Urdu 2,332, Thai 2,209, Tamil 2,131; catalog rows recounted |
| 2026-11-07 | minor | Re-measured after merging current main (Georgian catalog) with `cargo xtask i18n-coverage`: 2,400 keys; Arabic 2,252 (93.8%); Simplified Chinese, Spanish, French, Portuguese (Brazil), Indonesian, Japanese, German, Korean and Vietnamese at 2,277–2,286 (114–123 keys still in English); Hindi 398; Georgian 2,271; Hebrew, Romanian and Urdu 2,332, Thai 2,209, Tamil 2,131; catalog rows recounted |
| 2026-11-07 | minor | Re-measured the merged tree with `cargo xtask i18n-coverage`: 2,428 keys; Arabic 2,252 (92.8%); Simplified Chinese, Spanish, French, Portuguese (Brazil), Indonesian, Japanese, German, Korean and Vietnamese at 2,282–2,291 (137–146 keys still in English); Hindi 398; Georgian 2,271; Hebrew, Romanian and Urdu 2,332, Thai 2,209, Tamil 2,131; catalog rows recounted |
| 2026-11-08 | minor | Re-measured the tree merged with main using `cargo xtask i18n-coverage`: 2,431 keys; Arabic 2,252 (92.6%); Simplified Chinese, Spanish, French, Portuguese (Brazil), Indonesian, Japanese, German, Korean and Vietnamese at 2,282–2,291 (140–149 keys still in English); Hindi 399; Georgian 2,270; Azerbaijani 2,266; Hebrew, Romanian and Urdu 2,331, Thai 2,208, Tamil 2,130; 35 registered languages; catalog rows recounted |
| 2026-11-08 | minor | Re-measured again after merging current main using `cargo xtask i18n-coverage`: 2,449 keys; Arabic 2,255 (92.1%); Simplified Chinese, Spanish, French, Portuguese (Brazil), Indonesian, Japanese, German, Korean and Vietnamese at 2,285–2,294 (155–164 keys still in English); Hindi 400; Georgian 2,273; Azerbaijani 2,269; Hebrew, Romanian and Urdu 2,334, Thai 2,211, Tamil 2,133; catalog rows unchanged |
