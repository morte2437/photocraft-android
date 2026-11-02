---
title: Canvas Size added a white border around my photo
description: Why Image > Canvas Size fills the new canvas with white around a photo, and how to extend the canvas with transparency instead.
order: 25
verified: 0.11.1
---

The photo did not change size. **Image** ▸ **Canvas Size…** only moves the layers; the new canvas area is filled with the **Canvas extension color**, which defaults to **Background** — your background color, usually white. Choose **Transparent** there to add empty canvas instead.

## Symptoms

- You open a photo, choose **Image** ▸ **Canvas Size…**, make the canvas larger and click **OK**.
- The photo's layer now has a white or background-colored border, so the layer looks resized or padded.
- The photo content itself keeps its size and position.

## Fix

1. Choose **Image** ▸ **Canvas Size…** again.
2. Set **Canvas extension color:** to **Transparent**.
3. Enter the canvas size you want and click **OK**.

The new area is now empty. If you also want to change the photo's size, use **Image** ▸ **Image Size…**; that scales the image pixels.

## Why this happens

An opened JPEG or another fully opaque image becomes a locked **Background** layer, as in Photoshop. A Background layer cannot contain transparent pixels, so when the canvas grows, PhotoCraft fills the new area with the canvas extension color. **Background** is the default, so the border takes your background color — white until you change it. Layers that are not the Background are only moved and keep transparency.

Canvas Size never scales layer pixels: it moves them by the **Anchor:** you choose. Scaling is **Image** ▸ **Image Size…**'s job.

## Still stuck?

If the photo itself is scaled or cropped, or the border appears with **Transparent** selected, report it in the [issue tracker](https://github.com/storytold/photocraft/issues) with your version from **Help** ▸ **System Info…** and a screenshot of the Canvas Size settings.
