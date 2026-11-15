# Azerbaijani UI catalog

Azerbaijani (Latin script) is offered as **Azərbaycanca** (`az`). Regional preferences and
system locales such as `az-AZ`, `az_Latn_AZ.UTF-8` and `AZ-LATN-AZ` use the existing locale
matcher. English remains the source and fallback. The runtime registry has 29 languages
including English.

The catalog contains 4,606 entries, including 32 plural messages, 118 contextual entries and
eight command-ID overrides. The 2026-11-05 merge added the 29 new OpenType menu keys from
current `main`. The follow-up also adds five frame conversion/tool labels; all menu labels and the eight intentional command-ID overrides are checked.
The 2026-11-01 source scan covered 2,204/2,204 keys. Catalog completeness does not establish linguistic
quality or include untranslated engine errors.

## Terminology and provenance

| English | Azerbaijani |
|---|---|
| Layer | Qat |
| Layer mask | Qat maskası |
| Stroke (outline) | Kontur |
| Brush stroke | Fırça izi |
| Crop | Kəsmə |
| Rulers | Xətkeşlər |
| Path | Yol |
| Artboard | İş lövhəsi |
| Preset | Hazır ayar |
| Light (Camera Raw) | İşıq |
| Light (font weight) | İncə |

Google Translate supplied drafts of PhotoCraft's English labels. Core terminology, contexts
and plural messages were edited with OpenAI Codex. No proprietary localization resources were
used. The translations are MIT OR Apache-2.0, with attribution in `ATTRIBUTION.md` and
`crates/ui-egui/src/i18n/LICENSE-translations.txt`.

**Native Azerbaijani speaker review is requested before merge and has not been received.**
Please check the terminology, long help text, contextual meanings, counted nouns and the
natural wording of the catalog. Automated checks and AI-assisted edits do not replace that review.

## UI case-conversion audit (2026-11-06)

The catalog and rendered labels retain their original spelling, including `ƏəĞğIıİiÖöŞşÜüÇç`.
Search and access-key comparisons use the selected UI language. Azerbaijani and Turkish use
ICU4X case mapping: `i` ↔ `İ` and `ı` ↔ `I`, including contextual combining-dot handling.
English command names, menu paths used as English aliases and machine IDs keep English casing.

| UI path | Handling |
|---|---|
| Help menu search (`menus/search.rs`) | Localized labels and paths use their language; English aliases are ranked separately. |
| Command palette (`dialogs/palette.rs`) | Localized fuzzy and exact matches use their language; English aliases remain available. |
| Preferences and Keyboard Shortcuts (`prefs/prefs_ui/`) | Translated labels and section titles use localized casing; English schema paths, IDs and shortcut syntax stay invariant. |
| Dropdown typing (`widgets/type_ahead.rs`) | Labels and typed prefixes use localized casing; raw prefix text is retained so a following combining dot can complete an I. |
| Menu and dialog access keys (`menus/menu_keys.rs`, `dialogs/discard_ui.rs`) | Compare displayed letters using localized uppercase. Only ASCII key codes are assigned; an English key suffix is shown when required. Labels are not rewritten. |
| A menu key over the zoom field (`canvas/zoom_field.rs`) | Consume the text emitted by the logical I key for either Turkic keyboard letter, preventing it from leaking into the zoom field. |

The audit searched `ui-egui` and the native/web app shells for lower/uppercase operations,
case-insensitive comparisons and mnemonic handling. English schema/command humanization runs
before translation and remains language-independent. Native macOS menu labels are passed through
verbatim (only the mnemonic escape character is escaped). Language-picker sort keys are a fixed
accent-folded ordering of native names. File extensions, color/format tokens, font metadata,
and user-provided document/layer/preset names are not catalog translations and retain their
existing matching/identity rules. This does not audit third-party widget internals or document-text casing.

References: [Unicode special casing](https://www.unicode.org/faq/casemap_charprop.html),
[ICU4X case mapper](https://docs.rs/icu_casemap/2.3.0/icu_casemap/struct.CaseMapperBorrowed.html).
## Plurals

The existing one/other integer rule selects `one` at 1 and `other` at all other `u64` counts.
Counted nouns remain singular in both forms: `1 qat`, `20 qat`. Tests include zero, teens,
hundreds, thousands and `u64::MAX`.

## Validation

Current catalog scan (2026-11-06, merged main `7904eee76`): 4,606 entries, all 669 canonical
menu keys and eight intentional command-ID overrides. Current source coverage is
2,268/2,358 keys (96.18%); 90 newer strings fall back to English. The merge added six current
menu labels so the existing menu coverage regression remains complete.

Current case-conversion regressions exercise the actual shortcut grid and dropdown input,
menu underline layout, localized/English search, preference matching, combining dots and
the two distinct I pairs. Executed on Windows x64 with Rust 1.99.1/MSVC:

- All 15 Azerbaijani-focused checks passed: the six original language checks plus nine
  casing/input regressions. The Turkic helper and mnemonic layout also test Turkish.
- All eight additional regression groups passed: 40 i18n unit checks, 22 menu-key checks,
  four Help-search checks, eight palette checks, four dropdown checks, 69 discard-dialog
  checks, six zoom-field checks and one existing Preferences-search check (154 checks;
  some overlap with the Azerbaijani-focused run).
- Formatting, Clippy (`--all-targets -- -D warnings`), 31-crate layering, wasm (24 crates
  plus optional HEIF) and the offscreen snapshot build passed.
- Visually inspected before/after CPU captures at 1440 × 900 logical pixels and 1.5× scale.
  Before the fix, `interfeys` found no Preferences section and `ŞƏKİL ÖLÇÜSÜ` found no
  shortcut row. After the fix, they show Interface settings and Image Size respectively.
- The full UI gate was rerun with one test thread and is **not green**: unit and integration
  processes again exited `0xc0000005` at `omitting_an_occluded_checker_preserves_cpu_canvas_pixels`
  and `batch_and_processor_real_controls_write_the_chosen_webp_quality`, the same points
  observed on 2026-11-05. The example test and doctest targets passed.

### Earlier full-suite limitations

On 2026-11-05 the six Azerbaijani regressions, 40 i18n unit checks, formatting, Clippy,
layering, wasm and snapshot build passed. Editor/preferences CPU captures were visually
inspected. The complete i18n subset had 61 passed and eight CJK/Arabic missing-glyph failures
without the optional craft-fonts input. The full UI unit/integration processes exited
`0xc0000005` (`STATUS_ACCESS_VIOLATION`) even with one test thread. These are historical
results, not a successful full-suite validation of the current revision. Native-speaker
review remains pending. CPU captures do not establish hardware-GPU or native window/input acceptance.
