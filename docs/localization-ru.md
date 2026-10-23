# Russian terminology and dynamic UI labels

This note goes with the completion of the Russian catalog (`crates/ui-egui/src/i18n/ru.tsv`).
The catalog already covered every key `cargo xtask i18n-coverage` measures, but it lacked the
labels that only exist at runtime and that other catalogs (Italian, Serbian, Czech, Hungarian)
already translate: filter and adjustment parameter names, Camera Raw panels, lighting effects,
History / Layers / paths / channels menu labels, metadata fields, status messages, System Info
and dialog text. In Russian those showed in English.
They are now translated, reusing the vocabulary the catalog already uses elsewhere so the same
term reads the same everywhere. Translations are written from the meaning of PhotoCraft's English
labels; no proprietary translation resources, fonts, screenshots or user documents are included.

## Register and style

Commands are imperatives in the second person plural (*Сохранить*, *Удалить*, *Заново*);
where the catalog already uses a noun it is kept (*Уровни…*, *Кривые…*). Sentence case after the
first word, as in the rest of the catalog. Ellipses on dialog commands are preserved (`Ветер…`
→ `Ветер…`). Decimals use a comma (the file format's own labels keep their dots, e.g.
`Landscape, 10 x 8` stays `Альбомная, 10 x 8`). Latin product and technology names are not
translated (PhotoCraft, Camera Raw, RGB, CMYK, Lab, LUT, WebP); Camera Raw keeps `.xmp`; the
`HDR`, `HSL`, `HSB` and `W3C` acronyms and the `sGray`/`Pro Photo` identifiers stay Latin.

## Terminology

Existing choices were followed rather than re-invented, so the new rows use:

| English source | Russian | Note |
|---|---|---|
| Highlight(s) | Света | already in the catalog (`Highlights` → `Света`) |
| Midtone(s) | Полутона | already in the catalog |
| Shadow(s) | Тени | pairs with *Света* |
| Feather | Растушёвка | never *перо* |
| Fuzziness | Разброс | Color Range's tolerance |
| Dither | Дизеринг | |
| Pattern | Узор | also in *Узорный штамп* |
| Preset | Набор | *Набор кистей*, *набор инструментов* |
| Guide | Направляющая | *Схема направляющих* |
| Artboard | Монтажная область | |
| Workspace | Рабочая среда | |
| Stroke (contour) | Обводка | painterly strokes stay *штрихи*/*мазки* |
| Stroke (paint param) | Штрихи / мазки | `Strokes` → `Штрихи` |
| Amount | Величина | `Штрихи`, never *количество*, when it is a strength |
| Fill Opacity | Непрозрачность заливки | distinct from layer *Непрозрачность* |
| Feather Radius | Радиус растушёвки | |
| Sharpen | Резкость | `Smart Sharpen` → *Умная резкость*, `Unsharp Mask` → *Контурная резкость* |
| Noise | Шум | `Reduce Noise` → *Уменьшить шум* |
| Content-Aware | С учётом содержимого | |
| Camera Raw | Camera Raw | the panel name stays Latin (as German/Spanish do; Greek translates it) |
| Place Embedded / Linked | Поместить встроенный / связанный | |
| Rasterize | Растрировать | |

## Native review

The review was AI-assisted and each row was checked against the meaning of PhotoCraft's own
English label. A review by a native speaker is still welcome.

## Not covered

Engine error and status messages, user document, layer and preset names, and automation command
IDs stay in English, as in every language ([`localization-parity.md`](localization-parity.md)).
A few dynamic labels the other catalogs carry are not used by the current code; they are
translated to stay aligned with the other catalogs.

## Validation

- Every filter and adjustment parameter, symbolic choice and gallery name has a Russian row, as do
  the rasterize prompts, hardware status lines and composed status templates.
- `cargo test -p photocraft-ui-egui --lib`: 2,998 passed, 0 failed.
- `cargo xtask i18n-coverage`: `ru` remains at 100% of the measured keys.
- The offscreen `snapshot` example with `PHOTOCRAFT_LOCALE=ru` renders the dialogs with Russian
  labels, without missing glyphs or clipped text.
