# Spanish terminology and dynamic UI labels

This note goes with the review and completion of the Spanish catalog
(`crates/ui-egui/src/i18n/es.tsv`). It already covered every key measured by
`cargo xtask i18n-coverage`; the terminology review aligned its wording with the vocabulary a
Spanish-speaking image editor already knows, fixed mistranslations, and made the register
consistent. A later pass added the 1,276 runtime-only labels it still lacked. No proprietary
translation resources, fonts, screenshots or user documents are included: every translation was
written from the meaning of PhotoCraft's own English label.

## Register and style

- **Formal "usted"**, as in Photoshop's Spanish UI: «Haga clic», «Seleccione», «¿Desea guardar…?».
  The catalog mixed «usted» and «tú»; every user-facing instruction now uses «usted».
- **Sentence case** after the first word, except panel and tool names, which keep their capital
  as in Photoshop: «Panel Carácter», «Herramienta Pluma».
- Quotation marks are «angulares»: «{name}».
- Neutral Spanish, matching Adobe's single Spanish catalog (for example «coma flotante»).

## Dynamic UI coverage

After merging the newer upstream catalog rows, the catalog has 4,010 entries. It covers the
complete Italian runtime key set and retains two Spanish profile-prompt labels that other
catalogs also use. The additional labels include filter and adjustment parameters and choices,
Filter Gallery names, Camera Raw, Lighting Effects, History and Layers controls, metadata,
dialogs, status wrappers and System Info labels. Before this pass, those controls fell back to
English because many are assembled dynamically and cannot be found by the static coverage scan.

A future generated label without a Spanish row shows in English. `cargo xtask i18n-coverage`
lists missing rows for the keys it scans, which don't include these generated labels. A few
unused legacy labels are retained so the complete catalogs stay aligned.

## Terminology

| English source | Spanish UI | Was |
|---|---|---|
| Artboard | Mesa de trabajo | Lienzo (clashed with *Canvas*) |
| Pattern (fill, overlay, stamp) | Motivo | Patrón |
| Preset | Ajuste preestablecido | Preajuste |
| Keyboard Shortcuts | Métodos abreviados de teclado | Atajos de teclado |
| Grid | Cuadrícula | Rejilla |
| Slice | Sector | Sección |
| Plug-ins | Plugins | Complementos |
| Scratch Disks | Discos de memoria virtual | Discos temporales |
| Puppet Warp | Deformación de posición libre | Deformación de clavos |
| Pin (Puppet Warp) | Chincheta | Pin |
| Dodge / Burn (tools) | Sobreexponer / Subexponer | **swapped** |
| Color Dodge / Color Burn | Sobreexposición / Subexposición de color | **swapped** |
| Linear Dodge (Add) / Linear Burn | Sobreexposición lineal (añadir) / Subexposición lineal | |
| Hard Light / Pin Light / Hard Mix | Luz fuerte / Luz focal / Mezcla definida | Luz dura / Luz puntual / Mezcla dura |
| Pass Through (group) | Pasar a través | Herencia |
| Clone Stamp / Pattern Stamp | Tampón de clonar / Tampón de motivo | Sello… |
| Feather | Calar | Desvanecer (clashed with *Fade*) |
| Fade | Transición | Desvanecer |
| Color Range | Gama de colores | Intervalo de colores |
| Select and Mask | Seleccionar y aplicar máscara | Selección y máscara |
| Selective Color | Corrección selectiva | Color selectivo |
| Shadows/Highlights; Highlights | Sombras/iluminaciones; Iluminaciones | Sombras/luces; Luces |
| Color Lookup | Consulta de colores | Búsqueda de color |
| Content-Aware Fill / Scale / Move | Relleno / Escala según el contenido; Mover con detección de contenido | Relleno de contenido… |
| Adaptive Wide Angle | Gran angular adaptable | Ángulo ancho adaptativo |
| Constrain proportions | Restringir proporciones | **Construir** proporciones |
| Render (filter menu) | Interpretar | Renderizar |
| Smart Sharpen | Enfoque suavizado | Enfoque inteligente |
| Blend If | Fusionar si | Combinar si |
| Knockout (blending options) | Extraer | Cobertura |
| Lights / Darks (Camera Raw) | Claros / Oscuros | Tonos claros / Tonos oscuros |
| Master (Hue/Saturation) | Todos | General |

Menu names that are also actions keep Photoshop's distinction: **Vista** (the *View* menu, not
«Ver»), **Volver** (*Revert*), **Borrar** (*Clear*), **Cortar** (*Trim*, unlike *Crop* «Recortar»),
**Descubrir** (*Reveal*), **Purgar** (*Purge*). Product and technology names (Photomerge, Camera
Raw, droplet, OpenType, sRGB, Wayland) are not translated.

## Out of scope

Engine-generated error details, status text and parameter help that do not pass through the UI
catalog remain English in every language (see
[`localization-parity.md`](localization-parity.md)). User document, layer and preset names and
automation command IDs also remain unchanged. The completion was contributed by
@gorevyoneticisi with OpenAI Codex as an AI contributor under the repository's MIT OR
Apache-2.0 terms; a review by a native speaker is still welcome.
