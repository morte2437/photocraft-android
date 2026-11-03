# CPU canvas pixel alignment (#8321)

Original 200 × 200 pixel crops at screen coordinates (200, 250)–(400, 450) from 900 × 600 offscreen renders at display scale 1. The scene is a white 1024 × 768 RGB document with 60% monochromatic Add Noise, viewed at Actual Pixels through the CPU canvas (`--safe-gpu`). No external artwork is used.

`before.png` shows the half-pixel horizontal offset. `after.png` shows the aligned canvas and is byte-for-byte equal to the GPU canvas crop. Their grayscale standard deviations are 17.4105 and 24.6821 respectively. The 901-pixel-wide control stays sharp.

The source document fits the offscreen WARP adapter texture limit, so reduced-texture sampling does not mask the pixel-alignment comparison. These captures are original PhotoCraft assets, licensed MIT OR Apache-2.0 (the license files are alongside them).

Default Crop keeps its continuous camera positioning so the crop frame stays fixed during fractional pointer drags. Normal canvas views and Classic Crop use pixel alignment at integral physical magnifications.
