# Actions files (`.atn`)

PhotoCraft loads and saves action sets in the reference application's `.atn` format: Load Actions and
Save Actions in the Actions panel menu, `actions.load` / `actions.save` for agents, and
`photocraft batch --actions set.atn` on the command line. Reading and writing the file lives in
`photocraft-psd` (`crates/psd/src/atn.rs`, version 16 sets, exact round trip); the mapping between the
application's events and PhotoCraft commands lives in `photocraft-engine`
(`crates/engine/src/cmd/atn_cmds/`, table `EVENTS` in `events.rs`).

A step whose event is in the table below loads as the PhotoCraft command it names, plays, and saves back
as the same event. Any other step loads as a kept step (`actions.photoshopStep`), and Save Actions writes
it back unchanged. Playback stops at an enabled kept step with its name, so an action never silently skips
work. Kept steps with no effect on the document (tool selections and the like) are imported switched off,
so playback skips them.

This page covers the events PhotoCraft maps today, the settings some of them still lose, how steps name
layers and documents, newer and older events and files, and a measured reference for the events it does not
map yet, with everything needed to implement them without
the reference application.

## Supported events (117)

| Code | String ID | Label | PhotoCraft command(s) |
|---|---|---|---|
| `AdNs` | addNoise | Add Noise | `filter.noise.addNoise` |
| `Algn` | align | Align | `layer.align.leftEdges`, `layer.align.horizontalCenters`, `layer.align.rightEdges`, `layer.align.topEdges`, `layer.align.verticalCenters`, `layer.align.bottomEdges` |
| `AppI` | applyImageEvent | Apply Image | `image.applyImage` |
| (string ID only) | applyComp | Apply Layer Comp | `layerComp.apply` |
| (string ID only) | applyLocking | Apply Locking | `layer.lockLayers` |
| (string ID only) | assignProfile | Assign Profile | `edit.assignProfile` |
| (string ID only) | autoColor | Auto Color | `image.autoColor` |
| (string ID only) | blackAndWhite | Black & White | `image.adjustments.blackWhite` |
| `Blr ` | blur | Blur | `filter.blur.blur` |
| `BlrM` | blurMore | Blur More | `filter.blur.blurMore` |
| `Brdr` | border | Border | `select.modify.border` |
| (string ID only) | boxblur | Box Blur | `filter.blur.boxBlur` |
| `BrgC` | brightnessEvent | Brightness/Contrast | `image.adjustments.brightnessContrast` |
| `CnvS` | canvasSize | Canvas Size | `image.canvasSize` |
| `ChnM` | channelMixer | Channel Mixer | `image.adjustments.channelMixer` |
| (string ID only) | clearEvent | Clear | `select.editInQuickMaskMode`, `history.clear` |
| `Cls ` | close | Close | `file.close` |
| (string ID only) | collapseAllGroupsEvent | Collapse All Groups | `layer.setExpanded` |
| `ClrB` | colorBalance | Color Balance | `image.adjustments.colorBalance` |
| `ClrR` | colorRange | Color Range | `select.colorRange` |
| `Cntc` | contract | Contract | `select.modify.contract` |
| `CnvM` | convertMode | Convert Mode | `image.mode.rgb`, `image.mode.grayscale`, `image.mode.cmyk`, `image.mode.lab`, `image.mode.multichannel`, `image.mode.bits8`, `image.mode.bits16`, `image.mode.bits32`, `image.mode.bitmap` |
| (string ID only) | convertToProfile | Convert to Profile | `edit.convertToProfile` |
| (string ID only) | newPlacedLayer | Convert to Smart Object | `layer.smartObjects.convertToSmartObject` |
| `copy` | copyEvent | Copy | `edit.copy` |
| (string ID only) | copyEffects | Copy Layer Style | `layer.layerStyle.copyLayerStyle` |
| `CpyM` | copyMerged | Copy Merged | `edit.copyMerged` |
| `GrpL` | groupEvent | Create Clipping Mask | `layer.createClippingMask` |
| `Crop` | crop | Crop | `image.crop` |
| `Crst` | crystallize | Crystallize | `filter.pixelate.crystallize` |
| `Crvs` | curves | Curves | `image.adjustments.curves` |
| `cut ` | cut | Cut | `edit.cut` |
| `Dfrg` | defringe | Defringe | `layer.matting.defringe` |
| `Dlt ` | delete | Delete | `layer.delete`, `edit.clear`, `layer.layerMask.delete`, `layer.smartFilter.deleteFilterMask` |
| `Dstt` | desaturate | Desaturate | `image.adjustments.desaturate` |
| (string ID only) | selectNoLayers | Deselect Layers | `select.deselectLayers` |
| `Dspc` | despeckle | Despeckle | `filter.noise.despeckle` |
| `Dfs ` | diffuse | Diffuse | `filter.stylize.diffuse` |
| `Dplc` | duplicate | Duplicate | `layer.duplicate`, `image.duplicate` |
| `DstS` | dustAndScratches | Dust & Scratches | `filter.noise.dustAndScratches` |
| (string ID only) | placedLayerEditContents | Edit Contents | `layer.smartObjects.editContents` |
| `Embs` | emboss | Emboss | `filter.stylize.emboss` |
| `Eqlz` | equalize | Equalize | `image.adjustments.equalize` |
| `Exch` | exchange | Exchange | `tools.swapColors` |
| `Expn` | expand | Expand | `select.modify.expand` |
| `Expr` | export | Export | `file.export.saveForWebLegacy` |
| `Exps` | exposure | Exposure | `image.adjustments.exposure` |
| `Fthr` | feather | Feather | `select.modify.feather` |
| `Fl  ` | fill | Fill | `edit.fill`, `paint.bucket` |
| `GEfc` | filterGallery | Filter Gallery | `filter.gallery.spatter` |
| `FltI` | flattenImage | Flatten Image | `layer.flattenImage` |
| `Flip` | flip | Flip | `image.imageRotation.flipCanvasHorizontal`, `image.imageRotation.flipCanvasVertical`, `edit.transform.flipHorizontal`, `edit.transform.flipVertical` |
| `GsnB` | gaussianBlur | Gaussian Blur | `filter.blur.gaussianBlur` |
| `GdMp` | gradientMapEvent | Gradient Map | `image.adjustments.gradientMap` |
| `Grow` | grow | Grow | `select.grow` |
| `Hd  ` | hide | Hide | `layer.hideLayers` |
| `HghP` | highPass | High Pass | `filter.other.highPass` |
| `HStr` | hueSaturation | Hue/Saturation | `image.adjustments.hueSaturation` |
| (string ID only) | conditional | If | `actions.conditional` |
| `ImgS` | imageSize | Image Size | `image.imageSize` |
| `Invs` | inverse | Inverse | `select.inverse` |
| `Invr` | invert | Invert | `image.adjustments.invert` |
| `CpTL` | copyToLayer | Layer Via Copy | `layer.new.layerViaCopy` |
| `CtTL` | cutToLayer | Layer Via Cut | `layer.new.layerViaCut` |
| `Lvls` | levels | Levels | `image.adjustments.levels`, `image.autoTone`, `image.autoContrast` |
| `Mk  ` | make | Make | `layer.new.layer`, `layer.new.group`, `layer.groupLayers`, `layer.duplicate`, `layer.new.layerViaCopy`, `layer.new.layerViaCut`, `layer.newFillLayer.solidColor`, `layer.newFillLayer.pattern`, `layer.newAdjustmentLayer.levels`, `layer.newAdjustmentLayer.curves`, `layer.newAdjustmentLayer.hueSaturation`, `layer.newAdjustmentLayer.brightnessContrast`, `layer.newAdjustmentLayer.threshold`, `layer.newAdjustmentLayer.posterize`, `layer.newAdjustmentLayer.invert`, `layer.newAdjustmentLayer.colorBalance`, `layer.newAdjustmentLayer.vibrance`, `layer.newAdjustmentLayer.exposure`, `layer.newAdjustmentLayer.photoFilter`, `layer.newAdjustmentLayer.blackWhite`, `layer.newAdjustmentLayer.channelMixer`, `layer.newAdjustmentLayer.selectiveColor`, `layer.newAdjustmentLayer.gradientMap`, `layer.layerMask.revealAll`, `layer.layerMask.hideAll`, `layer.layerMask.revealSelection`, `layer.layerMask.hideSelection`, `file.new`, `file.newFromClipboard`, `history.newSnapshot` |
| (string ID only) | animationFramesFromLayers | Make Frames from Layers |  |
| `Mxm ` | maximum | Maximum | `filter.other.maximum` |
| `Mdn ` | median | Median | `filter.noise.median` |
| `Mrg2` | mergeLayersNew | Merge Layers | `layer.mergeLayers` |
| `MrgV` | mergeVisible | Merge Visible | `layer.mergeVisible`, `layer.stampVisible` |
| `Mnm ` | minimum | Minimum | `filter.other.minimum` |
| `MtnB` | motionBlur | Motion Blur | `filter.blur.motionBlur` |
| `move` | move | Move | `layer.translate`, `layer.arrange.bringToFront`, `layer.arrange.sendToBack`, `layer.arrange.bringForward`, `layer.arrange.sendBackward` |
| `Ofst` | offset | Offset | `filter.other.offset` |
| `Opn ` | open | Open | `file.openAs` |
| `past` | paste | Paste | `edit.paste`, `edit.pasteSpecial.pasteInPlace` |
| (string ID only) | pasteEffects | Paste Layer Style | `layer.layerStyle.pasteLayerStyle` |
| (string ID only) | photoFilter | Photo Filter | `image.adjustments.photoFilter` |
| `Plc ` | placeEvent | Place | `file.placeEmbedded`, `file.placeLinked` |
| `Ply ` | play | Play | `actions.play` |
| `Pstr` | posterize | Posterize | `image.adjustments.posterize` |
| `Prge` | purge | Purge | `edit.purge.all`, `edit.purge.clipboard`, `edit.purge.histories`, `edit.purge.undo` |
| `RdlB` | radialBlur | Radial Blur | `filter.blur.radialBlur` |
| (string ID only) | rasterizeLayer | Rasterize | `layer.rasterize.layer` |
| `Ungr` | ungroup | Release Clipping Mask | `layer.releaseClippingMask` |
| `Rset` | reset | Reset | `tools.defaultColors` |
| `RvlA` | revealAll | Reveal All | `image.revealAll` |
| `Rvrt` | revert | Revert | `file.revert` |
| `Rple` | ripple | Ripple | `filter.distort.ripple` |
| `Rtte` | rotateEventEnum | Rotate | `image.imageRotation.90cw`, `image.imageRotation.90ccw`, `image.imageRotation.180`, `image.rotation.arbitrary`, `edit.transform.rotate90Cw`, `edit.transform.rotate90Ccw`, `edit.transform.rotate180` |
| `save` | save | Save | `file.save`, `file.saveAs`, `file.saveACopy` |
| `slct` | select | Select | `layer.select`, `layer.selectAbove`, `layer.selectBelow`, `layer.addAboveToSelection`, `layer.addBelowToSelection`, `layer.selectTop`, `layer.selectBottom`, `channel.target`, `edit.undo`, `edit.redo`, `view.fitOnScreen`, `view.actualPixels`, `view.zoomIn`, `view.zoomOut`, `document.activate` |
| (string ID only) | selectAllLayers | Select All Layers | `select.allLayers` |
| (string ID only) | refineSelectionEdge | Select and Mask | `select.refineEdge` |
| (string ID only) | smartBrushWorkspace | Select and Mask | `select.refineEdge` |
| (string ID only) | autoCutout | Select Subject | `select.subject` |
| `SlcC` | selectiveColor | Selective Color | `image.adjustments.selectiveColor` |
| `setd` | set | Set | `select.all`, `select.deselect`, `select.reselect`, `select.loadSelection`, `select.rect`, `select.lasso`, `select.editInQuickMaskMode`, `tools.setColors`, `layer.setProps`, `layer.renameLayer`, `layer.setLabelColor`, `layer.setAdjustment`, `layer.fill.pattern.set`, `layer.new.layerFromBackground`, `layer.layerStyle.blendingOptions`, `layer.smartFilter.blendingOptions` |
| (string ID only) | adaptCorrect | Shadows/Highlights | `image.adjustments.shadowsHighlights` |
| `Shrp` | sharpen | Sharpen | `filter.sharpen.sharpen` |
| `ShrE` | sharpenEdges | Sharpen Edges | `filter.sharpen.sharpenEdges` |
| `ShrM` | sharpenMore | Sharpen More | `filter.sharpen.sharpenMore` |
| `Shw ` | show | Show | `layer.showLayers` |
| `Smlr` | similar | Similar | `select.similar` |
| `SmrB` | smartBlur | Smart Blur | `filter.blur.smartBlur` |
| `Smth` | smoothness | Smooth | `select.modify.smooth` |
| `Stop` | stop | Stop | `STOP_MESSAGE` |
| `Strk` | stroke | Stroke | `edit.stroke` |
| (string ID only) | surfaceBlur | Surface Blur | `filter.blur.surfaceBlur` |
| `Thrs` | threshold | Threshold | `image.adjustments.threshold` |
| `Trnf` | transform | Transform | `edit.transform`, `select.transformSelection` |
| (string ID only) | trim | Trim | `image.trim` |
| (string ID only) | ungroupLayersEvent | Ungroup Layers | `layer.ungroupLayers` |
| `UnsM` | unsharpMask | Unsharp Mask | `filter.sharpen.unsharpMask` |
| (string ID only) | updatePlacedLayer | Update Smart Objects | `layer.smartObjects.updateModifiedContent` |
| `Vbrn` | vibrance | Vibrance | `image.adjustments.vibrance` |

## Known gaps in mapped events

A mapped event can still lose a setting. Every mapped event was checked on 2026-11-01: the 5 296 steps of
the `.atn` corpus (sets the reference application recorded) and factory descriptors measured in the
reference application went through `actions.load` and `actions.save`, and each saved descriptor was compared
with the original. Then every step PhotoCraft saves for the cases in
`every_mapped_command_exports_and_imports_back` was played in the reference application: 128 of 151 ran.
Three failed on a wrong code (Vibrance, Gradient Map, Stroke Outside); the others needed a document the test
did not have (a mask, a group, a named pattern or channel). Checking every code the mapping writes found
three more (Pass Through, canvas extension colour, Absolute Colorimetric). The steps below change what a
step does; each line says what is lost and the fix.

### Import or export loses a setting (fix in `atn_cmds` only)

