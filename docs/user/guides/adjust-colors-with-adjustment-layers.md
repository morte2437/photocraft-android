---
title: Adjust colors without changing your pixels
description: Add a Curves, Levels or other adjustment layer in PhotoCraft so you can change or switch it off later.
order: 20
verified: 0.8.2
---

Use **Layer › New Adjustment Layer** and pick an adjustment. The change sits on its own layer above your image, so your original pixels stay as they were and you can edit or hide the adjustment later.

## Add an adjustment layer

1. Open your image and click the layer the adjustment should sit above in the **Layers** panel.
2. Choose **Layer › New Adjustment Layer**, then an adjustment such as **Curves…**, **Levels…**, **Vibrance…** or **Hue/Saturation…**.
3. Change the settings in the **Properties** panel, which shows the settings of the new layer.

The same list is behind the button at the bottom of the **Layers** panel, labelled **Create new fill or adjustment layer** when you hover over it.

The adjustments in the menu are **Brightness/Contrast**, **Levels**, **Curves**, **Exposure**, **Vibrance**, **Hue/Saturation**, **Color Balance**, **Black & White**, **Photo Filter**, **Channel Mixer**, **Color Lookup**, **Invert**, **Posterize**, **Threshold**, **Gradient Map** and **Selective Color**.

## Limit it to one area

1. Select the area first, for example with the **Elliptical Marquee Tool**. See [Select part of an image and mask it](select-and-mask.md).
2. Add the adjustment layer as above.
3. With the adjustment layer active, choose **Layer › Layer Mask › Reveal Selection**. The adjustment now shows only inside the selection.

## Change it later

Click the adjustment layer in the **Layers** panel and edit its settings in **Properties**. Every step you take is also listed in the **History** panel.

## Apply an adjustment straight to the pixels

If you do not need to change it later, use **Image › Adjustments**, for example **Image › Adjustments › Curves…**. These change the pixels of the layer directly and are not kept as a separate layer.
