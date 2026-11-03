# Sony ARW metadata evidence

2026-10-11: PhotoCraft now reads applied white balance and black levels from Sony's encrypted
SR2SubIFD when usable plain tags are absent. It also applies the measured ILCE-6000 image area.
This corrects source metadata; it does not add a camera colour profile or a JPEG tone match.

## Research and scope

PhotoCraft already contains the cRAW sensor decoder from [#83](https://github.com/storytold/photocraft/pull/83)
and the 512 black fallback from [#1902](https://github.com/storytold/photocraft/pull/1902).
The open [#2135](https://github.com/storytold/photocraft/pull/2135) proposes fitting camera-JPEG
colour and tone; [#2230](https://github.com/storytold/photocraft/pull/2230) adds camera profiles.
Their actual bodies and diffs were reviewed before this change; neither is merged at the
inspection date. This change addresses metadata missing from the existing decoder, without
duplicating those colour-development proposals.

LightCraft's [#115](https://github.com/storytold/lightcraft/pull/115),
[#535](https://github.com/storytold/lightcraft/issues/535) and its current camera-preview-colour
documentation distinguish applied WB, camera calibration and JPEG tone/DRO. Its SR2 investigation
suggested testing encrypted metadata before estimating WB from scene pixels. PhotoCraft's
implementation is independent: no sibling source, decoder dependency or camera colour table is
included. The first keystream words were recovered again from local metadata observations.

## Measurement

Three read-only local ILCE-6000 ARW/JPEG pairs were inspected with ExifTool 13.59 and PhotoCraft's
`rawinfo` example. No private files, scene pixels or metadata dumps are committed.

| Source fact | All three ARWs |
|---|---|
| Sensor / output image | 6048 × 4024 / 6000 × 4000 |
| cRAW code range after SonyToneCurve | 14-bit; white 16383 |
| SR2SubIFD BlackLevel (`0x7310`) | 512, 512, 512, 512 |
| Applied WB_RGGBLevels (`0x7313`) | 2764, 1024, 1024, 1784 |
| Applied R/G/B multipliers | 2.69921875, 1, 1.7421875 |
| Image area observation | [12, 12, 6012, 4012], corroborated by EXIF dimensions and JPEG framing |

Previously WB was estimated independently from each scene, yielding R/G/B gains of roughly
2.920/1/2.214, 2.661/1/1.823 and 3.063/1/1.645. All three now use the camera's applied gains.
The measured crop is specific to ILCE-6000 at this sensor size; an explicit existing crop wins.
Other models and sensor sizes are unaffected by that entry.

The pinned public-domain DSC-RX0 corpus sample independently verifies black 800 in all four
positions, rather than the former 512 fallback, and WB 2056/1024, 1, 1908/1024.

## Container and safety

Tag meanings come from [ExifTool's public Sony tag documentation](https://exiftool.org/TagNames/Sony.html).
IFD0's `50740` BYTE/UNDEFINED[4] points to SR2Private, whose `0x7200`, `0x7201` and `0x7221`
provide the block offset, byte length and key bytes. Only the observed `11 22 33 44` key is supported.
XOR of ciphertext with black-box decrypted metadata yielded the same first 127 big-endian
keystream words for all three files. Every subsequent whole word in each 56958-byte block
obeyed `k[n] = k[n-63] XOR k[n-127]`; the incomplete final word is stored in the clear.

Only four SHORT/SSHORT values for `0x7310` and `0x7313` are read. Their absolute value offsets
must resolve inside the decrypted block. Allocation is capped at 1 MiB and `Limits.max_alloc`;
the whole directory and source bytes must exist. Negative or excessive levels, unusable gains,
other keys and malformed metadata are ignored, retaining the existing decode and fallback.
Usable plain tags take precedence; invalid plain WB does not conceal valid encrypted WB.
Auto-WB fields and unverified Tag2010 layouts are not substituted for applied WB.

Synthetic tests cover both byte orders, offsets beyond the first recurrence period, fixed
keystream observations, an incomplete final word, precedence, invalid levels and gains, every
truncation of a procedural fixture, out-of-block offsets, absurd sizes and allocation limits.
The public RAW corpus pins the RX0 metadata and checks deterministic 16-bit development.

## Rendering limits

The full sensor buffer still passes through black subtraction, applied WB, demosaicing and
16-bit ProPhoto-compatible development. The embedded JPEG remains a comparison image; it is
not substituted for RAW pixels. All three files import and export at 6000 × 4000 in 16-bit RGB.

Local comparisons with the paired camera JPEGs and installed LightCraft 0.6.0 show reduced
WB casts, but PhotoCraft remains flatter, darker and less saturated. These ARWs have no DNG
colour matrix, so the existing neutral camera-to-sRGB fallback and scene-referred tone remain.
Camera JPEGs also have DRO Auto and lens/tone processing; they are visual references, not
linear sensor oracles. The remaining camera colour and tone work belongs to G14 and the open
colour/profile proposals above. Three uncontrolled scenes do not establish a camera calibration.
