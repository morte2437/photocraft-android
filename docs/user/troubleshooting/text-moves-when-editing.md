---
title: Text moves when I start editing a PSD text layer
description: In PhotoCraft 0.7.0, a text layer from a Photoshop file can jump when you click it with the Type tool. PhotoCraft 0.8.2 fixes the known causes; update.
order: 20
verified: 0.8.2
---

Update PhotoCraft. In version 0.7.0, a text layer from a Photoshop file could jump or disappear when you started editing it. PhotoCraft 0.8.2 reads the text's styles and position the way Photoshop saved them, so the text stays where it was.

## Symptoms

- You open a PSD that was made in Photoshop and click one of its text layers with the **Horizontal Type Tool**. The text shifts, often to the left, as soon as editing starts.
- Centered text may show as left-aligned in the options bar.
- In some files, text that you click can disappear.

## Fix

1. Check your version with **Help › System Info…**. If it is older than 0.8.2, download the [latest release](https://github.com/storytold/photocraft/releases/latest) or update with [ArtCraft Launcher](https://getartcraft.com/docs/craft-launcher).
2. Open the PSD again and click the text layer with the **Horizontal Type Tool**.

If you already saved the file in 0.7.0 after the text moved, the moved text is now in the file. Go back to your original PSD if you still have it.

## Why this happens

Clicking a PSD text layer with the Type tool starts editing it, and PhotoCraft then lays the text out itself instead of showing the pixels Photoshop saved. PhotoCraft 0.7.0 read the alignment and size of each run of text only from Photoshop's default styles, and ignored part of the text position that recent Photoshop versions store. Both are fixed in 0.8.2 ([issue 3795](https://github.com/storytold/photocraft/issues/3795), [pull request 3819](https://github.com/storytold/photocraft/pull/3819)).

## Still stuck?

If text still moves in 0.8.2, keep a copy of the original with **File › Save a Copy…**, then report it in the [issue tracker](https://github.com/storytold/photocraft/issues) with the PSD (zipped) and the Photoshop version that saved it. You can also ask in [Discord](https://discord.gg/artcraft). See [Get help with a problem](get-help.md).
