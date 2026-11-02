---
title: Can PhotoCraft open my Photoshop files?
description: PhotoCraft opens and saves layered PSD and PSB files and keeps what it does not understand yet.
order: 20
verified: 0.8.2
---

Yes. PhotoCraft opens, edits and saves layered PSD and PSB files, including 16-bit, 32-bit, CMYK and Lab documents.

When it saves the file again, the picture renders the same for 307 of the 309 files in the psd-tools test set the project checks against. Parts PhotoCraft does not model yet, such as raw blocks and descriptors, are carried over instead of being dropped. A re-saved file is not byte for byte the same as the original, because PhotoCraft rewrites image resources, layer records and the composite.

Text layers stay editable. In version 0.7.0 some of them moved when you started editing them; 0.8.2 fixes that, see [Text moves when I start editing a PSD text layer](../troubleshooting/text-moves-when-editing.md).

Results vary from file to file. If a PSD does not open or looks wrong, search the issue tracker or report it with the file.

PhotoCraft also opens layered TIFF, Paint.NET, OpenRaster, SVG, Krita and Affinity files (Krita and Affinity read only), and many flat formats. The full list is in [Open and save files](../guides/open-and-save-files.md).