| Event | What is lost | Fix |
|---|---|---|
| Transform (`Trnf`) | The reference point `FTcs` (`QCSt`: `Qcsa` centre, `Qcs0`..`Qcs8` corners and edges) is ignored on load, and save always writes `Qcsa`. A 50 % scale about the top-left corner of a 100 px square at 60,60 ends at 60,60..110,110 in the reference application, at 85,85..135,135 when played about the centre. | Read `FTcs` into the commands' `reference`; write it back. Pure offsets are unaffected. |
| Show (`Shw `) with `TglO` true | Alt-click on the eye ("show only this layer", a toggle: a second step restores the others) loads as a plain Show. | `TglO` true maps to `layer.showOnly`, and back. |
| Make `layerSection` (Group from Layers) | Newer recordings store the group name as a top-level `Nm  ` (no `Usng`); only `Usng.Nm  ` is read, so the group gets the default name. | Read the top-level `Nm  ` too. |
| Make `Lyr ` (New Layer) | `Usng.Grup` true (Use Previous Layer to Create Clipping Mask) is dropped. | Map to `clipToPrevious`; export it (today such a step is refused). |
| Make `AdjL` (adjustment layer) | `Usng.Clr ` (the colour label) is dropped. | Map the label; export it. |
| Make `contentLayer` with `Shp ` (Rectangle tool shape layer, with `strokeStyle`) | Loads as a solid colour fill layer covering the canvas: the shape and its stroke settings are lost. | Map to the shape command, or keep the step as a kept step until it is mapped. |
| Make with `Nw ` + `Usng ArSl.Slct` (Layer via Copy / Cut with layer options) | Only the name is kept; `Md  ` (blend mode), `Opct` and `Clr ` are dropped. | Carry them into the `layer.setProps` step that follows. |
| Set of the selection to `Chnl.Trsp` of a named layer (Cmd-click on another layer's thumbnail) | The layer in the reference is dropped, so the current layer's transparency is loaded. | Keep the layer in `select.loadSelection`; export it. |
| Background layer by property (`Lyr .Bckg`) | Becomes the layer name "Background" on load and is saved as a name reference. A localised reference application has no layer of that name ("Hintergrund" in German), and the step fails there. | Keep the property form: load as the Background layer, save as `Lyr .Bckg`. |
| Close (`Cls `) without `Svng` | Without `Svng` the reference application asks whether to save (without a dialog it refuses to close a changed document). Save writes `Svng N` (do not save), so playback there silently discards changes. 244 corpus steps. | Write no `Svng` for a Close that asks. |
| Photo Filter (`photoFilter`) with a `LbCl` colour | The reference application's default filter colour is Lab (`Lmnc`, `A   `, `B   `); only RGB, HSB and gray colours are read, so a recorded Photo Filter usually stays a kept step. | Convert Lab colours (through `photocraft-cms`) on load. |
| Vibrance (direct) | Saved as event `Vbrn`, which the reference application does not know: the step fails there ("command not available"). | Save as `vibrance`. |
| Gradient Map (direct) | Saved as event `GdMp`, which is the Gradient Map *class*, not an event: the step fails there. | Save as `gradientMapEvent`. |
| Stroke, Location Outside | Written as `StrL.OtsF`; the application's value is `Otsd` and it rejects `OtsF` ("parameters not valid"). A recorded `Otsd` does not load. | Use `Otsd` both ways. |
| Set layer blend mode Pass Through (groups) | Written as `BlnM.PsTh`; the application rejects it. Its value is `passThrough`. | Save `passThrough`. |
| Canvas Size, extension colour Foreground / Other | Written as `canvasExtensionColorType` `Frgd` / `Othr`; the application does not know them and opens the Canvas Size dialog instead. Its values are `FrgC` and `Clr `. | Use `FrgC` and `Clr ` both ways. |
| Convert to Profile, intent Absolute Colorimetric | Written as `Inte.AbsC`; the application reads it as Relative Colorimetric (it echoes `Clrm`). Its value is `AClr`; a recorded `AClr` does not load. | Use `AClr` both ways. |
| Convert Mode (`CnvM`) | Saved with `T   ` as an enum (`ClrS.Grys`); the application records and needs a class (`T   ` type `Grys`): with the enum every mode change fails there ("program error"). A recording on a layered document also carries `Fltt` and `Rstr` (false), which avoid the flatten question on playback. | Save `T   ` as a class; read and write `Fltt`/`Rstr`. |
| Make adjustment layer with `Usng.Opct` | The layer's opacity (e.g. 70 % in the bundled Gradient Map action) is dropped. | Map to the new layer's opacity. |
| Canvas Size in percent | A recorded size in `#Prc` (the bundled Frame Channel action records 200 %) stays a kept step. | Scale the current size by the percentage. |
| Image Size in cm or inches | Recorded in `#Rlt` (points) with `scaleStyles`; stays a kept step. | Convert with the document's resolution at playback. |
| Select a document by offset (`Dcmn` `rele`) | The reference application wraps around at the ends of the opening order (from the second of three documents, -2 is the third, +2 the first); `document.activate` refuses an offset past the ends (#7103), so the step fails in PhotoCraft. See Reference forms. | Wrap like the reference application. |

### The command has no such setting (needs a new command option first)

| Event | Setting | Note |
|---|---|---|
| Image Size (`ImgS`) | `scaleStyles` | Measured: with `scaleStyles` true a 20 px stroke becomes 10 px at 50 %; with false or no key it stays 20 px. `image.imageSize` always scales effects, so a step without the key plays differently in PhotoCraft, and a saved step plays unscaled there. 27 corpus steps carry it. |
| Convert to Profile (`convertToProfile`) | `Dthr` (dither), `Fltt` (flatten), `rasterizePlaced`, `sdwM` | Dropped on load. |
| Fill (`Fl  `) with Content-Aware | `contentAwareColorAdaptationFill`, `contentAwareRotateFill`, `contentAwareScaleFill`, `contentAwareMirrorFill` | Dropped on load. |
| Paste Layer Style (`pasteEffects`) | `allowPasteFXOnLayerSet` | Dropped on load. |
| Paste (`past`) | `As ` (paste a vector clipboard as pixels, path or shape) | Dropped on load. |

### Differences that do not change the result

- Value types: `long` and unit values are interchangeable for the reference application (Motion Blur
  `Angl`, Dust & Scratches `Rds `, Apply Image `Ofst`: same pixels either way); Apply Image's `With`
  object plays the same with class `null`.
- Save writes the 4-character event code where the file had the string ID, writes `null` as the
  top-level class (the application writes the event code), and writes an empty descriptor for steps that
  had none.
- Ids of the recording session (`LyrI`, `DocI`, `Idnt`, `layerSectionStart`, `layerSectionEnd`, `Vrsn`,
  `forceNotify`) and default flags (`MkVs` false, `Adjs` false) are not written back.
- An adjustment layer's values equal to the defaults are not written back (other values are).
- Colours are kept as 8-bit values (51.0031 becomes 51); gradient names become "Custom".
- Levels and Curves write the composite as `Chnl.Cmps` (the application skips `Ordn.Trgt`, #8543).

## Reference forms

How the reference application resolves a layer or document a step names, measured on 2026-11-01 by
playing `select` and `move` steps with each form (documents with and without a Background, a group,
layers above and below it). A recorded step can use any of these; PhotoCraft reads names and the target
today.

### Layers

The stack is counted from the bottom and includes a hidden "group end" entry below each group's
children. Example stack, bottom to top: Background, L1, L2, *group end*, Gchild, G (the group), L3.

| Form | Meaning | Measured |
|---|---|---|
| By name (`name`) | The layer with that name (the first match). | (supported) |
| Target (`Ordn.Trgt`) | The active layer. | (supported) |
| By index (`indx`) | Position in the stack above. With a Background, the Background is 0 and L1 is 1. Without one, the bottom layer is 1, and 0 fails. The group end is a position but cannot be selected: the step leaves the selection unchanged. A number past the top fails. | with Background: 0 Background, 1 L1, 2 L2, 3 (group end: no change), 4 Gchild, 5 G, 6 L3, 7 fails |
| By id (`Idnt`) | The layer id of the document the step was recorded in (`LyrI`, never reused in that document). In another document it names an unrelated layer or none. | id 3 = L2 with a Background, L1 without |
| By offset (`rele`) | Not supported for layers: every offset fails. | -3..3 all fail |
| `Ordn.Frwr` / `Ordn.Bckw` | The next layer up / down in the stack, entering groups (group ends are skipped). | from L2: Frwr Gchild, Bckw L1 |
| `Ordn.Frnt` / `Ordn.Back` | The top layer / the bottom layer (the Background when there is one). | L3 / Background |

**Move to an index** (`move`, `T` = layer by index `m`): counted as above on the stack before the move.
Moving down, the layer lands at index `m`; moving up, at `m - 1` (directly below the layer that had index
`m`). With a Background, `m` = 0 fails; without one, `m` = 0 sends the layer to the bottom. Moving to an
index inside a group puts the layer into that group.

| m (with Background, moving L2 at index 2) | 0 | 1 | 2, 3 | 4 | 5 | 6 |
|---|---|---|---|---|---|---|
| Stack after | fails | Bg, L2, L1, ... | unchanged | Bg, L1, *end*, L2, Gchild, G, L3 (L2 inside G, bottom) | ... *end*, Gchild, L2, G (inside G, top) | ... G, L2, L3 |

### Documents

| Form | Meaning | Measured (D1, D2, D3 opened in that order, D2 active) |
|---|---|---|
| By offset (`rele`) | That many documents after (positive) or before (negative) the active one, in opening order, **wrapping around** at the ends. 0 fails. | -1 D1, +1 D3, -2 D3, +2 D1 |
| By index (`indx`) | 1-based, in opening order; 0 fails. | 1 D1, 2 D2, 3 D3 |

PhotoCraft's `document.activate {"offset"}` refuses to wrap (#7103); the reference application wraps.

### Layer comps

Measured with three comps (CompA, CompB, CompC) that each show a different layer, applied with
`applyComp` and every reference form. The document's `compsList` gives each comp's `title`, `itemIndex`
(1-based), `ID` (long) and `selection`.

| Form | Result |
|---|---|
| By name (`name`) | Applies that comp. An unknown name fails. |
| By index (`indx`) | 1-based, in panel order. 0, negative and past-the-end indexes are **accepted and change nothing**. |
| By ID (`Idnt`) | Applies the comp with that `ID`. An unknown ID fails. |
| By offset (`rele`) | Always fails, for every offset. |
| Ordinal (`Ordn`) `Trgt`, `Nxt `, `Prvs` | Applies the current (last applied) comp; `Nxt ` and `Prvs` do not move. |
| Ordinal `Frst`, `Lst `, `Al  ` | Applies the **first** comp (also for `Lst `). |

Delete (`Dlt `) by index uses the same 1-based numbering. PhotoCraft's `applyComp` import reads only the
name form today; the index, ID and ordinal forms load as kept steps.

## Newer events (string ID only)

The events the reference application records for the menu items PhotoCraft has that are neither mapped
nor in the reference below: 63 candidates from PhotoCraft's menu catalogue, each run in the reference
application without a dialog on a document prepared for it (layers, a selection, a smart object, a type
layer, a layer style or a group) on 2026-11-01. 27 more menu items record no event of their own (puppet
warp and warp splits, combine shapes, scripts such as Load Files into Stack and Statistics, artboard and
video export, missing-font commands): they cannot be recorded or play as a script step.

**Measured: no settings beyond the target.** A step with these events carries at most `null` (the target
layer reference, `Ordn.Trgt`); mapping them needs only the event name.

| Event | Menu item |
|---|---|
| `reverse` | Layer > Arrange > Reverse (several layers selected) |
| `linkSelectedLayers` / `unlinkSelectedLayers` | Layer > Link Layers / Unlink Layers |
| `selectLinkedLayers` | Layer > Select Linked Layers |
| `disableLayerStyle` | Layer > Layer Style > Clear Layer Style |
| `placedLayerMakeCopy` | Layer > Smart Objects > New Smart Object via Copy |
| `placedLayerConvertToLayers` | Layer > Smart Objects > Convert to Layers |
| `placedLayerConvertToEmbedded` | Layer > Smart Objects > Embed Linked |
| `placedLayerUpdateAllModified` | Layer > Smart Objects > Update All Modified Content (the returned `documentID` and `layerIDs` are session ids, not settings) |
| `rasterizeAll` | Layer > Rasterize > All Layers |
| `addBlankVideoLayer` | Layer > Video Layers > New Blank Video Layer |
| `isolateLayers` | Select > Isolate Layers |
| `textLoadDefaultTypeStyles` / `textSaveDefaultTypeStyles` | Type > Load / Save Default Type Styles |
| `recordMeasurements` | Image > Analysis > Record Measurements |
| `CropPhotosAuto0001` | File > Automate > Crop and Straighten Photos |

**Measured: settings.**

| Event | Menu item | Keys |
|---|---|---|
| `placedLayerExportContents` | Layer > Smart Objects > Export Contents | `null`: path of the file written |
| `placedLayerConvertToLinked` | Layer > Smart Objects > Convert to Linked | `null`: target layer; `Usng`: path of the file written |
| `placedLayerRelinkToFile` | Layer > Smart Objects > Relink to File | `null`: path; `LyrI`: session layer id |
| `placedLayerReplaceContents` | Layer > Smart Objects > Replace Contents | `null`: path; `LyrI`: session layer id |
| `artboardFromLayerGroupEvent` | Layer > New > Artboard from Group | `null` target; `artboard` (object: `guideIDs` list, `artboardPresetName` text, `Clr ` RGB background colour, `artboardBackgroundType` long, 1 = white); `changeBackground` long |
| `shapeBlur` | Filter > Blur > Shape Blur | `Rds ` radius in pixels (factory 10); the shape itself comes from the dialog |
| Make `contentLayer` with `Usng` = reference to `textLayer` `Ordn.Trgt` | Type > Convert to Shape (recorded as Make, not as `convertToShape`) | `null`: class `contentLayer`; `Usng`: the type layer |

**Recorded through the menus** (2026-11-01, steps read back from the saved Actions panel):

| Menu | Recorded step | Keys |
|---|---|---|
| Layer > Layer Style > Scale Effects | `scaleEffectsEvent` | `Scl ` percent, whole number 1..1000 (the dialog clamps) |
| Layer > Layer Style > Create Layer(s) | Make, `null` = class `Lyr `, `Usng` = reference to the `Lefx` property of the target layer | none; the dialog first warns that some effects cannot be reproduced with layers (OK / Cancel) |
| Layer > New > Frame from Layers | Make, `null` = class `framedGroupSection`, `From` = layer **by name**, `layerSectionStart`, `layerSectionEnd` long, `Nm  ` text | the dialog also shows width and height, which are not recorded |
| Layer > New > Artboard from Layers | Make, `null` = class `artboardSection`, `From` = layer target, `layerSectionStart`, `layerSectionEnd`, `Nm  ` | |
| Layer > New > Artboard | Make, `null` = class `artboardSection`, `layerSectionStart`, `layerSectionEnd`, `Nm  `, `artboardRect` (`Top `, `Left`, `Btom`, `Rght` doubles, pixels) | |
| Layer > Smart Objects > Stack Mode > mode | `applyImageStackPluginRenderer`: `imageStackPlugin` **class** with the mode's code, `Nm  ` text = the menu name (localised, except "Entropy") | codes: Range `rang`, Entropy `entr`, Kurtosis `kurt`, Maximum `maxx`, Median `medn`, Minimum `minn`, Mean `avrg`, Skewness `skew`, Standard Deviation `stdv`, Summation `summ`, Variance `vari` |
| Layer > Smart Objects > Stack Mode > None | `imageStackConvertSmartObject` | none |
| Layer > Video Layers > New Blank Video Layer, Insert Blank Frame, Delete Frame, Show / Hide Altered Video | `addBlankVideoLayer`, `insertBlankFrame`, `deleteFrame`, `showAlteredVideo` / `hideAlteredVideo` | none |
| Layer > Video Layers > Interpret Footage | `interpretFootage` | `Vrsn` long (1), `interpretAlpha` (`alphaInterpretation`: `Ignr` ignore, …), `colorConvertAlterations` bool |
| Type > Create Work Path | Make, `null` = class `Path`, `From` = text layer (`TxLr`) target | |
| Edit > Find and Replace Text (Replace All) | `replace`, `null` = reference: property `replace` of all text layers (`TxLr` `Ordn.Al`), `Usng` = `find` text, `replace` text, `checkAll`, `Fwd `, `caseSensitive`, `wholeWord`, `ignoreAccents` bool | |
| Edit > Check Spelling | `spellCheck`, `null` = property `spellCheck` of all text layers, `Usng` = `checkAll` bool | recorded with its dialog switched on; the corrections themselves are not recorded |
| Image > Analysis > Set Measurement Scale > Custom | Set, `null` = property `measurementScale` of the document target, `T   ` = `pixelLength` long, `logicalLength` double, `logicalUnits` text | |
| Image > Analysis > Place Scale Marker | a script step (`AdobeScriptAutomation Scripts`) with `jsop` = `measurementScaleMarkerLength`, `measurementScaleMarkerDisplayText`, `measurementScaleMarkerTextPositionBottom` bool, `measurementScaleMarkerColor` text (`FFFFFF` / `000000`), `measurementScaleMarkerFont`, `measurementScaleMarkerFontSize` text ("4 pt") | the dialog is a bundled script; the step stores its options |
| Image > Analysis > Record Measurements | `recordMeasurements` | none |
| File > Close All | one `close` per document: `Svng` (`YsN ` `N   ` when "don't save" was chosen), `DocI`, `forceNotify` | not a `closeAll` event |

**Not recorded:** File > Close Others, Image > Variables > Define (the variables are kept, no step).

**Recorded with source files** (2026-11-01):

| Menu | Recorded step | Keys |
|---|---|---|
| Image > Apply Data Set | `apply`, `null` = `dataSetClass` **by name** | |
| File > Export > Data Sets as Files | `exportDataSet`, `null` = class `dataSetClass` | `All ` bool (all data sets), `T   ` alias (folder), `fileNamingRules` (`fileNamingComponents` list of 7 incl. `None`; components as in Batch plus `underscore` and `dataSetName`; `Mcnt`, `Win `, `unix`) |
| File > Import > Variable Data Sets | `importDataSets`, `null` = class `dataSetClass` | `Usng` alias (the text file), `Encd` (`dataSetEncoding`: `dataSetEncodingAuto`, …), `eraseAll` bool (replace existing), `useFirstColumn` bool (first column = data set names) |
| File > Package | `packageFile` | `T   ` alias (folder); writes "<document> Folder" (localised) with the document and a `Links` folder |
| File > Import > Video Frames to Layers | `importVideoToLayers` | `Usng` alias (video), `makeAnimation` bool, `frameSkip` long (limit to every n-th frame; absent when off) |
| File > Import > Notes (Annotations) | `importAnnots` | `Usng` alias (PDF). A file without notes shows "No annotations were imported" and records nothing |
| Layer > Video Layers > New Video Layer from File | `addVideoLayer` | `null` alias (video) |
| Layer > Video Layers > Replace Footage | `replaceFootage` | `null` alias (video) |
| Layer > Video Layers > Duplicate Frame, Restore Frame, Restore All Frames, Reload Frame | `duplicateFrame`, `restoreFrame`, `restoreAllFrames`, `reloadFrame` | none. Duplicate needs a painted frame; Restore needs an altered frame; Restore All asks for confirmation |
| File > Automate > Photomerge | the steps the script runs (open dialog, open, duplicate, `addLayerFromFile`, `align` with `layerTransformation`, `mergeAlignedLayers` …), then the script event `0f9db13f-a772-4035-9020-840f0e5e2f02` with an empty descriptor (class `cjsp`) | the layout and options are not in the step |
| File > Automate > Crop and Straighten Photos | `CropPhotosAuto0001` (no keys), then the steps it runs: `CropPhotos0001` with `Vl  ` (list of 8 doubles: the four corners), duplicate, `crop` | needs a selected, visible, unlocked layer, otherwise an error message and a step without effect |
| File > Automate > Conditional Mode Change | the Convert Mode it runs, then `8cba8cd6-cb66-11d1-bc43-0060b0a13dc4` | `Msge` text, `SrcM` list of `Cndn` enums (`UGry`, `URGB`, …: the source modes), `DstM` object whose class is the target mode (`CMYM` for CMYK) |
| File > Automate > Fit Image | the Image Size it runs, then `3caa3434-cb67-11d1-bc43-0060b0a13dc4` | `Msge` text, `Hght`, `Wdth` pixels, `limit` bool (don't enlarge) |

**Not recorded:** importing or applying inside Image > Variables > Data Sets (only the separate menu
commands record).

## Measured through dialogs and recordings

Measured on 2026-11-01 in two ways: by running the event with its dialog and pressing OK (the reference
application then returns the full descriptor), and by recording actions through the menus and reading
the recorded steps back from the saved Actions panel. Ranges come from the dialog's own "a value between
… is required" message.

| Event | Keys (recorded order) | Notes |
|---|---|---|
| Custom (`Cstm`) | `Scl ` long 1..9999, `Ofst` long -9999..9999, `Mtrx` list of 25 longs -999..999 | Without the dialog the step needs all 25 cells, or the application fails. |
| Fibers (`Fbrs`) | `Vrnc` long 1..64, `Strg` long 1..64, `RndS` long (seed) | |
| Displace (`Dspl`) | `HrzS`, `VrtS` long -999..999; `DspM` (`StrF` stretch to fit, or tile); `UndA` (`RptE` repeat edge pixels, or wrap around); `DspF` alias: path of the displacement map (.psd) | The map file is chosen after OK. |
| Color Lookup (`colorLookup`) | `lookupType` (`colorLookupType`: `3DLUT`, abstract, device link); `Nm  ` text: the file's path; `Dthr` bool; `profile` raw data (ICC); `LUTFormat` (`LUTFormatType`, e.g. `LUTFormatLOOK`); `LUT3DFileData` raw data: the LUT file itself; `LUT3DFileName` text | The LUT travels inside the step. |
| Shape Blur (`shapeBlur`) | `Rds ` pixels 5..1000; `customShape` object (`Nm  ` text, `Idnt` text: the shape's id); `keyOriginType` long (9) | |
| Lighting Effects (`lightFilterLightingEffects`) | `k3DLights` list of lights (`Nm  `, colour `Rd  `/`Grn `/`Bl  ` 0..1, `hots`, `FlOf`, `shdw`, attenuation keys, `Type`, `ison`, `key3DMatrix` raw + floats, `X   `/`Y   `/`Z   `, target `tarx`/`tary`/`tarz`, `key3DPosition`), `key3DCurrentCameraPosition`, `Glos`, `Mtrl`, `Exps`, `AmbB`, `AmbC` RGB, `BmpA`, `BmpC` (texture channel by name, localised: "Ohne" = none), `Wdth`, `Hght` | A dialog-less run returns nothing; only a recording shows the keys. |
| Blur Gallery (`blurbTransform`) | `blurbType`, `VrsM`, `VrsN`, `referenceRect` (rectangle), `blurbOpenPanel`, `blurbGeneralBlurEffectApplied`, `blurbSaveMaskChannel`, `blurbHighQuality`, `blurbWidgetList` (per widget: `blurbWidgetType` (0 = field blur), `blurbWidgetLocationX`/`Y`, `blurbWidgetSelected`, `blurbWidgetEffectEnabled`, `blurbGeneralBlurAmount`) | One event for all five Blur Gallery filters. |
| Liquify (`LqFy`) | `LqMe` raw data: the mesh | Opaque; keep the step as recorded. |
| Camera Raw Filter | `CrVe` text (version), `PrVN` long, `PrVe` long | With settings at their defaults; changed settings add keys. |
| Vanishing Point (`VnPt`) | none | The planes are not recorded. |
| Calculations | recorded as Make: `Nw  ` class channel, `Usng` object (`T   ` reference: source 1 channel and layer, `Clcl` blend mode, `Src2` reference) | Not a `calculations` event. |
| Image Size, size in cm or inches | `Wdth` in `#Rlt` (points), `scaleStyles`, `CnsP`, `Intr` | A physical size is recorded in points, not pixels: playback needs the document's resolution. |
| Canvas Size | only the changed side; `canvasExtensionColorType` `FrgC` for the foreground colour | |
| Convert Mode, several layers, "Don't Flatten" | `T   ` as a **class** (`Grys`), `Fltt` false, `Rstr` false | With `Fltt` and `Rstr` the step plays without the flatten question. |
| New Group with a colour | `Usng` (`Clr ` label), `layerSectionStart`, `layerSectionEnd`, top-level `Nm  ` | |
| Shear (`Shr `) | `ShrP` list of `Pnt ` (`Hrzn`, `Vrtc` doubles), `UndA` (`WrpA` wrap around, `RptE` repeat edge pixels), `ShrS` long, `ShrE` long | The dialog writes whole numbers: `Hrzn` -64..63 (dragging past the grid clamps), `Vrtc` 1..128 with the first point at 1 and the last at 128; dragging a point out of the grid deletes it. `ShrS` is 0 and `ShrE` the last point's index (count - 1). Script runs take any value unclamped and crashed on far-off `ShrE`/`ShrS` or a first `Vrtc` below 1. |
| Gradient tool, classic (`Grdn` / `gradientClassEvent`) | `From`, `T   ` points in pixels; `Opct` percent, `Md  ` blend mode, `Type` (`GrdT`: `Lnr `, `Rdl `, `Angl`, `Rflc`, `Dmnd`), `Rvrs` bool, `Dthr` bool, `UsMs` bool (transparency), `gradientsInterpolationMethod` (`Perc` perceptual, …), `Grad` gradient object (as in Gradient Map) | Recorded in that order. `Opct`, `Md  ` and `Rvrs` appear only when they differ from 100 %, Normal and off; `Dthr` and `UsMs` only when on. The five types are the only accepted values. A step without `Grad` fails. |
| Gradient tool, default mode | two steps: Make (`null` = class `contentLayer`, `Usng` = `Type` gradient layer object) then Set (`null` = `contentLayer` target, `T   ` = the same object plus `Algn` false) | The object: `Dthr`, `gradientsInterpolationMethod`, `Angl` angle, `Type`, `Scl ` percent, `Ofst` (`Hrzn`, `Vrtc` percent), `Grad`. The new tool makes a gradient fill layer; Make holds the first click, Set the finished drag. |
| Content-Aware Fill (`cafWorkspace`) | `cafSamplingRegion` (`cafSamplingRegionRectangular`, …), `cafSampleAllLayers` bool, `cafColorAdaptationLevel` (`cafColorAdaptationDefault`, …), `cafRotationAmount` (`cafRotationAmountNone`, …), `cafScale` bool, `cafMirror` bool, `cafOutput` (`cafOutputToNewLayer`, …) | Runs on the selection, which is recorded as its own step before it. |
| Perspective Warp (`perspectiveWarpTransform`) | `null` = layer target, `Md  ` (`perspectiveWarpMode`: `warp`), `referenceRect` rectangle in pixels, `vertices` list of points in pixels (the layout quad), `warpedVertices` (the same points after warping), `quads` list of `indices` (4 vertex indexes each) | Layout and warp are one step. |
| Print (`print`) then Print One Copy (`printOneCopy`) | `print`: `null` = reference `printOutput` property of the target document; `printOutput` (`PstS`, `Inte`, `printSixteenBit`, `printerName` text, `printProofSetup`), `printViewProofSetup`, `printOutputOptions` (caption, marks, `Bckg` colour, `BrdT`, `Bld `, `Rslt`, `vectorData`, `PgPs`, `Left`, `Top `, `Scl `, crop rectangle), `osSpecificPrintInfo` raw data, `Cptn` text. `printOneCopy` has no keys. | Pressing Print in the dialog records both steps. |
| Create Droplet (`CrtD`) | `In  ` alias: the droplet file; `Fldr` bool (include subfolders); `Usng` reference to the action; then the Batch keys below (destination, file naming, error log) when set | The same settings as Batch, with the droplet file instead of a source folder. |
| Sky Replacement (`skyReplacement`) | `shiftEdge` double, `borderSmoothness` long (fade edge, default 50), `Brgh` long, `temperature` long, `harmonizationOpacity` long (35), `foregroundLightingOpacity` long, `lightingMode` blend mode (`Mltp`), `edgeLightingOpacity` long (70), `skyReplacementOutput` (`skyReplacementOutputToNewSheets` new layers, …), `Idnt` text (the sky preset's id), `Nm  ` text (its name), `File` alias (the sky image) | Scale and Flip are **not** in the step: with 150 % and Flip on, the returned keys were the same. `foregroundLightingOpacity` is computed from the sky (44 and 29 for two skies). Without a chosen sky the dialog returns no keys. |

**Not recordable:** Edit > Purge (no step is recorded; this version's Purge menu has Clipboard,
Histories, All and Video Cache, no Undo); File > Export > Export As and Quick Export as PNG (the file
is written, no step is recorded). `vectorStrokeToFill` (Convert Stroke to Fill) exists only internally: the reference application knows
it by a localised command name ("command … is not currently available"), but it is in no menu, panel
menu, context menu, the stroke options (incl. their gear menu), the Properties panel or the Keyboard
Shortcuts summary of all commands and tools, and the event is refused even with a stroked path component
selected. It can't be triggered, so it is never recorded: no action contains it, nothing to map.

### Batch

Running Batch while recording records the steps the batch played, not a Batch step. The Batch
descriptor itself (`Btch`, as the dialog returns it):

| Key | Type | Meaning |
|---|---|---|
| `null` | alias, or enum | source folder; for open documents the enum `batchSourceType` `sourceOpenFiles` |
| `OvrO` | bool | override action "Open" commands (written only when on) |
| `Fldr` | bool | include all subfolders |
| `suppressWarnings` | bool | suppress colour profile warnings |
| `suppressOpenOptions` | bool | suppress file open options dialogs |
| `Usng` | reference | the action: `Actn` by name in `ASet` by name |
| `T   ` | alias | destination folder (destination Folder) |
| `SvAn` | bool | destination Save and Close (written instead of `T   `) |
| `Ovrd` | bool | override action "Save As" commands (written only when on) |
| `fileNamingRules` | object | file naming (below) |
| `Log ` | alias | folder of the error log ("log errors to file") |
| `Nm  ` | text | error log file name |

`fileNamingRules`: `fileNamingComponents` (list of up to six objects; each is `component` enum
`fileNamingComponent`, or `Txt ` text for typed text), `Strt` long (starting serial number), `Mcnt`, `Win `,
`unix` bool (compatibility).

| Component | Menu entry |
|---|---|
| `Nm  ` / `lowerCase` / `upperCase` | Document Name / document name / DOCUMENT NAME |
| `oneDigit` `twoDigit` `threeDigit` `fourDigit` | 1, 2, 3, 4 digit serial number |
| `lowerCaseSerial` / `upperCaseSerial` | serial letter (a, b, c…) / (A, B, C…) |
| `mmddyy` `mmdd` `yyyymmdd` `yymmdd` `yyddmm` `ddmmyy` `ddmm` | dates |
| `lowerCaseExtension` / `upperCaseExtension` | extension / EXTENSION |

The extension component is written **without a dot, unless it is the last component**, and nothing
else adds an extension: the name is exactly what the components spell. Measured on `a.png`:

| Components | Saved as |
|---|---|
| Name + extension | `a.png` |
| Name + EXTENSION | `a.PNG` |
| Name + extension + 3 digit serial | `apng001` (no extension) |
| Name + 3 digit serial | `a001` (no extension) |
| Name + extension + text `x` | `apngx` |
| Name + extension + extension | `apng.png` |
| Name + text `.jpg` | `a.jpg` (still PNG data) |
| DOCUMENT NAME + `_` + serial letter + `_` + SERIAL LETTER | `A_a_A` |

The two short date entries are `mmdd` and `ddmm` (on 1 November they give `1101` and `0111`).

#### Batch behaviour

Measured with a folder `in` holding `a.png`, `b.png`, `notes.txt` and `sub/a.png`, `sub/c.png`,
`sub/deeper/d.png`:

- **Order:** the folder's files by name, then each subfolder (by name, depth first). The serial number
  counts across all of them: `a_001` (`in/a`), `b_002`, `a_003` (`in/sub/a`), `c_004`, `d_005`.
- **Include All Subfolders:** results all go **flat into the destination folder**; the subfolder structure
  is not rebuilt. A later file with the same output name **silently overwrites** an earlier one
  (`in/a.png` and `in/sub/a.png` give one `a.png`).
- **Destination None:** every processed file stays open, changed and unsaved. **Save and Close** saves
  each file over itself in its own format and closes it. **Folder** saves a copy named by the file
  naming rules. An open document that was never saved asks with a Save As dialog (Photoshop format,
  the document's name) even with Folder as the destination.
- **Override Open:** files are opened only by the action's own Open steps; without one the action runs
  with no document and every file fails. **Override Save As:** files are saved only by the action's own
  Save As steps, into the destination folder; without one nothing is saved. The dialog says both when
  the box is ticked.
- **Files that are not images** (`notes.txt`) are an error for that file (code -25600, "extension not
  recognised"), not skipped silently.
- **Errors, Stop For Errors** (no `Log ` key): the batch stops at the failing file; files before it are
  saved, nothing after it is processed.
- **Errors, Log Errors To File:** the batch goes on, and at the end a message says an error occurred and
  the details are in the log. The log file is **UTF-16 LE with BOM and CRLF**, and is written even when
  nothing failed. In German the lines are (the application's own language):

  ```
  Stapelanfang
  <blank>
  Datei: "<full path, or the document's name for open documents>"
  Fehler: <message> (<error code>)
  <blank>
  …one pair per failed file…
  Stapelende
  ```
- **Cancel:** Esc during a batch stops the running command and asks "A command was cancelled or stopped
  during batch processing. Continue with the next file or stop?" (Continue / Stop). Stop ends the batch
  at once; the file being processed is not saved (12 of 60 written in the test). There is no progress
  bar: the documents open and close in the window as they are processed.
- **Return value:** when any file failed, the Batch step itself reports "cancelled" (the dialog returns no
  descriptor); a script run with Stop For Errors reports success even though it stopped.

### Actions panel menu

Button Mode; New Action, New Set, Duplicate, Delete, Play; Start Recording, Record Again, Insert Menu
Item, Insert Stop, Insert Conditional, Insert Path; Action Options, Playback Options, Allow Tool
Recording; Clear All Actions, Reset Actions, Load Actions, Replace Actions, Save Actions; the bundled sets
(Commands, Frames, Image Effects, LAB Black & White Technique, Production, Star Trails, Text Effects,
Textures, Video Actions); Close, Close Tab Group. Playback Options: Accelerated, Step by Step, Pause For
1..60 seconds.

How two menu entries record: **Insert Menu Item** records the chosen menu command's own event with no
descriptor (Image > Image Size gives an `imageSize` step without settings; playback opens its dialog).
**Insert Path** records a `set` of the work path: `null` = reference `Path` property `WrPt`, `T   ` = list
of path components (`shapeOperation`, `SbpL` list of subpaths, each with `Clsp` closed and `Pts ` list of
points with `Anch` in pixels).

## Recording that plays on any document

What the reference application records when a user works normally, measured on 2026-11-01 by recording
an action through the panel, mouse and keyboard on a document with layers A, B and C and a second open
document, then reading the steps back. This is the form PhotoCraft's recorder should write (AUTO-219-1);
today it copies journal entries with explicit layer ids.

| User action | Recorded step |
|---|---|
| Click a layer in the Layers panel | `select`, `null` = layer **by name** (`"B"`), `MkVs` false, `LyrI` list of layer ids (a hint, not the reference) |
| Ctrl+J (Layer via Copy) | `copyToLayer`, no keys |
| Click an empty spot of the canvas with Auto-Select on | `selectNoLayers`, `null` = layer target |
| Arrow keys with the Move tool | **one** `move` step per run of key presses, `null` = layer target, `T   ` offset in pixels: Right, Right, Right, Shift+Down gave `Hrzn` 3, `Vrtc` 10 |
| Eye icon | `hide` (or `show`), `null` = **list** of layers by name |
| Opacity typed in the Layers panel | `set` of the target layer, `T   ` = `Opct` percent (pressing Enter after typing recorded the same step twice) |
| Select All, Deselect | `set` of the selection (`fsel`) to `Al  ` / `None` |
| Alt+Backspace (fill with foreground) | `fill`, `Usng` `FrgC`, `Opct` 100 %, `Md  ` Normal |
| Type tool: click, type "Hallo", commit | **one** `make` of a text layer (`TxLr`) with `Txt ` = the whole text; position `TxtC` in **percent of the canvas**; bounds in points |
| Ctrl+E (Merge Down) | `mergeLayersNew`, no keys |
| Ctrl+Tab / Ctrl+Shift+Tab | `select` of a document **by offset** (`rele` +1 / -1) plus `DocI` (the document id, a hint) and `forceNotify` |

So the recorder names layers by name and acts on the target layer, never by id; it merges a run of
nudges into one relative move and a typing session into one text step; text and document positions are
relative.

### Playing on another document

The same recording played on two other documents (Play from step 2, skipping the setup step):

- **Same layer names, new ids:** plays through; layers are found by name, `LyrI` and `DocI` are ignored.
- **No layer with that name** (layers X, Y, Z): each step that names a missing layer stops with a
  message and Continue / Stop. Select by name: "The command 'Select' is not currently available". Hide by
  name: "The object 'Layer "A"' is not currently available". Steps on the target layer act on whatever
  layer is selected (Ctrl+J copied Z into "Z Kopie").
- **After `selectNoLayers`**, steps that need a target layer (move, set opacity, fill) each stop with
  "The command '…' is not currently available" and Continue / Stop. Continue skips the step; nothing is
  undone.
- **Play from a step:** selecting a step in the panel and pressing Play plays from that step to the end.

## Older files: version 12

Action sets saved by the reference application's version 5 era use file version 12. PhotoCraft rejects
them today; this is what reading them needs. Checked on 2026-11-01 on 325 real version 12 sets (all parse
to the end with this layout) and against AtnKit (MIT), which reads version 12 but gets two details wrong
(noted below).

**Only the descriptor differs.** The file header, the action record and the step record are the same as
in version 16 (set and action names are UTF-16 strings with a counted NUL; events are almost always
4-character codes, so the existing event table applies). Trailing bytes after the last action occur in
22 of 325 files (stale data from an overwritten file): keep them a warning.

The version 12 descriptor:

- The step's descriptor has no name and no class: it starts with the item count.
- Every identifier is a bare 4-byte code, with no length prefix: keys, classes, enum types and values,
  units, reference classes. No Unicode class names anywhere.
- Strings (`TEXT`, reference names) are the same UTF-16 strings as in version 16.

```text
step descriptor := count u32, count × (key [4], type [4], value)
object          := class [4], count u32, count × (key [4], type [4], value)

bool u8 | long i32 | comp i64 | doub f64 | UntF unit [4] + f64
UnFl unit [4] + n u32 + n × f64
TEXT UTF-16 string | enum type [4] + value [4]
Objc / GlbO object | type / GlbC class [4]
VlLs n u32 + n × (type [4] + value)
ObAr class [4] + object count u32 + count u32 + count × (key [4], type [4], value)
obj  n u32 + n × reference item
tdta / alis / Pth  u32 length + bytes

reference item := form [4] + class [4] + (prop: key [4] | Clss: nothing | Enmr: type [4] + value [4]
                  | rele: i32 | indx: u32 | Idnt: u32 | name: UTF-16 string)
```

AtnKit reads `indx` without its class code and has no case for `UnFl`, `ObAr` and `Pth `; with those
fixed, all 325 files parse. `Idnt` was not seen in a version 12 file; its layout follows `indx`.

**Into PhotoCraft's model:** every id becomes a 4-character code; the step descriptor gets the empty name
and class `null` that version 16 files use for code events; nested classes get the empty name. Save Actions
keeps writing version 16. Version 7 files (a different layout with 8-bit strings) and sets wrapped in
MacBinary stay rejected.

## Event reference: events not mapped yet

Each entry below is an event the reference application records that PhotoCraft does not map yet: its
4-character code and string ID, the PhotoCraft command it should map to, and every key the step carries,
in the order it is recorded, with type, unit, factory default and allowed values. When an event is
implemented, move it to the table above in the same change.

### How these values were measured

- **Keys, order, types, units, defaults:** each event was run in the reference application without a
  dialog on an RGB 8-bit document (with a selection, the clipboard or several layers where the step
  needs them), and the descriptor it returned was read back. That descriptor is what an action records.
  Settings were reset first, so the defaults are factory defaults. Steps that need a prepared document
  (a selection, the clipboard, several layers) were run with example values; their tables say so and show
  those values instead of defaults.
- **Ranges:** every number was set far above and below its range. The application clamps instead of
  refusing, so the values it returned are the exact limits. "Out-of-range values are clamped" in a table
  means exactly that; implement the same clamping on import.
- **Enum values:** every known enum value was tried; only values the application accepted and returned
  unchanged are listed. A value outside the list should keep the step as a kept step.
- **Names:** keys are stored by their 4-character code. The string ID column is a label; where one code
  has two names in the application, the one that fits the setting is shown.
- **Filter Gallery effects:** the "PhotoCraft today" column compares each value with PhotoCraft's own
  effect parameters (`crates/algo/src/artistic.rs`). All ranges and defaults already match; 26 parameters
  have a different name in PhotoCraft, given in that column.

### What an implementation needs to do

1. **Import:** `actions.load` maps the step to the command and params listed, using every key in the
   table; missing keys take the default from the table.
2. **Out-of-range values:** clamped to the allowed range, as the reference application does. Enum values
   outside the allowed list keep the step as a kept step, never a silent default.
3. **Export:** `actions.save` writes the same event with all keys in the order of the table, defaults
   included, with the units shown.
4. **Round trip:** load, then save, gives identical descriptors, for a step with defaults and for one
   with every value changed.
5. **Tests:** add the event to `every_mapped_command_exports_and_imports_back` and test the params that
   reach the command.

### Group A: The 47 Filter Gallery effects

Each Filter Gallery effect is recorded as its own event (one per effect), carrying a `GEfk` key with the effect id plus the effect's settings. Today only Spatter (`GEfk` = `Spt`) is mapped (to `filter.gallery.spatter`); the other effects load as kept steps that stop at playback. PhotoCraft already has every effect (`crates/algo/src/artistic.rs`) behind `filter.filterGallery`; this issue only maps them. Where our own range or default differs from the measured one, fix ours to match (marked in each table).

#### Accented Edges

- Event: `AccE` / `accentedEdges`
- PhotoCraft command: `filter.filterGallery effect "accentedEdges"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `EdgW` | edgeWidth | integer |  | 2 | 1 .. 14 (out-of-range values are clamped) | same |
| `EdgB` | edgeBrightness | integer |  | 38 | 0 .. 50 (out-of-range values are clamped) | same |
| `Smth` | smoothness | integer |  | 5 | 1 .. 15 (out-of-range values are clamped) | same |

#### Angled Strokes

- Event: `AngS` / `angledStrokes`
- PhotoCraft command: `filter.filterGallery effect "angledStrokes"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `DrcB` | directionBalance | integer |  | 50 | 0 .. 100 (out-of-range values are clamped) | same |
| `StrL` | strokeLength | integer |  | 15 | 3 .. 50 (out-of-range values are clamped) | same |
| `Shrp` | sharpness | integer |  | 3 | 0 .. 10 (out-of-range values are clamped) | maps to our `sharpness` (same range and default) |

#### Bas Relief

- Event: `BsRl` / `basRelief`
- PhotoCraft command: `filter.filterGallery effect "basRelief"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `Dtl ` | detail | integer |  | 13 | 1 .. 15 (out-of-range values are clamped) | same |
| `Smth` | smoothness | integer |  | 3 | 1 .. 15 (out-of-range values are clamped) | same |
| `LghD` | lightDirection | enumerated |  | `LDBt` lightDirBottom | `LDBL` lightDirBottomLeft, `LDBR` lightDirBottomRight, `LDBt` lightDirBottom, `LDLf` lightDirLeft, `LDRg` lightDirRight, `LDTL` lightDirTopLeft, `LDTR` lightDirTopRight, `LDTp` lightDirTop |  |

#### Chalk Charcoal

- Event: `ChlC` / `chalkCharcoal`
- PhotoCraft command: `filter.filterGallery effect "chalkCharcoal"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `ChrA` | charcoalArea | integer |  | 6 | 0 .. 20 (out-of-range values are clamped) | same |
| `ChlA` | chalkArea | integer |  | 6 | 0 .. 20 (out-of-range values are clamped) | same |
| `StrP` | strokePressure | integer |  | 1 | 0 .. 5 (out-of-range values are clamped) | same |

#### Charcoal

- Event: `Chrc` / `charcoal`
- PhotoCraft command: `filter.filterGallery effect "charcoal"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `ChAm` | charcoalAmount | integer |  | 1 | 1 .. 7 (out-of-range values are clamped) | maps to our `charcoalThickness` (same range and default) |
| `Dtl ` | detail | integer |  | 5 | 0 .. 5 (out-of-range values are clamped) | same |
| `LgDr` | lightDark | integer |  | 50 | 0 .. 100 (out-of-range values are clamped) | maps to our `lightDarkBalance` (same range and default) |

#### Chrome

- Event: `Chrm` / `chrome`
- PhotoCraft command: `filter.filterGallery effect "chrome"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `Dtl ` | detail | integer |  | 4 | 0 .. 10 (out-of-range values are clamped) | same |
| `Smth` | smoothness | integer |  | 7 | 0 .. 10 (out-of-range values are clamped) | same |

#### Colored Pencil

- Event: `ClrP` / `coloredPencil`
- PhotoCraft command: `filter.filterGallery effect "coloredPencil"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `Pncl` | pencilWidth | integer |  | 4 | 1 .. 24 (out-of-range values are clamped) | maps to our `pencilWidth` (same range and default) |
| `StrP` | strokePressure | integer |  | 8 | 0 .. 15 (out-of-range values are clamped) | same |
| `PprB` | paperBrightness | integer |  | 25 | 0 .. 50 (out-of-range values are clamped) | same |

#### Conte Crayon

- Event: `CntC` / `conteCrayon`
- PhotoCraft command: `filter.filterGallery effect "conteCrayon"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `FrgL` | foregroundLevel | integer |  | 11 | 1 .. 15 (out-of-range values are clamped) | same |
| `BckL` | backgroundLevel | integer |  | 7 | 1 .. 15 (out-of-range values are clamped) | maps to our `backgroundLevel` (same range and default) |
| `TxtT` | textureType | enumerated |  | `TxCa` texTypeCanvas | `TxBr` texTypeBrick, `TxBu` texTypeBurlap, `TxCa` texTypeCanvas, `TxSt` texTypeSandstone |  |
| `Scln` | scaling | integer |  | 100 | 50 .. 200 (out-of-range values are clamped) | same |
| `Rlf ` | relief | integer |  | 4 | 0 .. 50 (out-of-range values are clamped) | same |
| `LghD` | lightDirection | enumerated |  | `LDTp` lightDirTop | `LDBL` lightDirBottomLeft, `LDBR` lightDirBottomRight, `LDBt` lightDirBottom, `LDLf` lightDirLeft, `LDRg` lightDirRight, `LDTL` lightDirTopLeft, `LDTR` lightDirTopRight, `LDTp` lightDirTop |  |
| `InvT` | invertTexture | boolean |  | false |  |  |

#### Craquelure

- Event: `Crql` / `craquelure`
- PhotoCraft command: `filter.filterGallery effect "craquelure"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `CrcS` | crackSpacing | integer |  | 15 | 2 .. 100 (out-of-range values are clamped) | same |
| `CrcD` | crackDepth | integer |  | 6 | 0 .. 10 (out-of-range values are clamped) | same |
| `CrcB` | crackBrightness | integer |  | 9 | 0 .. 10 (out-of-range values are clamped) | same |

#### Crosshatch

- Event: `Crsh` / `crosshatch`
- PhotoCraft command: `filter.filterGallery effect "crosshatch"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `StrL` | strokeLength | integer |  | 9 | 3 .. 50 (out-of-range values are clamped) | same |
| `Shrp` | sharpness | integer |  | 6 | 0 .. 20 (out-of-range values are clamped) | maps to our `sharpness` (same range and default) |
| `Strg` | strength | integer |  | 1 | 1 .. 3 (out-of-range values are clamped) | maps to our `strength` (same range and default) |

#### Cutout

- Event: `Ct  ` / `cutout`
- PhotoCraft command: `filter.filterGallery effect "cutout"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `NmbL` | numberOfLayers | integer |  | 4 | 2 .. 8 (out-of-range values are clamped) | maps to our `numberOfLevels` (same range and default) |
| `EdgS` | edgeSimplicity | integer |  | 4 | 0 .. 10 (out-of-range values are clamped) | same |
| `EdgF` | edgeFidelity | integer |  | 2 | 1 .. 3 (out-of-range values are clamped) | same |

#### Dark Strokes

- Event: `DrkS` / `darkStrokes`
- PhotoCraft command: `filter.filterGallery effect "darkStrokes"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `Blnc` | balance | integer |  | 5 | 0 .. 10 (out-of-range values are clamped) | same |
| `BlcI` | blackIntensity | integer |  | 6 | 0 .. 10 (out-of-range values are clamped) | same |
| `WhtI` | whiteIntensity | integer |  | 2 | 0 .. 10 (out-of-range values are clamped) | same |

#### Diffuse Glow

- Event: `DfsG` / `diffuseGlow`
- PhotoCraft command: `filter.filterGallery effect "diffuseGlow"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `Grns` | graininess | integer |  | 6 | 0 .. 10 (out-of-range values are clamped) | same |
| `GlwA` | glowAmount | integer |  | 10 | 0 .. 20 (out-of-range values are clamped) | same |
| `ClrA` | clearAmount | integer |  | 15 | 0 .. 20 (out-of-range values are clamped) | same |

#### Dry Brush

- Event: `DryB` / `dryBrush`
- PhotoCraft command: `filter.filterGallery effect "dryBrush"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `BrsS` | brushSize | integer |  | 2 | 0 .. 10 (out-of-range values are clamped) | same |
| `BrsD` | brushDetail | integer |  | 8 | 0 .. 10 (out-of-range values are clamped) | same |
| `Txtr` | texture | integer |  | 1 | 1 .. 3 (out-of-range values are clamped) | same |

#### Film Grain

- Event: `FlmG` / `filmGrain`
- PhotoCraft command: `filter.filterGallery effect "filmGrain"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `Grn ` | grain | integer |  | 4 | 0 .. 20 (out-of-range values are clamped) | same |
| `HghA` | highlightArea | integer |  | 0 | 0 .. 20 (out-of-range values are clamped) | same |
| `Intn` | intensity | integer |  | 10 | 0 .. 10 (out-of-range values are clamped) | same |

#### Fresco

- Event: `Frsc` / `fresco`
- PhotoCraft command: `filter.filterGallery effect "fresco"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `BrsS` | brushSize | integer |  | 2 | 0 .. 10 (out-of-range values are clamped) | same |
| `BrsD` | brushDetail | integer |  | 8 | 0 .. 10 (out-of-range values are clamped) | same |
| `Txtr` | texture | integer |  | 1 | 1 .. 3 (out-of-range values are clamped) | same |

#### Glass

- Event: `Gls ` / `glass`
- PhotoCraft command: `filter.filterGallery effect "glass"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `Dstr` | distortion | integer |  | 5 | 0 .. 20 (out-of-range values are clamped) | maps to our `distortion` (same range and default) |
| `Smth` | smoothness | integer |  | 3 | 1 .. 15 (out-of-range values are clamped) | same |
| `TxtT` | textureType | enumerated |  | `TxFr` texTypeFrosted | `TxBr` texTypeBrick, `TxBu` texTypeBurlap, `TxCa` texTypeCanvas, `TxSt` texTypeSandstone |  |
| `Scln` | scaling | integer |  | 100 | 50 .. 200 (out-of-range values are clamped) | same |
| `InvT` | invertTexture | boolean |  | false |  |  |

#### Glowing Edges

- Event: `GlwE` / `glowingEdges`
- PhotoCraft command: `filter.filterGallery effect "glowingEdges"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `EdgW` | edgeWidth | integer |  | 2 | 1 .. 14 (out-of-range values are clamped) | same |
| `EdgB` | edgeBrightness | integer |  | 6 | 0 .. 20 (out-of-range values are clamped) | same |
| `Smth` | smoothness | integer |  | 5 | 1 .. 15 (out-of-range values are clamped) | same |

#### Graphic Pen

- Event: `GraP` / `graphicPen`
- PhotoCraft command: `filter.filterGallery effect "graphicPen"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `StrL` | strokeLength | integer |  | 15 | 1 .. 15 (out-of-range values are clamped) | same |
| `LgDr` | lightDark | integer |  | 50 | 0 .. 100 (out-of-range values are clamped) | maps to our `lightDarkBalance` (same range and default) |
| `SDir` | strokeDirection | enumerated |  | `SDRD` strokeDirRightDiag | `SDHz` strokeDirHorizontal, `SDLD` strokeDirLeftDiag, `SDRD` strokeDirRightDiag, `SDVt` strokeDirVertical |  |

#### Halftone Screen

- Event: `HlfS` / `halftoneScreen`
- PhotoCraft command: `filter.filterGallery effect "halftonePattern"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `HlSz` | halftoneSize | integer |  | 1 | 1 .. 12 (out-of-range values are clamped) | maps to our `size` (same range and default) |
| `Cntr` | contrast | integer |  | 5 | 0 .. 50 (out-of-range values are clamped) | maps to our `contrast` (same range and default) |
| `ScrT` | screenType | enumerated |  | `ScrD` screenDot | `ScrC` screenCircle, `ScrD` screenDot, `ScrL` screenLine |  |

#### Ink Outlines

- Event: `InkO` / `inkOutlines`
- PhotoCraft command: `filter.filterGallery effect "inkOutlines"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `StrL` | strokeLength | integer |  | 4 | 1 .. 50 (out-of-range values are clamped) | same |
| `DrkI` | darkIntensity | integer |  | 20 | 0 .. 50 (out-of-range values are clamped) | same |
| `LghI` | lightIntensity | integer |  | 10 | 0 .. 50 (out-of-range values are clamped) | same |

#### Mosaic  Tiles

- Event: `MscT` (no string ID)
- PhotoCraft command: `filter.filterGallery effect "mosaicTiles"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `TlSz` | tileSize | integer |  | 12 | 2 .. 100 (out-of-range values are clamped) | same |
| `GrtW` | groutWidth | integer |  | 3 | 1 .. 15 (out-of-range values are clamped) | same |
| `LghG` | lightenGrout | integer |  | 9 | 0 .. 10 (out-of-range values are clamped) | same |

#### Neon Glow

- Event: `NGlw` / `neonGlow`
- PhotoCraft command: `filter.filterGallery effect "neonGlow"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `Sz  ` | size | integer |  | 5 | -24 .. 24 (out-of-range values are clamped) | maps to our `glowSize` (same range and default) |
| `Brgh` | brightness | integer |  | 15 | 0 .. 50 (out-of-range values are clamped) | maps to our `glowBrightness` (same range and default) |
| `Clr ` | color | object |  | obj 'RGBC' RGBColor |  |  |

Nested values as recorded (defaults):
```
    'Rd  ' red : DOUBLETYPE = 0
    'Grn ' green : DOUBLETYPE = 0
    'Bl  ' blue : DOUBLETYPE = 255
```

#### Note Paper

- Event: `NtPr` / `notePaper`
- PhotoCraft command: `filter.filterGallery effect "notePaper"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `ImgB` | imageBalance | integer |  | 25 | 0 .. 50 (out-of-range values are clamped) | same |
| `Grns` | graininess | integer |  | 10 | 0 .. 20 (out-of-range values are clamped) | same |
| `Rlf ` | relief | integer |  | 11 | 0 .. 25 (out-of-range values are clamped) | same |

#### Ocean Ripple

- Event: `OcnR` / `oceanRipple`
- PhotoCraft command: `filter.filterGallery effect "oceanRipple"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `RplS` | rippleSize | integer |  | 9 | 1 .. 15 (out-of-range values are clamped) | same |
| `RplM` | rippleMagnitude | integer |  | 9 | 0 .. 20 (out-of-range values are clamped) | same |

#### Paint Daubs

- Event: `PntD` / `paintDaubs`
- PhotoCraft command: `filter.filterGallery effect "paintDaubs"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `Sz  ` | size | integer |  | 8 | 1 .. 50 (out-of-range values are clamped) | maps to our `brushSize` (same range and default) |
| `Shrp` | sharpness | integer |  | 7 | 0 .. 40 (out-of-range values are clamped) | maps to our `sharpness` (same range and default) |
| `BrsT` | brushType | enumerated |  | `BrSm` brushSimple | `BrDR` brushDarkRough, `BrSm` brushSimple, `BrSp` brushSparkle, `BrbW` brushWideBlurry, `BrsL` brushLightRough, `BrsW` brushWideSharp |  |

#### Palette Knife

- Event: `PltK` / `paletteKnife`
- PhotoCraft command: `filter.filterGallery effect "paletteKnife"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `StrS` | strokeSize | integer |  | 25 | 1 .. 50 (out-of-range values are clamped) | same |
| `StDt` | strokeDetail | integer |  | 3 | 1 .. 3 (out-of-range values are clamped) | same |
| `Sftn` | softness | integer |  | 0 | 0 .. 10 (out-of-range values are clamped) | same |

#### Patchwork

- Event: `Ptch` / `patchwork`
- PhotoCraft command: `filter.filterGallery effect "patchwork"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `SqrS` | squareSize | integer |  | 4 | 0 .. 10 (out-of-range values are clamped) | same |
| `Rlf ` | relief | integer |  | 8 | 0 .. 25 (out-of-range values are clamped) | same |

#### Photocopy

- Event: `Phtc` / `photocopy`
- PhotoCraft command: `filter.filterGallery effect "photocopy"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `Dtl ` | detail | integer |  | 7 | 1 .. 24 (out-of-range values are clamped) | same |
| `Drkn` | darkness | integer |  | 8 | 1 .. 50 (out-of-range values are clamped) | maps to our `darkness` (same range and default) |

#### Plaster

- Event: `Plst` / `plaster`
- PhotoCraft command: `filter.filterGallery effect "plaster"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `ImgB` | imageBalance | integer |  | 20 | 0 .. 50 (out-of-range values are clamped) | same |
| `Smth` | smoothness | integer |  | 2 | 1 .. 15 (out-of-range values are clamped) | same |
| `LghP` | lightPosition | enumerated |  | `LPTp` lightPosTop | `LPBL` lightPosBottomLeft, `LPBr` lightPosBottomRight, `LPBt` lightPosBottom, `LPLf` lightPosLeft, `LPRg` lightPosRight, `LPTL` lightPosTopLeft, `LPTR` lightPosTopRight, `LPTp` lightPosTop |  |

#### Plastic Wrap

- Event: `PlsW` / `plasticWrap`
- PhotoCraft command: `filter.filterGallery effect "plasticWrap"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `HghS` | highlightStrength | integer |  | 15 | 0 .. 20 (out-of-range values are clamped) | same |
| `Dtl ` | detail | integer |  | 9 | 1 .. 15 (out-of-range values are clamped) | same |
| `Smth` | smoothness | integer |  | 7 | 1 .. 15 (out-of-range values are clamped) | same |

#### Poster Edges

- Event: `PstE` / `posterEdges`
- PhotoCraft command: `filter.filterGallery effect "posterEdges"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `EdgT` | edgeThickness | integer |  | 2 | 0 .. 10 (out-of-range values are clamped) | same |
| `EdgI` | edgeIntensity | integer |  | 1 | 0 .. 10 (out-of-range values are clamped) | same |
| `Pstr` | posterization | integer |  | 2 | 0 .. 6 (out-of-range values are clamped) | same |

#### Reticulation

- Event: `Rtcl` / `reticulation`
- PhotoCraft command: `filter.filterGallery effect "reticulation"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `Dnst` | density | integer |  | 12 | 0 .. 50 (out-of-range values are clamped) | same |
| `BlcL` | blackLevel | integer |  | 40 | 0 .. 50 (out-of-range values are clamped) | maps to our `foregroundLevel` (same range and default) |
| `WhtL` | whiteLevel | integer |  | 5 | 0 .. 50 (out-of-range values are clamped) | maps to our `backgroundLevel` (same range and default) |

#### Rough Pastels

- Event: `RghP` / `roughPastels`
- PhotoCraft command: `filter.filterGallery effect "roughPastels"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `StrL` | strokeLength | integer |  | 6 | 0 .. 40 (out-of-range values are clamped) | same |
| `StDt` | strokeDetail | integer |  | 4 | 1 .. 20 (out-of-range values are clamped) | same |
| `TxtT` | textureType | enumerated |  | `TxCa` texTypeCanvas | `TxBr` texTypeBrick, `TxBu` texTypeBurlap, `TxCa` texTypeCanvas, `TxSt` texTypeSandstone |  |
| `Scln` | scaling | integer |  | 100 | 50 .. 200 (out-of-range values are clamped) | same |
| `Rlf ` | relief | integer |  | 20 | 0 .. 50 (out-of-range values are clamped) | same |
| `LghD` | lightDirection | enumerated |  | `LDBt` lightDirBottom | `LDBL` lightDirBottomLeft, `LDBR` lightDirBottomRight, `LDBt` lightDirBottom, `LDLf` lightDirLeft, `LDRg` lightDirRight, `LDTL` lightDirTopLeft, `LDTR` lightDirTopRight, `LDTp` lightDirTop |  |
| `InvT` | invertTexture | boolean |  | false |  |  |

#### Smudge Stick

- Event: `SmdS` / `smudgeStick`
- PhotoCraft command: `filter.filterGallery effect "smudgeStick"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `StrL` | strokeLength | integer |  | 2 | 0 .. 10 (out-of-range values are clamped) | same |
| `HghA` | highlightArea | integer |  | 0 | 0 .. 20 (out-of-range values are clamped) | same |
| `Intn` | intensity | integer |  | 10 | 0 .. 10 (out-of-range values are clamped) | same |

#### Spatter

- Event: `Spt ` / `spatter`
- PhotoCraft command: `filter.filterGallery effect "spatter"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `SprR` | sprayRadius | integer |  | 10 | 0 .. 25 (out-of-range values are clamped) | same |
| `Smth` | smoothness | integer |  | 5 | 1 .. 15 (out-of-range values are clamped) | same |

#### Sponge

- Event: `Spng` / `sponge`
- PhotoCraft command: `filter.filterGallery effect "sponge"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `BrsS` | brushSize | integer |  | 2 | 0 .. 10 (out-of-range values are clamped) | same |
| `Dfnt` | definition | integer |  | 12 | 0 .. 25 (out-of-range values are clamped) | same |
| `Smth` | smoothness | integer |  | 5 | 1 .. 15 (out-of-range values are clamped) | same |

#### Sprayed Strokes

- Event: `SprS` / `sprayedStrokes`
- PhotoCraft command: `filter.filterGallery effect "sprayedStrokes"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `StrL` | strokeLength | integer |  | 12 | 0 .. 20 (out-of-range values are clamped) | same |
| `SprR` | sprayRadius | integer |  | 7 | 0 .. 25 (out-of-range values are clamped) | same |
| `SDir` | strokeDirection | enumerated |  | `SDRD` strokeDirRightDiag | `SDHz` strokeDirHorizontal, `SDLD` strokeDirLeftDiag, `SDRD` strokeDirRightDiag, `SDVt` strokeDirVertical |  |

#### Stained Glass

- Event: `StnG` / `stainedGlass`
- PhotoCraft command: `filter.filterGallery effect "stainedGlass"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `ClSz` | cellSize | integer |  | 10 | 2 .. 50 (out-of-range values are clamped) | same |
| `BrdT` | borderThickness | integer |  | 4 | 1 .. 20 (out-of-range values are clamped) | same |
| `LghI` | lightIntensity | integer |  | 3 | 0 .. 10 (out-of-range values are clamped) | same |

#### Stamp

- Event: `Stmp` / `stamp`
- PhotoCraft command: `filter.filterGallery effect "stamp"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `LgDr` | lightDark | integer |  | 25 | 0 .. 50 (out-of-range values are clamped) | maps to our `lightDarkBalance` (same range and default) |
| `Smth` | smoothness | integer |  | 5 | 1 .. 50 (out-of-range values are clamped) | same |

#### Sumie

- Event: `Smie` / `sumie`
- PhotoCraft command: `filter.filterGallery effect "sumiE"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `StrW` | strokeWidth | integer |  | 10 | 3 .. 15 (out-of-range values are clamped) | same |
| `StrP` | strokePressure | integer |  | 2 | 0 .. 15 (out-of-range values are clamped) | same |
| `Cntr` | contrast | integer |  | 16 | 0 .. 40 (out-of-range values are clamped) | maps to our `contrast` (same range and default) |

#### Texturizer

- Event: `Txtz` / `texturizer`
- PhotoCraft command: `filter.filterGallery effect "texturizer"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `TxtT` | textureType | enumerated |  | `TxCa` texTypeCanvas | `TxBr` texTypeBrick, `TxBu` texTypeBurlap, `TxCa` texTypeCanvas, `TxSt` texTypeSandstone |  |
| `Scln` | scaling | integer |  | 100 | 50 .. 200 (out-of-range values are clamped) | same |
| `Rlf ` | relief | integer |  | 4 | 0 .. 50 (out-of-range values are clamped) | same |
| `LghD` | lightDirection | enumerated |  | `LDTp` lightDirTop | `LDBL` lightDirBottomLeft, `LDBR` lightDirBottomRight, `LDBt` lightDirBottom, `LDLf` lightDirLeft, `LDRg` lightDirRight, `LDTL` lightDirTopLeft, `LDTR` lightDirTopRight, `LDTp` lightDirTop |  |
| `InvT` | invertTexture | boolean |  | false |  |  |

#### Torn Edges

- Event: `TrnE` / `tornEdges`
- PhotoCraft command: `filter.filterGallery effect "tornEdges"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `ImgB` | imageBalance | integer |  | 25 | 0 .. 50 (out-of-range values are clamped) | same |
| `Smth` | smoothness | integer |  | 11 | 1 .. 15 (out-of-range values are clamped) | same |
| `Cntr` | contrast | integer |  | 17 | 1 .. 25 (out-of-range values are clamped) | maps to our `contrast` (same range and default) |

#### Underpainting

- Event: `Undr` / `underpainting`
- PhotoCraft command: `filter.filterGallery effect "underpainting"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `BrsS` | brushSize | integer |  | 6 | 0 .. 40 (out-of-range values are clamped) | same |
| `TxtC` | textureCoverage | integer |  | 16 | 0 .. 40 (out-of-range values are clamped) | maps to our `textureCoverage` (same range and default) |
| `TxtT` | textureType | enumerated |  | `TxCa` texTypeCanvas | `TxBr` texTypeBrick, `TxBu` texTypeBurlap, `TxCa` texTypeCanvas, `TxSt` texTypeSandstone |  |
| `Scln` | scaling | integer |  | 100 | 50 .. 200 (out-of-range values are clamped) | same |
| `Rlf ` | relief | integer |  | 4 | 0 .. 50 (out-of-range values are clamped) | same |
| `LghD` | lightDirection | enumerated |  | `LDTp` lightDirTop | `LDBL` lightDirBottomLeft, `LDBR` lightDirBottomRight, `LDBt` lightDirBottom, `LDLf` lightDirLeft, `LDRg` lightDirRight, `LDTL` lightDirTopLeft, `LDTR` lightDirTopRight, `LDTp` lightDirTop |  |
| `InvT` | invertTexture | boolean |  | false |  |  |

#### Water Paper

- Event: `WtrP` / `waterPaper`
- PhotoCraft command: `filter.filterGallery effect "waterPaper"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `FbrL` | fiberLength | integer |  | 15 | 3 .. 50 (out-of-range values are clamped) | same |
| `Brgh` | brightness | integer |  | 60 | 0 .. 100 (out-of-range values are clamped) | same |
| `Cntr` | contrast | integer |  | 80 | 0 .. 100 (out-of-range values are clamped) | maps to our `contrast` (same range and default) |

#### Watercolor

- Event: `Wtrc` / `watercolor`
- PhotoCraft command: `filter.filterGallery effect "watercolor"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `BrsD` | brushDetail | integer |  | 9 | 1 .. 14 (out-of-range values are clamped) | same |
| `ShdI` | shadingIntensity | integer |  | 1 | 0 .. 10 (out-of-range values are clamped) | maps to our `shadowIntensity` (same range and default) |
| `Txtr` | texture | integer |  | 1 | 1 .. 3 (out-of-range values are clamped) | same |

#### Grain

- Event: `Grn ` / `grain`
- PhotoCraft command: `filter.filterGallery effect "grain"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `Intn` | intensity | integer |  | 40 | 0 .. 100 (out-of-range values are clamped) | same |
| `Cntr` | contrast | integer |  | 50 | 0 .. 100 (out-of-range values are clamped) | maps to our `contrast` (same range and default) |
| `Grnt` | grainType | enumerated |  | `GrnR` grainRegular | `GrCn` grainContrasty, `GrSf` grainSoft, `GrSp` grainSpeckle, `GrSr` grainSprinkles, `GrSt` grainStippled, `GrnC` grainClumped, `GrnE` grainEnlarged, `GrnH` grainHorizontal, `GrnR` grainRegular, `GrnV` grainVertical |  |

### Group B: The Render filters Clouds, Difference Clouds, Fibers and Lens Flare

All four commands exist in PhotoCraft; only the `.atn` mapping is missing. Lighting Effects is covered with the events that need a recorded step (its lights are only recorded through the dialog).

#### Clouds

- Event: `Clds` / `clouds`
- PhotoCraft command: `filter.render.clouds`
- Uses the current foreground and background colours; the step itself carries no colours.

No parameters: the step carries an empty descriptor.

#### Difference Clouds

- Event: `DfrC` / `differenceClouds`
- PhotoCraft command: `filter.render.differenceClouds`
- Same as Clouds, blended with Difference.

No parameters: the step carries an empty descriptor.

#### Fibers

- Event: `Fbrs` (no string ID)
- PhotoCraft command: `filter.render.fibers`
- `randomSeed` makes playback reproduce the recorded result; keep it.

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `Vrnc` | (none) | integer |  | 16 | not clamped in the step (values are returned unchanged); limits pending, see "Events that need a step recorded through their dialog" |
| `Strg` | strengthPlugin | integer |  | 4 | not clamped in the step (values are returned unchanged); limits pending, see "Events that need a step recorded through their dialog" |
| `RndS` | randomSeed | integer |  | random | any 32-bit integer; a new random value per recording |

#### Lens Flare

- Event: `LnsF` / `lensFlare`
- PhotoCraft command: `filter.render.lensFlare`
- Lens types: `Zm  ` 50-300 mm Zoom, `Nkn ` 35 mm Prime, `Nkn1` 105 mm Prime, `PnVs` Movie Prime.

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `Brgh` | brightness | integer |  | 100 | 10 .. 300 (out-of-range values are clamped) |
| `FlrC` | flareCenter | object |  | obj 'Pnt ' point | `Hrzn` / `Vrtc` doubles, 0.4 / 0.4 by default; not clamped: (5, -5) is kept as given and plays |
| `Lns ` | lens | enumerated |  | `Zm  ` zoom | `Nkn ` nikon, `Nkn1` nikon105, `PnVs` panaVision, `Zm  ` zoom |

Nested values as recorded (defaults):
```
    'Hrzn' horizontal : DOUBLETYPE = 0.40000000596046
    'Vrtc' vertical : DOUBLETYPE = 0.40000000596046
```

### Group C: The Distort filters (Pinch, Polar Coordinates, Shear, Spherize, Twirl, Wave, ZigZag)

All commands exist in PhotoCraft (`filter.distort.*`); only the `.atn` mapping is missing. Displace needs a displacement map file and is listed separately at the end.

#### Pinch

- Event: `Pnch` / `pinch`
- PhotoCraft command: `filter.distort.pinch`

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `Amnt` | amount | integer |  | 50 | -100 .. 100 (out-of-range values are clamped) |

#### Polar Coordinates

- Event: `Plr ` / `polar`
- PhotoCraft command: `filter.distort.polarCoordinates`

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `Cnvr` | convert | enumerated |  | `RctP` rectToPolar | `PlrR` polarToRect, `RctP` rectToPolar |

#### Shear

- Event: `Shr ` / `shear`
- PhotoCraft command: `filter.distort.shear`
- `shearPoints` is a list of curve points; keep their order and count.

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `ShrP` | shearPoints | list |  | list[2] | list of `Pnt ` points (`Hrzn`, `Vrtc`) from top to bottom; default two points (0, 1) and (0, 128). The dialog writes whole numbers: `Hrzn` -64..63 (dragging past the grid edge clamps), `Vrtc` 1..128 with the first point at 1 and the last at 128; dragging a point out of the grid deletes it. A script run accepts any value without clamping (e.g. `Hrzn` ±10000, fractions) and crashed on a first point with `Vrtc` < 1: clamp to the dialog ranges on import |
| `UndA` | undefinedArea | enumerated |  | `WrpA` wrapAround | `RptE` repeatEdgePixels, `WrpA` wrapAround |
| `ShrS` | shearStart | integer |  | 0 | index of the first curve point; the dialog always writes 0 |
| `ShrE` | shearEnd | integer |  | 1 | index of the last curve point (point count - 1); the dialog writes it. Script runs with other values change the result, and far-off values (`ShrE` 100 or 1000, `ShrS` -1000) crashed the application: on import accept only `ShrS` 0 and `ShrE` = point count - 1, otherwise keep the step as a kept step |

Nested values as recorded (defaults):
```
    [0] OBJECTTYPE = obj 'Pnt ' point
        'Hrzn' horizontal : DOUBLETYPE = 0
        'Vrtc' vertical : DOUBLETYPE = 1
    [1] OBJECTTYPE = obj 'Pnt ' point
        'Hrzn' horizontal : DOUBLETYPE = 0
        'Vrtc' vertical : DOUBLETYPE = 128
```

#### Spherize

- Event: `Sphr` / `spherize`
- PhotoCraft command: `filter.distort.spherize`

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `Amnt` | amount | integer |  | 100 | -100 .. 100 (out-of-range values are clamped) |
| `SphM` | spherizeMode | enumerated |  | `Nrml` normal | `HrzO` horizontalOnly, `Nrml` normal, `VrtO` verticalOnly |

#### Twirl

- Event: `Twrl` / `twirl`
- PhotoCraft command: `filter.distort.twirl`

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `Angl` | angle | integer |  | 50 | -999 .. 999 (out-of-range values are clamped) |

#### Wave

- Event: `Wave` / `wave`
- PhotoCraft command: `filter.distort.wave`
- `randomSeed`: keep it so playback reproduces the recorded result.

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `Wvtp` | waveType | enumerated |  | `WvSn` waveSine | `WvSn` waveSine, `WvSq` waveSquare, `WvTr` waveTriangle |
| `NmbG` | numberOfGenerators | integer |  | 5 | 1 .. 999 (out-of-range values are clamped) |
| `WLMn` | wavelengthMin | integer |  | 10 | 1 .. 999 (out-of-range values are clamped) |
| `WLMx` | wavelengthMax | integer |  | 120 | 1 .. 999 (out-of-range values are clamped) |
| `AmMn` | amplitudeMin | integer |  | 5 | 1 .. 999 (out-of-range values are clamped) |
| `AmMx` | amplitudeMax | integer |  | 35 | 1 .. 999 (out-of-range values are clamped) |
| `SclH` | scaleHorizontal | integer |  | 100 | 1 .. 100 (out-of-range values are clamped) |
| `SclV` | scaleVertical | integer |  | 100 | 1 .. 100 (out-of-range values are clamped) |
| `UndA` | undefinedArea | enumerated |  | `RptE` repeatEdgePixels | `RptE` repeatEdgePixels, `WrpA` wrapAround |
| `RndS` | randomSeed | integer |  | random | any 32-bit integer; a new random value per recording |

#### ZigZag

- Event: `ZgZg` / `zigZag`
- PhotoCraft command: `filter.distort.zigZag`

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `Amnt` | amount | integer |  | 10 | -100 .. 100 (out-of-range values are clamped) |
| `NmbR` | numberOfRipples | integer |  | 5 | 0 .. 20 (out-of-range values are clamped) |
| `ZZTy` | zigZagType | enumerated |  | `PndR` pondRipples | `ArnC` aroundCenter, `OtFr` outFromCenter, `PndR` pondRipples |

#### Displace

Displace reads a displacement map from a file chosen in a dialog; record one step and add its keys (file path form) before implementing.

### Group D: The Stylize and Pixelate filters

All commands exist in PhotoCraft (`filter.stylize.*`, `filter.pixelate.*`); only the `.atn` mapping is missing.

#### Extrude

- Event: `Extr` / `extrude`
- PhotoCraft command: `filter.stylize.extrude`

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `ExtS` | extrudeSize | integer |  | 30 | 2 .. 255 (out-of-range values are clamped) |
| `ExtD` | extrudeDepth | integer |  | 30 | 1 .. 255 (out-of-range values are clamped) |
| `ExtF` | extrudeSolidFace | boolean |  | false |  |
| `ExtM` | extrudeMaskIncomplete | boolean |  | false |  |
| `ExtT` | extrudeType | enumerated |  | `Blks` blacks | `Blks` blacks, `Pyrm` pyramids |
| `ExtR` | extrudeRandom | enumerated |  | `Rndm` random | `LvlB` levelBased, `Rndm` random |

#### Find Edges

- Event: `FndE` / `findEdges`
- PhotoCraft command: `filter.stylize.findEdges`

No parameters: the step carries an empty descriptor.

#### Solarize

- Event: `Slrz` / `solarize`
- PhotoCraft command: `filter.stylize.solarize`

No parameters: the step carries an empty descriptor.

#### Trace Contour

- Event: `TrcC` / `traceContour`
- PhotoCraft command: `filter.stylize.traceContour`

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `Lvl ` | level | integer |  | 128 | whole number 0..255 (the dialog clamps to the nearest value). A recorded step carries the real level (e.g. 77); only a script run echoes 128 back. Levels <= 0 and >= 255 give the same image. |
| `Edg ` | edge | enumerated |  | `Upr ` upper | `CntE` enum: `Lwr ` lower, `Upr ` upper; recorded as set in the dialog |

#### Wind

- Event: `Wnd ` / `wind`
- PhotoCraft command: `filter.stylize.wind`

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `WndM` | windMethod | enumerated |  | `Wnd ` wind | `Blst` blast, `Stgr` stagger, `Wnd ` wind |
| `Drct` | direction | enumerated |  | `Left` left | `Left` left, `Rght` right |

#### Tiles

- Event: `Tls ` / `tiles`
- PhotoCraft command: `filter.stylize.tiles`

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `TlNm` | tileNumber | integer |  | 10 | 1 .. 99 (out-of-range values are clamped) |
| `TlOf` | tileOffset | integer |  | 10 | 1 .. 99 (out-of-range values are clamped) |
| `FlCl` | fillColor | enumerated |  | `FlBc` fillBack | `FlBc` fillBack, `FlFr` fillFore, `FlIn` fillInverse, `FlSm` fillSame |

#### Color Halftone

- Event: `ClrH` / `colorHalftone`
- PhotoCraft command: `filter.pixelate.colorHalftone`

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `Rds ` | radius | integer |  | 8 | 4 .. 127 (out-of-range values are clamped) |
| `Ang1` | angle1 | integer |  | 108 | -10000 .. 10000 (out-of-range values are clamped) |
| `Ang2` | angle2 | integer |  | 162 | -10000 .. 10000 (out-of-range values are clamped) |
| `Ang3` | angle3 | integer |  | 90 | -10000 .. 10000 (out-of-range values are clamped) |
| `Ang4` | angle4 | integer |  | 45 | -10000 .. 10000 (out-of-range values are clamped) |

#### Facet

- Event: `Fct ` / `facet`
- PhotoCraft command: `filter.pixelate.facet`

No parameters: the step carries an empty descriptor.

#### Fragment

- Event: `Frgm` / `fragment`
- PhotoCraft command: `filter.pixelate.fragment`

No parameters: the step carries an empty descriptor.

#### Mezzotint

- Event: `Mztn` / `mezzotint`
- PhotoCraft command: `filter.pixelate.mezzotint`

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `MztT` | mezzotintType | enumerated |  | `FnDt` fineDots | `CrsD` coarseDots, `FnDt` fineDots, `GrnD` grainyDots, `LngL` longLines, `LngS` longStrokes, `MdmD` mediumDots, `MdmL` mediumLines, `MdmS` mediumStrokes, `ShSt` shortStrokes, `ShrL` shortLines |

#### Mosaic

- Event: `Msc ` / `mosaic`
- PhotoCraft command: `filter.pixelate.mosaic`

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `ClSz` | cellSize | unitdouble |  | 2 | 2 .. 200 pixels (out-of-range values are clamped). Write it as a unit double in `#Pxl`: the application accepts that and returns it unchanged. Read a value without a unit as pixels (the application returns its own default that way). |

#### Pointillize

- Event: `Pntl` / `pointillize`
- PhotoCraft command: `filter.pixelate.pointillize`

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `ClSz` | cellSize | integer |  | 5 | 3 .. 300 (out-of-range values are clamped) |

### Group E: Video, Other and matting filters and the direct Gradient Map / Replace Color adjustments

All commands exist in PhotoCraft; only the `.atn` mapping is missing. Gradient Map and Replace Color here are the direct adjustments (Image > Adjustments), not adjustment layers (those are already mapped).

#### De-Interlace

- Event: `Dntr` / `deInterlace`
- PhotoCraft command: `filter.video.deInterlace`

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `IntE` | interlaceEliminateType | enumerated |  | `ElmO` eliminateOddFields | `ElmE` eliminateEvenFields, `ElmO` eliminateOddFields |
| `IntC` | interlaceCreateType | enumerated |  | `CrtI` createInterpolation | `CrtD` createDroplet, `CrtI` createInterpolation |

#### NTSC Colors

- Event: `NTSC` (no string ID)
- PhotoCraft command: `filter.video.ntscColors`

No parameters: the step carries an empty descriptor.

#### HSB/HSL

- Event: `HsbP` (no string ID)
- PhotoCraft command: `filter.other.hsbHsl`

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `Inpt` | input | enumerated |  | `RGBC` RGBColor | `HSBl` HSBColorEnum, `HSLC` HSLColor, `RGBC` RGBColor |
| `Otpt` | output | enumerated |  | `HSBl` HSBColorEnum | `HSBl` HSBColorEnum, `HSLC` HSLColor, `RGBC` RGBColor |

#### Remove Black Matte

- Event: `RmvB` / `removeBlackMatte`
- PhotoCraft command: `layer.matting.removeBlackMatte`

No parameters: the step carries an empty descriptor.

#### Remove White Matte

- Event: `RmvW` / `removeWhiteMatte`
- PhotoCraft command: `layer.matting.removeWhiteMatte`

No parameters: the step carries an empty descriptor.

#### Gradient Map (direct)

- Event: `GrMp` (no string ID)
- PhotoCraft command: `image.adjustments.gradientMap`
- The `gradient` object is the same gradient form the adjustment layer already maps; reuse that code.

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `` | gradientsInterpolationMethod | enumerated |  |  gradientInterpolationMethodType.'Gcls' classic |  |
| `Grad` | gradient | object |  | obj 'Grdn' gradientClassEvent |  |

Nested values as recorded (defaults):
```
    'Nm  ' name : STRINGTYPE = "Vorder- zu Hintergrundfarbe"
    'GrdF' gradientForm : ENUMERATEDTYPE = 'GrdF' gradientForm.'CstS' customStops
    'Intr' interfaceIconFrameDimmed : DOUBLETYPE = 4096
    'Clrs' colors : LISTTYPE = list[2]
        [0] OBJECTTYPE = obj 'Clrt' colorStop
            'Clr ' color : OBJECTTYPE = obj 'RGBC' RGBColor
                'Rd  ' red : DOUBLETYPE = 0
                'Grn ' green : DOUBLETYPE = 0
                'Bl  ' blue : DOUBLETYPE = 0
            'Type' type : ENUMERATEDTYPE = 'Clry' colorStopType.'UsrS' userStop
            'Lctn' location : INTEGERTYPE = 0
            'Mdpn' midpoint : INTEGERTYPE = 50
        [1] OBJECTTYPE = obj 'Clrt' colorStop
            'Clr ' color : OBJECTTYPE = obj 'RGBC' RGBColor
                'Rd  ' red : DOUBLETYPE = 255
                'Grn ' green : DOUBLETYPE = 255
                'Bl  ' blue : DOUBLETYPE = 255
            'Type' type : ENUMERATEDTYPE = 'Clry' colorStopType.'UsrS' userStop
            'Lctn' location : INTEGERTYPE = 4096
            'Mdpn' midpoint : INTEGERTYPE = 50
    'Trns' transparency : LISTTYPE = list[2]
        [0] OBJECTTYPE = obj 'TrnS' transferSpec
            'Opct' opacity : UNITDOUBLE = 100 '#Prc' percentUnit
            'Lctn' location : INTEGERTYPE = 0
            'Mdpn' midpoint : INTEGERTYPE = 50
        [1] OBJECTTYPE = obj 'TrnS' transferSpec
            'Opct' opacity : UNITDOUBLE = 100 '#Prc' percentUnit
            'Lctn' location : INTEGERTYPE = 4096
            'Mdpn' midpoint : INTEGERTYPE = 50
```

#### Average

- Event: `Avrg` (no string ID)
- PhotoCraft command: `filter.blur.average`
- Measured with example values in a prepared document: the Value column shows what was sent and recorded, not a factory default (a step always carries the values the user set).

No parameters: the step carries an empty descriptor.

#### Custom

- Event: `Cstm` / `custom`
- PhotoCraft command: `filter.other.custom`
- Measured with example values in a prepared document: the Value column shows what was sent and recorded, not a factory default (a step always carries the values the user set).

| Key | String ID | Type | Unit | Example value | Allowed |
|---|---|---|---|---|---|
| `Scl ` | scale | integer |  | 1 | 1 .. 9999 (measured in the dialog). Without the dialog the value is ignored |
| `Ofst` | offset | integer |  | 0 | -9999 .. 9999 (measured in the dialog). Without the dialog the value is ignored |
| `Mtrx` | matrix | list |  | none: a step always carries all 25 cells | exactly 25 integers (5 x 5, row by row), each -999 .. 999; a 3 x 3 list makes the application fail |

#### Replace Color

- Event: `RplC` / `replaceColor`
- PhotoCraft command: `image.adjustments.replaceColor`
- Measured with example values in a prepared document: the Value column shows what was sent and recorded, not a factory default (a step always carries the values the user set).

| Key | String ID | Type | Unit | Example value | Allowed |
|---|---|---|---|---|---|
| `Fzns` | fuzziness | integer |  | 40 | 0 .. 200 (out-of-range values are clamped) |
| `Mnm ` | minimum | object |  | obj 'LbCl' labColor |  |
| `Mxm ` | maximum | object |  | obj 'LbCl' labColor |  |
| `H   ` | hue | integer |  | 30 | -180 .. 180 (out-of-range values are clamped) |
| `Strt` | saturation | integer |  | 0 | -100 .. 100 (out-of-range values are clamped) |
| `Lght` | lightness | integer |  | 0 | -100 .. 100 (out-of-range values are clamped) |
| `` | colorModel | integer |  | 0 |  |

Nested values as recorded (example values):
```
    'Lmnc' luminance : DOUBLETYPE = 40
    'A   ' a : DOUBLETYPE = 30
    'B   ' b : DOUBLETYPE = 40
    'Lmnc' luminance : DOUBLETYPE = 40
    'A   ' a : DOUBLETYPE = 30
    'B   ' b : DOUBLETYPE = 40
```

### Group F: Selection combinations, Paste Into/Outside, Fade, Link, Distribute, Define Pattern and older merge/mask events

Steps recorded in an ordinary editing session. The values below were measured in a document set up for each step (a selection, the clipboard, linked or several selected layers). Align, Copy/Paste Layer Style and Rename are already mapped under their current events and are not repeated here.

#### Add to Selection (rectangle)

- Event: `AddT` / `addTo`
- PhotoCraft command: `select.rect {"mode":"add"}`
- Shift-drag with the Rectangular Marquee.
- Measured with example values in a prepared document: the Value column shows what was sent and recorded, not a factory default (a step always carries the values the user set).

| Key | String ID | Type | Unit | Example value | Allowed |
|---|---|---|---|---|---|
| `null` | null | reference |  | ref 'Chnl' channel prop 'fsel' selection |  |
| `T   ` | to | object |  | obj 'Rctn' rectangle |  |

Nested values as recorded (example values):
```
    'Top ' top : UNITDOUBLE = 30 '#Pxl' pixelsUnit
    'Left' left : UNITDOUBLE = 30 '#Pxl' pixelsUnit
    'Btom' bottom : UNITDOUBLE = 90 '#Pxl' pixelsUnit
    'Rght' right : UNITDOUBLE = 90 '#Pxl' pixelsUnit
```

#### Add to Selection (ellipse)

- Event: `AddT` / `addTo`
- PhotoCraft command: `select.rect {"mode":"add","ellipse":true}`
- Same event with an `Elps` target and anti-aliasing.
- Measured with example values in a prepared document: the Value column shows what was sent and recorded, not a factory default (a step always carries the values the user set).

| Key | String ID | Type | Unit | Example value | Allowed |
|---|---|---|---|---|---|
| `null` | null | reference |  | ref 'Chnl' channel prop 'fsel' selection |  |
| `T   ` | to | object |  | obj 'Elps' ellipse |  |
| `AntA` | antiAlias | boolean |  | true |  |

Nested values as recorded (example values):
```
    'Top ' top : UNITDOUBLE = 30 '#Pxl' pixelsUnit
    'Left' left : UNITDOUBLE = 30 '#Pxl' pixelsUnit
    'Btom' bottom : UNITDOUBLE = 90 '#Pxl' pixelsUnit
    'Rght' right : UNITDOUBLE = 90 '#Pxl' pixelsUnit
```

#### Subtract from Selection

- Event: `SbtF` / `subtractFrom`
- PhotoCraft command: `select.rect {"mode":"subtract"}`
- Measured with example values in a prepared document: the Value column shows what was sent and recorded, not a factory default (a step always carries the values the user set).

| Key | String ID | Type | Unit | Example value | Allowed |
|---|---|---|---|---|---|
| `null` | null | reference |  | ref 'Chnl' channel prop 'fsel' selection |  |
| `T   ` | to | object |  | obj 'Rctn' rectangle |  |

Nested values as recorded (example values):
```
    'Top ' top : UNITDOUBLE = 30 '#Pxl' pixelsUnit
    'Left' left : UNITDOUBLE = 30 '#Pxl' pixelsUnit
    'Btom' bottom : UNITDOUBLE = 60 '#Pxl' pixelsUnit
    'Rght' right : UNITDOUBLE = 60 '#Pxl' pixelsUnit
```

#### Intersect with Selection

- Event: `IntW` / `interfaceWhite`
- PhotoCraft command: `select.rect {"mode":"intersect"}`
- Measured with example values in a prepared document: the Value column shows what was sent and recorded, not a factory default (a step always carries the values the user set).

| Key | String ID | Type | Unit | Example value | Allowed |
|---|---|---|---|---|---|
| `null` | null | reference |  | ref 'Chnl' channel prop 'fsel' selection |  |
| `T   ` | to | object |  | obj 'Rctn' rectangle |  |

Nested values as recorded (example values):
```
    'Top ' top : UNITDOUBLE = 30 '#Pxl' pixelsUnit
    'Left' left : UNITDOUBLE = 30 '#Pxl' pixelsUnit
    'Btom' bottom : UNITDOUBLE = 120 '#Pxl' pixelsUnit
    'Rght' right : UNITDOUBLE = 120 '#Pxl' pixelsUnit
```

#### Paste Into

- Event: `PstI` / `pasteInto`
- PhotoCraft command: `edit.pasteSpecial.pasteInto`
- Measured with example values in a prepared document: the Value column shows what was sent and recorded, not a factory default (a step always carries the values the user set).

| Key | String ID | Type | Unit | Example value | Allowed |
|---|---|---|---|---|---|
| `AntA` | antiAlias | enumerated |  | `Anno` antiAliasNone | not measured yet |
| `As  ` | as | class |  | class 'Pxel' pixel |  |

#### Paste Outside

- Event: `PstO` / `pasteOutside`
- PhotoCraft command: `edit.pasteSpecial.pasteOutside`
- Measured with example values in a prepared document: the Value column shows what was sent and recorded, not a factory default (a step always carries the values the user set).

| Key | String ID | Type | Unit | Example value | Allowed |
|---|---|---|---|---|---|
| `AntA` | antiAlias | enumerated |  | `Anno` antiAliasNone | not measured yet |
| `As  ` | as | class |  | class 'Pxel' pixel |  |

#### Fade

- Event: `Fade` / `fade`
- PhotoCraft command: `edit.fade`
- Recorded right after the step it fades.
- Measured with example values in a prepared document: the Value column shows what was sent and recorded, not a factory default (a step always carries the values the user set).

| Key | String ID | Type | Unit | Example value | Allowed |
|---|---|---|---|---|---|
| `Opct` | opacity | unitdouble | `#Prc` | 50 | 0 .. 100 (out-of-range values are clamped) |
| `Md  ` | mode | enumerated |  | `Mltp` multiply | not measured yet |

#### Link Layers

- Event: `` / `linkSelectedLayers`
- PhotoCraft command: `layer.linkLayers`
- Measured with example values in a prepared document: the Value column shows what was sent and recorded, not a factory default (a step always carries the values the user set).

| Key | String ID | Type | Unit | Example value | Allowed |
|---|---|---|---|---|---|
| `null` | null | reference |  | ref 'Lyr ' layer enum 'Ordn' ordinal.'Trgt' targetEnum |  |

#### Distribute

- Event: `Dstr` / `distort`
- PhotoCraft command: `layer.distribute.leftEdges (one command per edge)`
- `Usng` selects the edge or centre; one enum value per Distribute menu item.
- Measured with example values in a prepared document: the Value column shows what was sent and recorded, not a factory default (a step always carries the values the user set).

| Key | String ID | Type | Unit | Example value | Allowed |
|---|---|---|---|---|---|
| `null` | null | reference |  | ref 'Lyr ' layer enum 'Ordn' ordinal.'Trgt' targetEnum |  |
| `Usng` | using | enumerated |  | `AdLf` ADSLefts | not measured yet |

#### Remove Layer Mask (older files)

- Event: `RmvL` / `removeLayerMask`
- PhotoCraft command: `layer.layerMask.delete`
- The reference application asks "Apply mask before deleting?" when this step plays without the apply choice; see the table for the key it records.
- Measured with example values in a prepared document: the Value column shows what was sent and recorded, not a factory default (a step always carries the values the user set).

No parameters: the step carries an empty descriptor.

#### Define Pattern

- Event: `DfnP` / `definePattern`
- PhotoCraft command: `edit.definePattern`

No parameters: the step carries an empty descriptor.

#### Merge Layers (older files)

- Event: `MrgL` (no string ID)
- PhotoCraft command: `layer.mergeLayers`

No parameters: the step carries an empty descriptor.

### Group G: Split Channels, Merge Spot Channel, Trap and Match Color

Channel and colour steps. Trap was measured in a CMYK document (it only exists there).

#### Split Channels

- Event: `SplC` / `splitChannels`
- PhotoCraft command: `channel.split`
- Measured with example values in a prepared document: the Value column shows what was sent and recorded, not a factory default (a step always carries the values the user set).

| Key | String ID | Type | Unit | Example value | Allowed |
|---|---|---|---|---|---|
| `` | documentIDs | list |  | list[3] |  |

Nested values as recorded (example values):
```
    [0] 182
    [1] 185
    [2] 188
```

#### Merge Spot Channel

- Event: `MSpt` / `mergeSpotChannel`
- PhotoCraft command: `channel.mergeSpot`
- Measured with example values in a prepared document: the Value column shows what was sent and recorded, not a factory default (a step always carries the values the user set).

No parameters: the step carries an empty descriptor.

#### Trap

- Event: `Trap` / `trap`
- PhotoCraft command: `image.trap`
- Measured with example values in a prepared document: the Value column shows what was sent and recorded, not a factory default (a step always carries the values the user set).

| Key | String ID | Type | Unit | Example value | Allowed |
|---|---|---|---|---|---|
| `Wdth` | width | unitdouble | `#Pxl` | 2 | up to 32767 (clamped); the minimum was not measured (a huge negative value ran out of memory) |

#### Match Color

- Event: `` / `matchColor`
- PhotoCraft command: `image.adjustments.matchColor`

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `Lght` | lightness | integer |  | 100 |  |
| `ClrR` | colorRange | integer |  | 100 |  |
| `Fade` | fade | integer |  | 0 |  |
| `` | noReference | boolean |  | true |  |

### Group I: Newer features (Focus Area, Remove Background, Select Sky, Oil Paint, Smart Sharpen)

These events only have a string ID (no 4-character code). Steps recorded in newer versions use them.

#### Select > Focus Area

- Event: `` / `focusMask`
- PhotoCraft command: `select.focusArea`

No parameters: the step carries an empty descriptor.

#### Remove Background (quick action)

- Event: `` / `removeBackground`
- PhotoCraft command: `layer.removeBackground`

No parameters: the step carries an empty descriptor.

#### Select > Sky

- Event: `` / `selectSky`
- PhotoCraft command: `select.sky`

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `` | sampleAllLayers | boolean |  | false |  |

#### Filter > Stylize > Oil Paint

- Event: `` / `oilPaint`
- PhotoCraft command: `filter.stylize.oilPaint`

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `` | lightingOn | boolean |  | true |  |
| `` | stylization | double |  | 4 |  |
| `` | cleanliness | double |  | 2.29999995231628 |  |
| `` | brushScale | double |  | 0.80000001192093 |  |
| `` | microBrush | double |  | 10 |  |
| `LghD` | lightDirection | integer |  | -60 |  |
| `` | specularity | double |  | 1.29999995231628 |  |

#### Filter > Sharpen > Smart Sharpen

- Event: `` / `smartSharpen`
- PhotoCraft command: `filter.sharpen.smartSharpen`

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `` | presetKind | enumerated |  |  presetKindType. presetKindCustom |  |
| `` | useLegacy | boolean |  | true |  |
| `Amnt` | amount | unitdouble | `#Prc` | 200 |  |
| `Rds ` | radius | unitdouble | `#Pxl` | 1 |  |
| `` | moreAccurate | boolean |  | false |  |
| `blur` | blur | enumerated |  |  blurType. lensBlur |  |

### Events that need a step recorded through their dialog

These carry settings that only exist once their dialog was used, so a dialog-less run returns no
values or ignores the ones it was given. They are listed so nobody measures them again from scratch;
their tables will be added once a recorded step was read.

| Event | Why |
|---|---|
| Generative Fill (`syntheticFill`) | AI feature: in PhotoCraft this goes through the shared AI layer; until then such steps stay kept steps. |

### Recorded under another event

Some older events are no longer what the application records; PhotoCraft already maps the current one:
Clear (recorded as Delete), Rasterize Type (`rasterizeLayer`), Convert to Profile (`convertToProfile`),
Copy and Paste Layer Style (`copyEffects`, `pasteEffects`), Align (`align`), Rename Layer (Set of the
layer's name), Deselect (Set of the selection to None). Steps recorded with Backlight, Fill Flash, Color
Cast, Variations, Texture Fill and the old Snapshot events come only from very old versions and stay kept
steps.
