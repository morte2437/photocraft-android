# Georgian localization and terminology

This note goes with the Georgian catalog (`crates/ui-egui/src/i18n/ka.tsv`). It covers every key
`cargo xtask i18n-coverage` measures and the runtime labels the other complete catalogs carry
(filter and adjustment parameters, Camera Raw, panels, dialogs, status messages). Translations are
written from the meaning of PhotoCraft's English labels; no proprietary translation resources,
fonts, screenshots or user documents are included.

## Choosing Georgian

Open **Edit > Preferences > Interface > Language** (Georgian: **რედაქტირება › პარამეტრები ›
ინტერფეისი › ენა**) and choose **ქართული**. With `interface.language` set to `auto`, the system
locales `ka`, `ka-GE` and `ka_GE.UTF-8` resolve to Georgian.

## Register and style

Commands are verbal nouns (masdar: *შენახვა*, *გახსნა*, *შრის წაშლა*), not imperatives; prompts
and hints address the user politely (*დაწკაპეთ*, *გადაათრიეთ*). Product and technology names
(PhotoCraft, OpenType, CMYK, Lab) stay as they are, and the trailing `…` of dialog commands is kept.

## Georgian script

- Mkhedruli has no letter case, so nothing is capitalised and labels never use Mtavruli.
  The text engine's **All Caps** maps Mkhedruli to Mtavruli through Unicode `to_uppercase`, which
  is the Unicode-standard behaviour; there is no title-case feature that needs a Georgian guard.
- Nouns stay singular after a numeral, so the catalog uses one plural form and count-neutral
  wording: `{n} layers` is `შრე: {n}` (`plural_none` in the registry).
- UI font: PhotoCraft bundles no Georgian face. The fallback list in `crates/text/src/scripts.rs`
  uses system fonts: Noto Sans Georgian, Segoe UI, Sylfaen, DejaVu Sans and
  FreeSans. The web build has no system fonts, so it needs a Georgian face before Georgian can render
  there.
- PhotoCraft has no per-language date or number formatting, so nothing Georgian-specific applies.

## Terminology

Follow this table when adding or changing Georgian strings.

| English source | Georgian | Note |
|---|---|---|
| File / Edit / Select / View / Window | ფაილი / რედაქტირება / მონიშვნა / ხედი / ფანჯარა | menu bar |
| Image | გამოსახულება | the edited picture |
| Canvas / Canvas Size… | ტილო / ტილოს ზომა… | |
| Layer / Layers | შრე / შრეები | |
| Selection | მონიშნული | the selected area; the menu is მონიშვნა |
| Layer Mask / Vector Mask | შრის ნიღაბი / ვექტორული ნიღაბი | |
| Clipping Mask | მომჭრელი ნიღაბი | |
| Smart Object | ჭკვიანი ობიექტი | |
| Rasterize | რასტრირება | |
| Path / Work Path | ტრაექტორია / სამუშაო ტრაექტორია | not კონტური |
| Stroke | კონტური | outline of a shape or text |
| Channel / Alpha | არხი / ალფა | |
| Spot Color / Spot Channel | სპოტ-ფერი / სპოტ-ფერის არხი | |
| Adjustments | კორექცია | |
| Blend Mode / Blending Options | შერევის რეჟიმი / შერევის პარამეტრები | |
| Opacity | გაუმჭვირვალობა | |
| Preset / Presets | პრესეტი / პრესეტები | |
| Pattern | პატერნი | |
| Swatches | ფერის ნიმუშები | |
| Slice | ნაჭერი | |
| Crop | ჩამოჭრა | |
| Feather | კიდეების შერბილება | |
| Hue / Lightness | ფერის ტონი / სიღიავე | |
| Threshold | ზღვარი | |
| Brush / Eraser / Smudge | ფუნჯი / საშლელი / გადღაბნა | |
| Healing Brush | აღმდგენი ფუნჯი | |
| Reset | ჩამოყრა | |
| Paste | ჩასმა | |
| Workspace | სამუშაო სივრცე | |
| Foreground / Background Color | წინა პლანის ფერი / ფონის ფერი | |

## Not covered

Engine error and status messages, user document, layer and preset names, automation command IDs
and the default `Untitled` / `Background` names stay in English, as in every language.

## Maintenance

Add a row to `ka.tsv` for each new English label: `context<TAB>source<TAB>translation`, with
`@plural` rows holding one Georgian form and `@id` rows overriding a command id. Keep `{name}`
placeholders, `\n`, `…` and shortcut text exactly as in the source. Checks:

- `cargo test -p photocraft-ui-egui --lib i18n`: registry, locale tags, placeholders and catalog
  consistency.
- `cargo xtask i18n-coverage`: measured key coverage for `ka`.
