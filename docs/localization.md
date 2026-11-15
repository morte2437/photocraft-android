# UI localisation and live language switching

PhotoCraft uses the same Rust catalog implementation in the native egui shell and the
WebAssembly build: the `craft-i18n` crate (craft-libs) over the catalogs of
[storytold/craft-i18n](https://github.com/storytold/craft-i18n), shared by the Crafting Apps.
English source strings are lookup keys. UTF-8 TSV catalogs (`locales/<code>/photocraft.tsv`, over
the shared `common.tsv`) supply plain, contextual, command-ID and plural translations; missing
entries fall back to English. The build embeds them from the checkout `CRAFT_I18N_DIR` names, at
the commit in `craft-i18n.rev`; without it PhotoCraft builds and runs in English only.

## Supported languages

| Language | Preference code | Integer plural forms (CLDR) |
|---|---|---|
| English | `en` | one / other |
| 简体中文 | `zh-hans` | one form |
| 日本語 | `ja` | one form |
| 한국어 | `ko` | one form |
| Русский | `ru` | one / few / many |
| Українська | `uk` | one / few / many (teens use many) |
| Français | `fr` | singular for 0 and 1 / plural for 2+ |
| 繁體中文 | `zh-hant` | one form |
| Español | `es` | one / other |
| Čeština | `cs` | one / few / other |
| Deutsch | `de` | one / other |
| Bahasa Indonesia | `id` | one form |
| Português (Brasil) | `pt-br` | singular for 0 and 1 / plural for 2+ |
| Italiano | `it` | one / other |
| Polski | `pl` | one / few (2–4, not 12–14) / many |
| Ελληνικά | `el` | one / other |
| Nederlands | `nl` | one / other |
| Magyar | `hu` | one / other (both read the same: "3 réteg") |
| Tiếng Việt | `vi` | one form |
| ไทย | `th` | one form |
| தமிழ் | `ta` | one / other |
| Srpski (latinica) | `sr-latn` | one / few / other (teens use other) |
| Bosanski | `bs` | one / few / other (teens use other) |
| हिन्दी | `hi` | one (0 and 1) / other |
| Svenska | `sv` | one / other |
| Suomi | `fi` | one / other (partitive after every count but 1) |
| Dansk | `da` | one / other |
| Norsk bokmål | `nb` | one / other |
| ქართული | `ka` | one / other (both count-neutral: `შრე: {n}`) |
| עברית | `he` | one / two / other (two repeats other) |
| اردو | `ur` | one / other |
| Română | `ro` | one / few (0, 2–19, 101–119 …) / other (20–100, 120 …) |
| Azərbaycanca | `az` | one (1) / other (all other integer counts) |

Azerbaijani (`az`, Latin script) is registered with a complete menu catalog. Regional tags
such as `az-AZ` and `az-Latn-AZ` select it. The catalog includes the existing catalogs'
dynamic labels, contextual translations and two integer plural forms. Machine-assisted
drafts have edited core terminology; native-speaker review is pending. See
[`localization-az.md`](localization-az.md) for terminology, provenance and validation limits.

Languages registered with `complete_menus` cover the current menu labels, `tl!` literals, blend
mode names, and generated preference labels. A string a catalog lacks shows in English:
`cargo xtask i18n-report` lists the missing strings per language (with `CRAFT_I18N_DIR` set), and
`cargo xtask ci` prints that list as its last step without failing. Vietnamese (`vi`) is
selectable, resolves regional tags such as `vi-VN`, and is registered with `complete_menus`, its
coverage level with the other complete catalogs. This does not include every engine error or status message: those remain English, as do
automation command IDs and user-provided document, layer and preset names.

UI search and access-key comparisons use locale-aware dotted/dotless I rules for Azerbaijani
and Turkish. English command aliases remain searchable using English casing. See the
[Azerbaijani case-conversion audit](localization-az.md#ui-case-conversion-audit-2026-11-06).

## Switch without restarting

Open **Edit > Preferences > Interface > Language** and choose a language. The Preferences
dialog previews its title, controls and buttons in the chosen language. **Cancel** discards
the preview, **Apply** commits it without closing the dialog, and **OK** commits and closes.
The rest of the interface follows the committed preference on its next draw. Existing
documents, undo history and tools remain available throughout a language change.

The existing preference store saves the selection for future launches. `auto` follows the
native system locale; an unsupported code follows the same fallback. Regional tags such as
`fr-CA`, `ko-KR`, `vi-VN` and `zh-CN` resolve to the corresponding registered catalog. Traditional and
Simplified Chinese remain distinct. Every Portuguese locale (`pt`, `pt-BR`, `pt-PT`) uses the
Brazilian Portuguese catalog. The Serbian catalog (`sr-latn`) is written in Latin script
(Ekavian); every Serbian locale (`sr`, `sr-RS`, `sr-ME`, `sr-Latn-RS`, `sr_RS@latin`, and Cyrillic
`sr-Cyrl-RS`) resolves to it, and a preference saved as `sr` by an earlier version loads as
`sr-latn`.
The Bosnian catalog is written in Latin script (Ijekavian); every Bosnian locale (`bs-BA`,
`bs-Latn-BA`, `bs_BA.UTF-8`, and Cyrillic `bs-Cyrl-BA`) resolves to it.
Swedish locales from Sweden and Finland (`sv-SE`, `sv-FI`, `sv_FI.UTF-8`) use the Swedish catalog;
Finnish (`fi-FI`) uses the Finnish one. Every Norwegian locale (`nb-NO`, the macrolanguage `no`,
and Nynorsk `nn-NO` from POSIX systems) uses the Bokmål catalog.

### First launch and system language

With no saved preference, `interface.language` defaults to `auto`, so the first rendered
window uses the first supported system UI language. An older preferences file without this
field also uses Auto. If detection fails or none of the preferred languages is registered,
the UI uses English (`en`). A saved manual language choice takes precedence on later launches;
startup never overwrites it or writes the detected language into the preference file.

The native-only, pinned `sys-locale` dependency provides safe Rust access to the platform APIs:

- **Windows:** `GetUserPreferredUILanguages`, in the user's preferred order. This uses UI
  languages rather than the Region setting for dates and numbers; they can differ.
- **macOS:** `CFLocaleCopyPreferredLanguages`, in preferred order, without starting `defaults`
  or depending on a particular `.plist` representation.
- **Linux/BSD:** the platform's standard locale environment, including `LANGUAGE` lists.

`PHOTOCRAFT_LOCALE` is an optional single-tag override for reproducible launches; an unsupported
override falls back to English. Empty overrides use the system. Unix locale variables do not
override Windows/macOS UI-language preferences. Tags are read once per process (at most 64,
128 bytes each) and matched against the supported language registry. Changes to the
OS language list itself take effect at the next launch. The web build keeps its existing
English Auto fallback until browser-locale detection is implemented.

API implementation and licensing: [sys-locale](https://github.com/1Password/sys-locale).

Agents use the existing `prefs.set` command through CLI, MCP or the desktop control channel:

```json
{"method":"engine.execute","params":{"command":"prefs.set","params":{"path":"interface.language","value":"ko"}}}
```

Control requests are processed before synchronising the drawing language, so a request
updates the labels in the same rendered frame. Drawing language is local to the UI thread;
a scoped preview restores the previous language even if drawing unwinds. Menu and dialog
identity continues to use command IDs, independent of translated labels.

This is hot switching among the embedded catalogs. `build.rs` embeds the craft-i18n catalogs with
`include_str!`; a catalog change reaches PhotoCraft when `craft-i18n.rev` moves to a commit that
has it, and needs a rebuild. External catalog watching and file-resource hot reload are not
implemented.

## Right-to-left languages

Arabic, Hebrew, Persian, Urdu and the other right-to-left languages listed in
`i18n::dir_for_code` mirror the window's chrome when chosen: the menu bar starts at the right
with File, the toolbox docks on the right and the panel dock and its rail on the left, and the
title, options and status bars read from the right (`crates/ui-egui/src/direction.rs`). The
direction follows the language code, so a catalog needs no extra setting.

Not mirrored yet: panel interiors, dialogs, menu rows (shortcuts stay on the right), submenus
(they open to the right), document tabs and the canvas. egui 0.37 lays text out with the Unicode
Bidirectional Algorithm (each paragraph takes the direction of its first strong letter), so
catalogs, layer names and document names are handed to it in logical order, as typed; never
pre-reorder a label. The title line forces a left-to-right paragraph so "— PhotoCraft" stays at
the right of a right-to-left document name, and `bidi_text::elide` cuts a right-to-left name at
its logical end. Document text is laid out by the text engine.

Arabic (`ar`) is the first right-to-left catalog: Modern Standard Arabic, Western digits, the six
CLDR plural forms (zero|one|two|few|many|other). To check the mirrored layout with any language, launch with
`PHOTOCRAFT_UI_DIRECTION=rtl` (or `ltr` to force left-to-right); it is read once per process
and has no effect on the web build:

```sh
PHOTOCRAFT_UI_DIRECTION=rtl cargo run -p photocraft-ui-egui --example snapshot -- --out rtl.png
```

## Fonts

The native lazy CJK font loader prioritises the selected Japanese, Korean or Chinese script.
Switching languages rebuilds the font definitions and resets the existing loader, so fonts
removed from the definitions can be registered again. Idle frames keep their font cache.
Other languages retain the system's CJK fallback preference for document names and text.

Font files are not added to this repository. Native rendering uses installed system fonts
and the optional craft-fonts build input; see [Fonts](development.md#fonts-craft-fonts).
The web build can select every catalog, but CJK glyph delivery and automatic browser-locale
detection remain separate outstanding work. Switching catalogs does not supply missing fonts.

## Add or maintain a catalog

Translations live in craft-i18n, not here (its `docs/integration.md`). A PhotoCraft PR that adds
or changes interface text adds the English only; translators add the other languages to
`locales/<code>/photocraft.tsv` in craft-i18n PRs, and they show here once `craft-i18n.rev` moves
to a commit that has them. A missing translation never fails a build or a release: the string shows
in English. A few tests check named rows of the pinned catalogs (Hindi's Camera Raw row, for
one), so moving `craft-i18n.rev` to a commit that drops such a row fails that test until it is
updated.

1. Translate the meaning of the English source keys. Preserve `{name}` placeholders, escaped
   characters and the trailing `…` on dialog commands (an `@id` row without them is skipped and
   the label's translation shows). Use `@id` to disambiguate a command label and `@plural` for
   count-sensitive text, one form per CLDR category. Keep product names such as PhotoCraft.
2. A new language: craft-i18n gets `locales/<code>/photocraft.tsv` with its `@id language.name`
   row, then PhotoCraft registers its code in `i18n::LANGUAGES`.
3. Set `complete_menus` once `cargo xtask i18n-report` lists nothing missing for the language.

```sh
git clone https://github.com/storytold/craft-i18n ../craft-i18n
git -C ../craft-i18n checkout "$(tr -d '[:space:]' < craft-i18n.rev)"
export CRAFT_I18N_DIR="$PWD/../craft-i18n"
cargo test -p photocraft-ui-egui --locked
cargo xtask i18n-report
```

Render representative dialogs with the offscreen `snapshot` example, including a sequence
of language changes in one process. Inspect the PNGs for missing glyphs and truncated labels.
Translators are credited in craft-i18n's `ATTRIBUTION.md`; no proprietary localisation resources
are used.
