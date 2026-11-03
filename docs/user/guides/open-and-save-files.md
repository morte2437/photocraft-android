---
title: Open and save files
description: Open a PSD or other image in PhotoCraft, and save it with its layers or as a flat copy.
order: 10
verified: 0.8.2
---

Open a file with **File › Open…** and keep your layers with **File › Save As…**, choosing **PSD Document** or **PhotoCraft** as the type. Formats such as PNG and JPEG are flat: they hold one image with no layers.

## Open a file

1. Choose **File › Open…**.
2. Pick your file. The type menu in the dialog offers **All Formats**, **PhotoCraft**, **OpenRaster**, **Krita**, **SVG** and **Paint.NET**.
3. Confirm in the dialog.

You can also reopen a recent file from **File › Open Recent**. On Linux under Wayland, dropping a file on the window does not work yet; see [Install PhotoCraft](../getting-started/install.md).

PhotoCraft opens PSD and PSB (including 16-bit, 32-bit, CMYK and Lab files), layered TIFF, Paint.NET (`.pdn`), OpenRaster (`.ora`), SVG, and the flat formats PNG, JPEG, TIFF, WebP, GIF, BMP, TGA, ICO, QOI, PNM, PCX, OpenEXR, Radiance HDR and FITS. Krita files (`.kra`) open as one image layer made from the picture Krita saved, without Krita's layers. HEIC and HEIF photos from iPhone and Mac open too, read only, and so do camera raw files such as DNG, CR2, CR3, NEF and ARW. It can also open Affinity files (`.af`, `.afphoto`, `.afdesign`, `.afpub`), read only; PhotoCraft cannot save Affinity files. Anything it cannot match exactly, such as layer effects and adjustments, is approximated or left out and PhotoCraft shows a warning.

## Save with layers

1. Choose **File › Save As…**.
2. Pick **PSD Document** (`.psd` or `.psb`) to share the file with Photoshop, or **PhotoCraft** (`.pcraft`) for PhotoCraft's own format.
3. Name the file and save.

After that, **File › Save** saves into the same file. **File › Save a Copy…** saves a copy under a new name. While a large file saves, the status bar and the tab title show "Saving" with a percentage.

The **Save As…** dialog also lists OpenRaster, PNG, JPEG, WebP, TIFF, Targa, OpenEXR and FITS. **File › Save** on an opened JPEG keeps its quality, and on a WebP keeps it lossy or lossless. Check the file extension in the name box: PhotoCraft writes the format that the extension names. If you want a flat PNG or JPEG with a size and quality preview, use [Export an image](export-an-image.md) instead.

## Good to know

Saving a PSD again keeps the layers and the rendered result, but the file is not byte for byte the same as the one you opened. PhotoCraft rewrites image resources, layer records and the composite. It keeps parts it does not model yet instead of dropping them.
