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

## Action playback history

Actions create one history entry named `Action: <name>` by default. Nested plays
share the outer action's entry, and a failing or Stop step keeps the completed
prefix. A play with no document change adds no entry and preserves redo.
`actions.setHistory {"singleState":false}` restores per-step history. The option
is saved as `actions.createSingleHistoryState` in preferences and defaults to true
when loading older preferences. It is separate from Playback Options' performance
modes; the Playback Options dialog can use the same setting when #5212 lands.

Step-dialog confirmation joins the original play only while its document and
history still match the paused state. Cancel keeps the completed prefix; an
intervening edit or document switch prevents the stale dialog from applying.
Batch and droplets group each processed document independently under the action's
name and use the same saved option.
Existing non-linear History ordering and visibility options continue to apply.

Explicit Clear History and Purge Histories steps keep their normal effects. Later
edits form a grouped entry from that new boundary, without restoring discarded
states. If an action explicitly chooses an intermediate row as the History Brush
source, its pixels are retained as a named snapshot when grouping removes that
row; the action still adds only one Undo entry.

## Supported events

| Code | String ID | Label | PhotoCraft command(s) |
|---|---|---|---|
| `AdNs` | addNoise | Add Noise | `filter.noise.addNoise` |
| `Algn` | align | Align | `layer.align.leftEdges`, `layer.align.horizontalCenters`, `layer.align.rightEdges`, `layer.align.topEdges`, `layer.align.verticalCenters`, `layer.align.bottomEdges` |
| `AppI` | applyImageEvent | Apply Image | `image.applyImage` |
| (string ID only) | applyComp | Apply Layer Comp | `layerComp.apply` |
| (string ID only) | applyLocking | Apply Locking | `layer.lockLayers` |
| (string ID only) | assignProfile | Assign Profile | `edit.assignProfile` |
| (string ID only) | autoColor | Auto Color | `image.autoColor` |
| `Avrg` | Avrg | Average | `filter.blur.average` |
| (string ID only) | blackAndWhite | Black & White | `image.adjustments.blackWhite` |
| `Blr ` | blur | Blur | `filter.blur.blur` |
| `BlrM` | blurMore | Blur More | `filter.blur.blurMore` |
| `Brdr` | border | Border | `select.modify.border` |
| (string ID only) | boxblur | Box Blur | `filter.blur.boxBlur` |
| `BrgC` | brightnessEvent | Brightness/Contrast | `image.adjustments.brightnessContrast` |
| (string ID only) | Adobe Camera Raw Filter | Camera Raw Filter | `filter.cameraRaw` |
| `CnvS` | canvasSize | Canvas Size | `image.canvasSize` |
| `ChnM` | channelMixer | Channel Mixer | `image.adjustments.channelMixer` |
| (string ID only) | clearEvent | Clear | `select.editInQuickMaskMode`, `history.clear` |
| `Cls ` | close | Close | `file.close` |
| `Clds` | clouds | Clouds | `filter.render.clouds` |
| (string ID only) | collapseAllGroupsEvent | Collapse All Groups | `layer.setExpanded` |
| `ClrB` | colorBalance | Color Balance | `image.adjustments.colorBalance` |
| `ClrR` | colorRange | Color Range | `select.colorRange` |
| (string ID only) | cafWorkspace | Content-Aware Fill | `edit.contentAwareFill` |
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
| `Cstm` | custom | Custom | `filter.other.custom` |
| `cut ` | cut | Cut | `edit.cut` |
| `Dntr` | deInterlace | De-Interlace | `filter.video.deInterlace` |
| `Dfrg` | defringe | Defringe | `layer.matting.defringe` |
| `Dlt ` | delete | Delete | `layer.delete`, `edit.clear`, `layer.layerMask.delete`, `layer.smartFilter.deleteFilterMask` |
| `Dstt` | desaturate | Desaturate | `image.adjustments.desaturate` |
| (string ID only) | selectNoLayers | Deselect Layers | `select.deselectLayers` |
| `Dspc` | despeckle | Despeckle | `filter.noise.despeckle` |
| `DfrC` | differenceClouds | Difference Clouds | `filter.render.differenceClouds` |
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
| `Fbrs` |  | Fibers | `filter.render.fibers` |
| `Fl  ` | fill | Fill | `edit.fill`, `paint.bucket` |
| `GEfc` | filterGallery | Filter Gallery | `filter.gallery.accentedEdges`, `filter.gallery.angledStrokes`, `filter.gallery.basRelief`, `filter.gallery.chalkCharcoal`, `filter.gallery.charcoal`, `filter.gallery.chrome`, `filter.gallery.coloredPencil`, `filter.gallery.conteCrayon`, `filter.gallery.craquelure`, `filter.gallery.crosshatch`, `filter.gallery.cutout`, `filter.gallery.darkStrokes`, `filter.gallery.diffuseGlow`, `filter.gallery.dryBrush`, `filter.gallery.filmGrain`, `filter.gallery.fresco`, `filter.gallery.glass`, `filter.gallery.glowingEdges`, `filter.gallery.graphicPen`, `filter.gallery.halftonePattern`, `filter.gallery.inkOutlines`, `filter.gallery.mosaicTiles`, `filter.gallery.neonGlow`, `filter.gallery.notePaper`, `filter.gallery.oceanRipple`, `filter.gallery.paintDaubs`, `filter.gallery.paletteKnife`, `filter.gallery.patchwork`, `filter.gallery.photocopy`, `filter.gallery.plaster`, `filter.gallery.plasticWrap`, `filter.gallery.posterEdges`, `filter.gallery.reticulation`, `filter.gallery.roughPastels`, `filter.gallery.smudgeStick`, `filter.gallery.spatter`, `filter.gallery.sponge`, `filter.gallery.sprayedStrokes`, `filter.gallery.stainedGlass`, `filter.gallery.stamp`, `filter.gallery.sumiE`, `filter.gallery.texturizer`, `filter.gallery.tornEdges`, `filter.gallery.underpainting`, `filter.gallery.waterPaper`, `filter.gallery.watercolor`, `filter.gallery.grain` |
| `FltI` | flattenImage | Flatten Image | `layer.flattenImage` |
| `Flip` | flip | Flip | `image.imageRotation.flipCanvasHorizontal`, `image.imageRotation.flipCanvasVertical`, `edit.transform.flipHorizontal`, `edit.transform.flipVertical` |
| (string ID only) | focusMask | Focus Area | `select.focusArea` |
| `GsnB` | gaussianBlur | Gaussian Blur | `filter.blur.gaussianBlur` |
| `Grdn` | gradientClassEvent | Classic Gradient (custom RGB, centred midpoints) | `paint.gradient` |
| (string ID only) | gradientMapEvent | Gradient Map | `image.adjustments.gradientMap` |
| `GrMp` | (code only) | Gradient Map (recorded classic form) | `image.adjustments.gradientMap` |
| `RplC` | replaceColor | Replace Color (Lab sample box) | `image.adjustments.replaceColor` |
| `Grow` | grow | Grow | `select.grow` |
| `Hd  ` | hide | Hide | `layer.hideLayers` |
| `HghP` | highPass | High Pass | `filter.other.highPass` |
| `HsbP` | HsbP | HSB/HSL | `filter.other.hsbHsl` |
| `HStr` | hueSaturation | Hue/Saturation | `image.adjustments.hueSaturation` |
| (string ID only) | conditional | If | `actions.conditional` |
| `ImgS` | imageSize | Image Size | `image.imageSize` |
| `Invs` | inverse | Inverse | `select.inverse` |
| `Invr` | invert | Invert | `image.adjustments.invert` |
| `CpTL` | copyToLayer | Layer Via Copy | `layer.new.layerViaCopy` |
| `CtTL` | cutToLayer | Layer Via Cut | `layer.new.layerViaCut` |
| `LnsF` | lensFlare | Lens Flare | `filter.render.lensFlare` |
| `Lvls` | levels | Levels | `image.adjustments.levels`, `image.autoTone`, `image.autoContrast` |
| `Mk  ` | make | Make | `layer.layerStyle.createLayer`, `layer.new.layer`, `layer.new.group`, `layer.groupLayers`, `layer.duplicate`, `layer.new.layerViaCopy`, `layer.new.layerViaCut`, `layer.newFillLayer.solidColor`, `layer.newFillLayer.pattern`, `layer.newAdjustmentLayer.levels`, `layer.newAdjustmentLayer.curves`, `layer.newAdjustmentLayer.hueSaturation`, `layer.newAdjustmentLayer.brightnessContrast`, `layer.newAdjustmentLayer.threshold`, `layer.newAdjustmentLayer.posterize`, `layer.newAdjustmentLayer.invert`, `layer.newAdjustmentLayer.colorBalance`, `layer.newAdjustmentLayer.vibrance`, `layer.newAdjustmentLayer.exposure`, `layer.newAdjustmentLayer.photoFilter`, `layer.newAdjustmentLayer.blackWhite`, `layer.newAdjustmentLayer.channelMixer`, `layer.newAdjustmentLayer.selectiveColor`, `layer.newAdjustmentLayer.gradientMap`, `layer.layerMask.revealAll`, `layer.layerMask.hideAll`, `layer.layerMask.revealSelection`, `layer.layerMask.hideSelection`, `file.new`, `file.newFromClipboard`, `history.newSnapshot`, `type.convertToShape`, `type.createWorkPath` |
| (string ID only) | animationFramesFromLayers | Make Frames from Layers |  |
| (string ID only) | matchColor | Match Color | `image.adjustments.matchColor` |
| `Mxm ` | maximum | Maximum | `filter.other.maximum` |
| `Mdn ` | median | Median | `filter.noise.median` |
| `Mrg2` | mergeLayersNew | Merge Layers | `layer.mergeLayers` |
| `MSpt` | mergeSpotChannel | Merge Spot Channel | `channel.mergeSpot` |
| `MrgV` | mergeVisible | Merge Visible | `layer.mergeVisible`, `layer.stampVisible` |
| `Mnm ` | minimum | Minimum | `filter.other.minimum` |
| `MtnB` | motionBlur | Motion Blur | `filter.blur.motionBlur` |
| `move` | move | Move | `layer.translate`, `layer.moveTo`, `layer.arrange.bringToFront`, `layer.arrange.sendToBack`, `layer.arrange.bringForward`, `layer.arrange.sendBackward` |
| `NTSC` | NTSC | NTSC Colors | `filter.video.ntscColors` |
| `Ofst` | offset | Offset | `filter.other.offset` |
| (string ID only) | oilPaint | Oil Paint | `filter.stylize.oilPaint` |
| `Opn ` | open | Open | `file.openAs` |
| `past` | paste | Paste | `edit.paste`, `edit.pasteSpecial.pasteInPlace` |
| (string ID only) | pasteEffects | Paste Layer Style | `layer.layerStyle.pasteLayerStyle` |
| (string ID only) | photoFilter | Photo Filter | `image.adjustments.photoFilter` |
| `Pnch` | pinch | Pinch | `filter.distort.pinch` |
| `Plc ` | placeEvent | Place | `file.placeEmbedded`, `file.placeLinked` |
| `Ply ` | play | Play | `actions.play` |
| `Plr ` | polar | Polar Coordinates | `filter.distort.polarCoordinates` |
| `Pstr` | posterize | Posterize | `image.adjustments.posterize` |
| `Prge` | purge | Purge | `edit.purge.all`, `edit.purge.clipboard`, `edit.purge.histories`, `edit.purge.undo` |
| `RdlB` | radialBlur | Radial Blur | `filter.blur.radialBlur` |
| (string ID only) | rasterizeLayer | Rasterize | `layer.rasterize.layer` |
| `Ungr` | ungroup | Release Clipping Mask | `layer.releaseClippingMask` |
| (string ID only) | removeBackground | Remove Background | `layer.removeBackground` |
| `RmvB` | removeBlackMatte | Remove Black Matte | `layer.matting.removeBlackMatte` |
| `RmvW` | removeWhiteMatte | Remove White Matte | `layer.matting.removeWhiteMatte` |
| `Shr ` | shear | Shear | `filter.distort.shear` |
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
| `Shw ` | show | Show | `layer.showLayers`, `layer.showOnly` |
| `Smlr` | similar | Similar | `select.similar` |
| (string ID only) | selectSky | Sky | `select.sky` |
| (string ID only) | scaleEffectsEvent | Scale Effects | `layer.layerStyle.scaleEffects` |
| `SmrB` | smartBlur | Smart Blur | `filter.blur.smartBlur` |
| (string ID only) | smartSharpen | Smart Sharpen | `filter.sharpen.smartSharpen` |
| `Smth` | smoothness | Smooth | `select.modify.smooth` |
| `Sphr` | spherize | Spherize | `filter.distort.spherize` |
| `SplC` | splitChannels | Split Channels | `channel.split` |
| `Stop` | stop | Stop | `STOP_MESSAGE` |
| `Strk` | stroke | Stroke | `edit.stroke` |
| (string ID only) | surfaceBlur | Surface Blur | `filter.blur.surfaceBlur` |
| `Thrs` | threshold | Threshold | `image.adjustments.threshold` |
| `Trnf` | transform | Transform | `edit.transform`, `select.transformSelection` |
| `Trap` | trap | Trap | `image.trap` |
| (string ID only) | trim | Trim | `image.trim` |
| `Twrl` | twirl | Twirl | `filter.distort.twirl` |
| (string ID only) | ungroupLayersEvent | Ungroup Layers | `layer.ungroupLayers` |
| `UnsM` | unsharpMask | Unsharp Mask | `filter.sharpen.unsharpMask` |
| (string ID only) | updatePlacedLayer | Update Smart Objects | `layer.smartObjects.updateModifiedContent` |
| (string ID only) | vibrance | Vibrance | `image.adjustments.vibrance` |
| `Wave` | wave | Wave | `filter.distort.wave` |
| `ZgZg` | zigZag | ZigZag | `filter.distort.zigZag` |
| (string ID only) | reverse | Reverse | `layer.arrange.reverse` |
| (string ID only) | linkSelectedLayers | Link Layers (toggles, as measured) | `layer.linkLayers` |
| (string ID only) | unlinkSelectedLayers | Unlink Layers | `layer.linkLayers {"linked":false}` |
| (string ID only) | selectLinkedLayers | Select Linked Layers | `layer.selectLinkedLayers` |
| (string ID only) | disableLayerStyle | Clear Layer Style | `layer.layerStyle.clear` |
| (string ID only) | placedLayerMakeCopy | New Smart Object via Copy | `layer.smartObjects.newSmartObjectViaCopy` |
| (string ID only) | placedLayerConvertToLayers | Convert to Layers | `layer.smartObjects.convertToLayers` |
| (string ID only) | placedLayerConvertToEmbedded | Embed Linked | `layer.smartObjects.convertToEmbedded` |
| (string ID only) | placedLayerUpdateAllModified | Update All Modified Content | `layer.smartObjects.updateAllModifiedContent` |
| (string ID only) | rasterizeAll | Rasterize All Layers | `layer.rasterize.allLayers` |
| (string ID only) | addBlankVideoLayer | New Blank Video Layer | `layer.videoLayers.newBlankVideoLayer` |
| (string ID only) | isolateLayers | Isolate Layers | `select.isolateLayers` |
| (string ID only) | textLoadDefaultTypeStyles | Load Default Type Styles | `type.loadDefaultTypeStyles` |
| (string ID only) | textSaveDefaultTypeStyles | Save Default Type Styles | `type.saveDefaultTypeStyles` |
| (string ID only) | recordMeasurements | Record Measurements | `image.analysis.recordMeasurements` |
| (string ID only) | CropPhotosAuto0001 | Crop and Straighten Photos | `file.automate.cropAndStraightenPhotos` |
| (string ID only) | placedLayerExportContents | Export Contents | `layer.smartObjects.exportContents` |
| (string ID only) | placedLayerConvertToLinked | Convert to Linked | `layer.smartObjects.convertToLinked` |
| (string ID only) | placedLayerRelinkToFile | Relink to File | `layer.smartObjects.relinkToFile` |
| (string ID only) | placedLayerReplaceContents | Replace Contents | `layer.smartObjects.replaceContents` |
| (string ID only) | shapeBlur | Shape Blur (radius-only form) | `filter.blur.shapeBlur` |
| (string ID only) | artboardFromLayerGroupEvent | Artboard from Group | `layer.new.artboardFromGroup` |
| (string ID only) | editArtboardEvent | Artboard background color (flags 8/1) | `layer.artboard.set` |

### Newer event mapping boundaries

The 23 requested forms in the two **Measured** tables under
[Newer events](#newer-events-string-id-only) are mapped. Artboard from Group
retains the current-layer target, preset tag, fractional RGB, background type
and integer `changeBackground`, including absence. A flag of 1 or absence applies
the background; other integers give white. An omitted type defaults to white.
Recorded guide IDs are preserved on save and ignored on playback, as measured
in [the guide/background reference below](#newer-events-string-id-only).
Existing document guides remain unchanged. The preset does not resize the group.

The additional `editArtboardEvent` maps background-only changes with `changeSizes`
8 and `changeBackground` 1. Its rectangle and preset remain recording metadata:
they do not move or resize the target artboard. Colors remain in the saved step
even for white, black or transparent backgrounds. Other flags, extra settings,
unknown targets and forms that cannot reconstruct exactly remain whole kept
steps. Native geometry edits without a matching wire form refuse export.

The sixteen target-only events accept an empty descriptor or the current-layer
`null` reference (`Lyr `/`Ordn.Trgt`). Save retains which form was imported.
Unknown targets or extra keys stay complete kept steps. Unrepresented native
options are refused on export. Playback uses the native command's preconditions
and current state. Isolate Layers toggles the panel view; type defaults remain
session settings; Record Measurements uses the current measurement configuration.

Link Layers plays as the toggle the reference application performs (played twice on the same
layers it links, then unlinks them; see the measurements below). Unlink is explicit and
idempotent: it keeps an already unlinked selection unlinked and clears singleton link groups.
Undo restores an edit. Explicit native layer lists cannot be exported as these targeted events.

The four Smart Object file events use Open/Place's `Pth `/`txtu` path encoding.
Export Contents uses `null` for its path; Convert to Linked uses the current-layer
`null` reference and `Usng` for its path; Relink and Replace use `null` for the path.
A positive integer `LyrI` is a recording-session hint, never a PhotoCraft layer
identity, and is omitted on save under the normalization rule below. Undecodable
paths or extra keys keep the entire step. File access happens at playback; a
missing or unusable file reports the native command's error.

Convert Type to Shape retains Make's `contentLayer` class reference and `Usng`
target `textLayer` reference; it does not export a separate `convertToShape`
event. Other Make mappings, including Solid Color and Pattern fills, are unchanged.
Shape Blur maps only the measured radius form (`Rds ` pixels, 5–1000; omitted
radius uses the native factory value 10). Later dialog forms carrying
`customShape` or `keyOriginType` remain kept. Native explicit shape choices cannot
be saved as radius-only descriptors; no custom asset lookup or new kernel is added.

Crop and Straighten Photos executes the native scan operation once, creating one
document per photo found inside the selection and leaving the source unchanged.
It requires a selected, visible layer with no pixel or full lock, including
inherited group state, and remains unavailable for Indexed Color documents.
The separate `CropPhotos0001` helper remains a complete kept Photoshop step.
Playback stops at that enabled helper before subsequent duplicate/crop steps can
run. This covers the parent event, not complete recordings with generated helpers.

### Content-Aware Fill recordings

`cafWorkspace` maps Rectangular / Default color adaptation / None rotation with New Layer or
Current Layer output to `edit.contentAwareFill`, retaining all seven keys and the three booleans:
Sample All Layers, Scale and Mirror. The selection comes from the preceding selection step.
New Layer is in the measured reference; Current Layer and the enumeration type IDs are explicit
in [Stephen Marsh's published Action Manager example](https://community.adobe.com/questions-712/expand-and-content-aware-fill-functions-are-not-working-via-javascript-1172085).

| Recorded key, in order | Supported value | Native parameter |
|---|---|---|
| `cafSamplingRegion` | `cafSamplingRegionRectangular` | `sampling: "rectangular"` |
| `cafSampleAllLayers` | boolean | `sampleAllLayers` |
| `cafColorAdaptationLevel` | `cafColorAdaptationDefault` | `colorAdaptation: "default"` |
| `cafRotationAmount` | `cafRotationAmountNone` | `rotationAdaptation: "none"` |
| `cafScale` | boolean | `scale` |
| `cafMirror` | boolean | `mirror` |
| `cafOutput` | `cafOutputToNewLayer`, `cafOutputToCurrentLayer` | `output: "new"`, `"current"` |

Descriptor types, source identity and key order survive save while its native settings are
unchanged. Editing a supported setting exports the new values in canonical form instead of
restoring stale source values; malformed preservation data still refuses export.
Native export writes the seven keys explicitly; the known native boolean, color
adaptation, rotation and Current Layer output defaults are supported when sampling is explicitly
Rectangular. A native call using the Auto sampling default cannot yet be exported.

The reference's ellipses do not establish additional enum forms. Auto/Custom sampling,
other adaptation levels, Duplicate Layer output, absent keys, unknown settings and
malformed or repeated fields remain complete kept steps. Native layer targets, sampling/fill
edits and nondefault seeds also refuse export rather than lose their settings. Playback uses
the existing native fill and its selection, authorization, job and Undo/Redo behavior. The
remaining CAF forms and the other recorded tools in #10710 are still separate work.

### Layer Style menu recordings

`scaleEffectsEvent` maps to `layer.layerStyle.scaleEffects` with the recorded `Scl `
whole percentage in `#Prc`, from 1 through 1000. A missing recorded value is not
given a guessed default. Native export makes its effective default of 100 explicit;
fractional, out-of-range and unrepresented options refuse export.

Create Layer(s) maps Make (`Mk  ` / `make`) to `layer.layerStyle.createLayer` when
`null` is the `Lyr ` class reference and `Usng` is the `Lefx` property of the target
layer (`Prpr.Lefx`, `Lyr .Ordn.Trgt`). It has no recorded operation settings.
The existing native command creates the effect layers. The warning dialog is not
an extra action parameter. Explicit native layer IDs have no measured export form.
Malformed references stay complete kept steps, rather than creating a blank layer;
ordinary layer creation still uses its layer-options object or class-only form.

Both mappings retain descriptor identity, order and step flags while unchanged.
A supported scale edit exports its new value canonically; damaged preservation
metadata remains refused. These two rows are the implemented subset of #10723;
the other menu recordings remain separate work. Playback reuses native history
and rendering; this does not claim Photoshop pixel equivalence.

### Type Create Work Path recordings

Make (`Mk  ` / `make`) maps to `type.createWorkPath` when `null` is the `Path`
class reference and `From` is the target text layer (`TxLr.Ordn.Trgt`). Playback
uses the existing native glyph-outline command and leaves the text layer editable.
Undo restores the previous work path; Redo restores the new one.

The mapping preserves the complete descriptor, including key order and reference
names. Unknown or duplicate keys, named sources, other source classes and other
reference chains remain complete kept Photoshop steps. Native export supports
the implicit active text layer only; explicit layer targets refuse export.
This is another implemented subset of #10723, not support for its remaining events.

### Settings retained by mapped events

- Transform keeps its recorded `FTcs` reference point. Show with `TglO` true maps to
  `layer.showOnly`; repeating it restores the other layers' visibility.
- Make keeps top-level group names, new-layer clipping (`Usng.Grup`) and adjustment colour labels
  (`Usng.Clr `). Layer via Copy / Cut keeps names, blend modes, opacity and colour labels.
- Loading another layer's transparency keeps its source reference. Background property references
  remain independent of translated names; ordinary layer-name references remain names.
- Close without `Svng` preserves the unanswered save choice. Headless playback refuses to discard a
  changed document. Native `file.close` still defaults to discard; `save: "ask"` opts into this policy.
- Photo Filter converts recorded Lab colours through `photocraft-cms`.
- Direct Vibrance and Gradient Map save as the string events `vibrance` and `gradientMapEvent`.
  Stroke Outside uses `StrL.Otsd`, group Pass Through uses `BlnM.passThrough`, Canvas Size extension
  colours use `FrgC` and `Clr `, and Absolute Colorimetric intent uses `Inte.AClr`.
- Make adjustment layers keeps surrounding `Usng.Opct` as layer opacity, independently of the
  adjustment settings. Recorded percentages use eight-bit layer-opacity storage at every document
  depth: 70 % becomes 179/255. Native creation accepts an optional 0..1 `opacity` fraction.
  A default adjustment with only layer opacity still saves as a default preset.
- Document offsets wrap in opening order, independently of dragged tabs. An ATN zero offset remains
  a kept step; the native command retains its existing zero-offset no-op.
- Convert Mode saves `T   ` as a class. Layer-retaining RGB, grayscale, CMYK and Lab conversions
  read and write explicit false `Fltt` and `Rstr` flags. True or malformed flags remain kept steps;
  depth, bitmap and multichannel recordings with these flags also remain kept.
- Canvas Size preserves percentage units and resolves them against the playback document.
  Percentages are absolute sizes; mixed units and relative percentages remain kept steps.
- Image Size preserves points (`#Rlt`) and resolves them using the playback document's resolution.
  Mixed units and unsupported physical-size/resolution combinations remain kept steps.
  Without resampling, native Image Size still changes only resolution.

Shape creation (`contentLayer`) with geometry or stroke settings remains a kept step. Save preserves
its descriptor, and enabled playback stops instead of creating a canvas-wide solid fill. Editable
shape creation is not mapped yet.

The Vibrance event and adjustment-opacity handling incorporate [@bommerts' contribution in #8659](https://github.com/storytold/photocraft/pull/8659).

### Command options retained by mapped events

- Image Size preserves `scaleStyles`. Native calls default to true; an absent ATN flag means
  false (the reference application plays both alike), so Save writes the key only when it is true.
  Crop and resolution-only resizing retain their existing behavior.
- Convert to Profile preserves `Dthr`, `Fltt` and `rasterizePlaced`, defaulting to false.
  `sdwM` is written as the reference application records it (-1); other values and malformed
  flags remain kept steps. Absolute intent writes `AClr`, with `AbsC` still accepted on import.
- Content-Aware Fill preserves colour, quarter-turn rotation, scale and mirror adaptation.
  Existing native defaults remain unchanged.
- Paste Layer Style preserves `allowPasteFXOnLayerSet`. False skips selected groups without
  applying their styles to children; a selection with no eligible targets fails without editing.
- Paste preserves `As ` path or shape using the original bounded SVG source. Unsupported
  geometry fails instead of silently rasterizing. `As Pxel` plays as an ordinary Paste (or Paste
  in Place), as does omission.

Generated fixtures verify these native options and their load/save/playback behavior. Conversion order,
SVG rasterization and placement were not compared with the reference application.

### Render filters

Native commands without an explicit Lens Flare center still use 0.5 / 0.5. Imported steps materialize the measured center above. A missing Fibers seed is captured once at import and preserved for later playback and export. Native color overrides and other settings without ATN fields are refused during strict export.

## Recognized steps kept disabled

These recognized events have no playable command. They load as kept steps switched
off, and Save Actions preserves their descriptors. They are excluded from the
supported-event count and the measured unmapped reference.

| String ID | Label |
|---|---|
| animationFramesFromLayers | Make Frames from Layers |

### Supported direct adjustment settings

#### Gradient Map (direct)

- Event: `GrMp` (no string ID)
- PhotoCraft command: `image.adjustments.gradientMap`
- Supported: classic (`Gcls`, also when omitted), 2..64 ordered custom RGB stops at midpoint 50, smoothness 4096, and the two opaque endpoint transparency stops. Fractional RGB components retain their precision and the recorded name and event survive save.
- Other interpolation methods, midpoints, transparency, unsorted stops and unsupported descriptor forms remain kept steps. This does not change the separate legacy `GdMp` event mapping.

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

#### Replace Color

- Event: `RplC` / `replaceColor`
- PhotoCraft command: `image.adjustments.replaceColor`
- Supported: the explicit D50 Lab minimum/maximum box and `colorModel` 0, with the four integer settings clamped to the ranges below. Matching uses original pixel channels through the document profile. Unsupported color models, malformed or missing samples and unknown keys remain kept steps.
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


## Known gaps in mapped events

A mapped event can still lose a setting. The original 117-event mapping was checked on 2026-11-01: the 5 296 steps of
the `.atn` corpus (sets the reference application recorded) and factory descriptors measured in the
reference application went through `actions.load` and `actions.save`, and each saved descriptor was compared
with the original. Then every step PhotoCraft saves for the cases in
`every_mapped_command_exports_and_imports_back` was played in the reference application: 128 of 151 ran.
Three failed on a wrong code (Vibrance, Gradient Map, Stroke Outside); the others needed a document the test
did not have (a mask, a group, a named pattern or channel). Checking every code the mapping writes found
three more (Pass Through, canvas extension colour, Absolute Colorimetric). The steps below change what a
step does. Every one of them is fixed now: the sections "Settings retained by mapped events" and
"Command options retained by mapped events" above say how each setting is kept. What remains are
differences that do not change what a step does.

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

### Classic Gradient recordings

`Grdn` / `gradientClassEvent` maps to `paint.gradient` for custom RGB gradients
with byte-valued colours and centred colour/transparency midpoints. The five
measured styles are Linear, Radial, Angle, Reflected and Diamond. Pixel endpoints,
opacity, blend mode, reverse, dither and the transparency switch are explicit;
an omitted `Dthr` is off, rather than the native command's on default. The
recorded interpolation method maps to the existing Classic, Perceptual, Linear
or Smooth implementation, and `Intr` supplies smoothness.

Unchanged recordings retain their complete descriptor bytes, key order, ID
encoding and explicit defaults. A supported edit writes the recorded key order:
`From`, `T   `, non-default `Opct` and `Md  `, `Type`, non-default `Rvrs`, enabled
`Dthr` and `UsMs`, `gradientsInterpolationMethod`, then required `Grad`. Source
data is never replayed in place of edited executable settings. Native export
requires explicit stops, transparency (an empty list means opaque) and method;
it does not resolve a named or current gradient preset at save time.

Unknown/duplicate keys, absent interpolation methods, non-centred midpoints,
fractional RGB colours, noise gradients and dynamic foreground/background stops
remain kept steps. The current preset model does not represent those forms
faithfully. Invalid units/enums or missing `Grad` also stay kept, rather than
using a different gradient. The default Make/Set gradient fill-layer recordings
are separate work. Synthetic fixtures follow the measured reference below;
no new Photoshop recording or pixel-equivalence measurement is claimed.

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

The original survey covered menu events that were neither mapped nor in the reference below:
63 candidates from PhotoCraft's menu catalogue, each run in the reference
application without a dialog on a document prepared for it (layers, a selection, a smart object, a type
layer, a layer style or a group) on 2026-11-01. 27 more menu items record no event of their own (puppet
warp and warp splits, combine shapes, scripts such as Load Files into Stack and Statistics, artboard and
video export, missing-font commands): they cannot be recorded or play as a script step.

The two Measured tables remain the recording reference. The mappings and remaining
limits added since that survey are listed under [Newer event mapping boundaries](#newer-event-mapping-boundaries).

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
| `artboardFromLayerGroupEvent` | Layer > New > Artboard from Group | `null` target; `artboard` (object: `guideIDs` list, `artboardPresetName` text, `Clr ` RGB background colour, `artboardBackgroundType` long: 1 white, 2 black, 3 transparent, 4 other = `Clr `); `changeBackground` long. See *Artboard from Group: guides and background* below. |
| `shapeBlur` | Filter > Blur > Shape Blur | `Rds ` radius in pixels (factory 10); the shape itself comes from the dialog |
| Make `contentLayer` with `Usng` = reference to `textLayer` `Ordn.Trgt` | Type > Convert to Shape (recorded as Make, not as `convertToShape`) | `null`: class `contentLayer`; `Usng`: the type layer |

**Recorded through the menus** (2026-11-01, steps read back from the saved Actions panel):

| Menu | Recorded step | Keys |
|---|---|---|
| Layer > New > Frame from Layers | Make, `null` = class `framedGroupSection`, `From` = layer **by name**, `layerSectionStart`, `layerSectionEnd` long, `Nm  ` text | the dialog also shows width and height, which are not recorded |
| Layer > New > Artboard from Layers | Make, `null` = class `artboardSection`, `From` = layer target, `layerSectionStart`, `layerSectionEnd`, `Nm  ` | |
| Layer > New > Artboard | Make, `null` = class `artboardSection`, `layerSectionStart`, `layerSectionEnd`, `Nm  `, `artboardRect` (`Top `, `Left`, `Btom`, `Rght` doubles, pixels) | |
| Properties panel > Artboard Background Color (Artboard selected) | `editArtboardEvent` | `null` target; `artboard` (object: `artboardRect` `Top `/`Left`/`Btom`/`Rght` pixels, `guideIDs` list, `artboardPresetName` text, `Clr `, `artboardBackgroundType` 1..4 as above); `changeSizes` long (8 when only the background changed), `changeBackground` 1. `Clr ` is only meaningful with type 4: with types 1..3 it holds the previously used colour |
| Layer > Smart Objects > Stack Mode > mode | `applyImageStackPluginRenderer`: `imageStackPlugin` **class** with the mode's code, `Nm  ` text = the menu name (localised, except "Entropy") | codes: Range `rang`, Entropy `entr`, Kurtosis `kurt`, Maximum `maxx`, Median `medn`, Minimum `minn`, Mean `avrg`, Skewness `skew`, Standard Deviation `stdv`, Summation `summ`, Variance `vari` |
| Layer > Smart Objects > Stack Mode > None | `imageStackConvertSmartObject` | none |
| Layer > Video Layers > New Blank Video Layer, Insert Blank Frame, Delete Frame, Show / Hide Altered Video | `addBlankVideoLayer`, `insertBlankFrame`, `deleteFrame`, `showAlteredVideo` / `hideAlteredVideo` | none |
| Layer > Video Layers > Interpret Footage | `interpretFootage` | `Vrsn` long (1), `interpretAlpha` (`alphaInterpretation`: `Ignr` ignore, …), `colorConvertAlterations` bool |
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

**Artboard from Group: guides and background** (measured 2026-11-02, script runs and a recording through the menu):

- **Recorded form:** the menu item opens no dialog. The step always has `guideIDs` empty, `artboardPresetName` "", `changeBackground` 1, and the
  background type and colour **last used** for an artboard (the Properties panel choice is sticky: after picking Black for one artboard, the
  next Artboard from Group records type 2 and makes a black artboard). Guides in the document do not change the recorded step.
- **`guideIDs`:** session metadata, not a playback input. Guide ids are a counter for the whole application session (a new document's
  guides got 5, 6, 7 after an earlier one's 2, 3, 4). Playing the step with the document's own guide ids, an unknown id (99999) or an
  empty list gives the same result: no error, the guides stay document guides (`kind` document), and the new artboard's `guideIDs` is empty.
  An implementation can ignore the list on playback and write `[]`.
- **`changeBackground`:** 1 or absent applies `artboardBackgroundType` and `Clr `. Any other value (0, 2, 3, -1, 7 tried) ignores both and
  makes a white artboard (type 1, `Clr ` 255/255/255), and the step's result then has no `changeBackground` key.
  `artboardBackgroundType` absent with `changeBackground` 1 gives type 1 (white).
- The artboard takes the group's bounds; `Clr ` is stored on the artboard as given even when the type is 1..3.

**Measured for the mapped events** (2026-11-02, script runs in the reference application):

- **Transform reference point** (`FTcs`, type `QCSt`): `Qcs0` top-left, `Qcs1` top-right, `Qcs2` bottom-right, `Qcs3` bottom-left
  corner; `Qcs4` top, `Qcs5` right, `Qcs6` bottom, `Qcs7` left edge centre; `Qcsa` the centre. `Qcs8` and unknown values also scale
  about the centre (not recorded).
- **Convert to Profile** records `sdwM` = -1 (with `Inte`, `Dthr`, `Fltt`, `rasterizePlaced`).
- **Plain Edit > Paste** records `AntA` = `Annt.Anno` and `As  ` = class `Pxel`.
- **Link Layers** (`linkSelectedLayers`) toggles when played: played twice on the same two layers it links, then unlinks them.
- **Split Channels** splits a document with a single layer, Background or not (a lone layer gives one more document); a document
  with more than one layer is refused.
- **Load Actions** with a set whose name is already in the panel adds a second set of that name; nothing is replaced.

- PhotoCraft preserves fractional recorded RGB values exactly. Native actions with partial settings export an empty preset and guide list when absent.
  An absent native color exports as white: it matches the native custom-color fallback and is visually ignored for white, black and transparent backgrounds.
  A custom background without a color uses the White variant (type 1); an explicit custom white color keeps type 4.
  This is canonical export metadata, not a claim about the reference application's remembered color. Imported steps retain their recorded color instead.

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
| Gradient tool, default mode | two steps: Make (`null` = class `contentLayer`, `Usng` = `Type` gradient layer object) then Set (`null` = `contentLayer` target, `T   ` = the same object plus `Algn` false) | The object: `Dthr`, `gradientsInterpolationMethod`, `Angl` angle, `Type`, `Scl ` percent, `Ofst` (`Hrzn`, `Vrtc` percent), `Grad`. The new tool makes a gradient fill layer; Make holds the first click, Set the finished drag. |
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

Batch cancellation conformance (Photoshop 2026 observation above; PhotoCraft's producer is
`file_cmds::batch_run::process_batch_outputs`, and its desktop consumer is `jobs_ui::tick`):

| Reference step | Acceptance gate | Status |
|---|---|---|
| Plan folder order, names and serials once | Continue retains the original plan and next serial | Implemented and tested; Esc between inputs does not skip the unopened input |
| Show the opened input while its action runs | A read-only document preview appears in the canvas and closes with the input | Implemented and visually checked; deliberate deviation: worker snapshots are not live tabs or Layers panel targets, so they cannot be edited or saved by mistake |
| Interrupt the current command on Esc | The current input reaches a checkpoint before any output write | Implemented and tested before save; once the crash-safe atomic write begins, Esc is refused because publication cannot be retracted |
| Ask Continue or Stop | Continue skips the interrupted input; Stop returns prior outputs and skips the rest | Implemented and tested in engine and UI |
| Keep headless cancellation direct | `jobs.cancel` returns partial results without an interactive question | Implemented and tested, including a worker waiting for a desktop decision |
| No progress bar | The desktop Batch preview is the visible feedback | Implemented, UI tested and visually checked |
| Preserve error policy and log | Continue retains earlier errors and writes one final UTF-16 LE log | Implemented and tested with errors before and after Esc |

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

## Event reference: measured specs

Each entry below is an event the reference application records, as measured there: its 4-character code
and string ID, the PhotoCraft command it maps to (or should map to), and every key the step carries, in the
order it is recorded, with type, unit, factory default and allowed values. The Supported events table says
which of them PhotoCraft maps today; for the others an entry has everything needed to implement them. When
an event is implemented, add it to that table in the same change.
Current support and limits of the newer events are described under [Newer event mapping boundaries](#newer-event-mapping-boundaries).

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

The seven Distort events below map to PhotoCraft's `filter.distort.*` commands for their documented supported forms, including complete Shear curves. Displace needs a displacement map file and is listed separately at the end.

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

Supported for complete curves: import clamps finite grid coordinates to the measured ranges below,
maps them to the native top-to-bottom position `(Vrtc - 1) / 127` and half-width offset `Hrzn / 64`,
and preserves fractional recorded coordinates exactly on save. Both edge modes are supported.
Only `ShrS` 0 and `ShrE` equal to the last point's index map; an omitted `ShrE` still defaults to 1.
The adapter accepts 2..4096 points as an execution safety budget, not a measured Photoshop format
maximum. This bounds point-vector allocation and the renderer's linear per-row curve search while
allowing fractional scripted points. Larger recorded curves stay intact as kept steps, and native
exports above that budget are refused before point conversion. No renderer performance is claimed.
The budget covers adapter mapping and playback; input parsing and generic export copies remain outside it.
Malformed settings, unknown keys, nonfinite values, and curves whose order or points the native
renderer would change remain complete kept steps. Export refuses unrepresentable native options
and mask targets. The native Shear renderer and its ordinary command behavior are unchanged.

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

All commands exist in PhotoCraft. The two direct adjustments, classic Gradient Map and Lab-range Replace Color, are supported above; the remaining events below still need their `.atn` mappings. These direct adjustments are separate from adjustment layers.

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
| `Scl ` | scale | integer |  | 1 | 1 .. 9999 (measured in the dialog); applied also when played without the dialog (scale 2 halves the result) |
| `Ofst` | offset | integer |  | 0 | -9999 .. 9999 (measured in the dialog); applied also when played without the dialog (offset 50 adds 50) |
| `Mtrx` | matrix | list |  | none: a step always carries all 25 cells | exactly 25 integers (5 x 5, row by row), each -999 .. 999; a 3 x 3 list makes the application fail |

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
