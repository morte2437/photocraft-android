> HISTORICAL 108-PR BUILD: this record describes the earlier source and artifact. The current branch has 21 requested PRs removed. See [trimmed integration](trimmed-integration-2026-10-29.md) for current scope and validation. Do not use the old portable artifact to validate these removals.

# PhotoCraft Windows portable test protocol

TEST BUILD — manual results are NOT RUN until a tester records them. This sheet is the entry point; the per-PR appendix remains the detailed regression specification.

## Frozen build record

Validated and packaged source: `f5b3f1238bb414b43851ebe86f3a58ce83bb9b48`. Base: `5f9b4391874aabc9deb2fcb905267613e4d93ec8`.
Version: `0.17.0-integration.20261029.f5b3f123`; Windows x64; pinned fonts `b076417b5880bd4154bd22f00695b1d28c6bea83`.
[Full-sweep CI](https://github.com/FredApps/photocraft/actions/runs/40858867520); [portable build](https://github.com/FredApps/photocraft/actions/runs/40867283787);
[download artifact](https://github.com/FredApps/photocraft/actions/runs/40867283787/artifacts/12895845995). Portable ZIP: `photocraft-0.17.0-integration.20261029.f5b3f123-windows-x64-portable.zip`.
SHA256: `4125eccd5b009f30dc2d72f64bfafcf32220dc5574f765d0b3664ac4d9f992ba`. Artifact ID `12895845995`, job ID `123425396034`.
The source SHA was checked against the run and artifact metadata. The ZIP was inspected for portable
content and x64 PE executables; this is not a claim that GUI/manual cases passed.
Release workflow 390299892 was temporarily enabled for the authorized dispatch and its original
`disabled_manually` state was restored while the accepted run continued. No stable publication or installation.
This test build is **unsigned** because the fork has no Windows signing secrets. The packaging job
executed the CLI version check and printed the expected version and `f5b3f1238` source prefix.

Download the verified ZIP, verify its SHA256, and extract it into a new owned test folder with the portable marker. Keep inputs and outputs in separate test folders. Do not replace an installed PhotoCraft or existing user preferences. Source/Cargo checks are in the main integration plan; they are not required to follow this portable GUI sheet.

Before launching from the test PowerShell session:

```powershell
$zip = '.\photocraft-0.17.0-integration.20261029.f5b3f123-windows-x64-portable.zip'
$expected = '4125eccd5b009f30dc2d72f64bfafcf32220dc5574f765d0b3664ac4d9f992ba'
if ((Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expected) {
    throw 'Portable ZIP hash mismatch; do not run it.'
}
# Use a new, empty test directory. The artifact bundle also contains MSI/symbols;
# select the portable ZIP named above, not the MSI.
Expand-Archive -LiteralPath $zip -DestinationPath '.\PhotoCraft-test-f5b3f123'
Test-Path '.\PhotoCraft-test-f5b3f123\portable.txt' # must be True
$env:PHOTOCRAFT_NO_PRINT = '1'
$env:LPDEST = 'photocraft-no-such-printer'
& '.\PhotoCraft-test-f5b3f123\photocraft.exe'
```

Use the default renderer unless executing the separately specified Windows Intel/DX12 case. For that case only, set `$env:WGPU_BACKEND = 'dx12'` in its dedicated session and record the actual adapter and proof its render assertions ran.

## P0 acceptance cases

Record a separate result for every depth/platform/reference variant. Stop acceptance on a crash, data loss, wrong-document edit or failed P0. A missing required fixture, font, OS or Photoshop installation is BLOCKED, not PASSED.

| ID | Fixture and action | Expected result | Status |
|---|---|---|---|
| W01 Native document and history | Create a 600×400 RGB document; add a transparent layer, paint, Undo/Redo, save `.pcraft`, close/reopen. Repeat supported 8/16/32f variants. | Live Undo/Redo restores pixels; reopening preserves dimensions, layers, masks and pixels. Persistence of undo history is not claimed. | NOT RUN |
| W02 Captured path ownership | In document A select a named saved path and start a Pen draft; switch to document B, which has a path of the same name. Repeat stale-source/closed-source cases from the appendix. | A valid draft commits only to its captured A target. B stays unchanged. Stale sources refuse safely with a diagnostic; failed commits are not retried each frame. | NOT RUN |
| W03 Adjustment content versus mask | Create an adjustment with a mask. Select its mask thumbnail, then click the adjustment name; open Properties. Repeat explicit mask-thumbnail targeting. | Name selects adjustment contents and correct brackets/Properties; explicit thumbnail selects mask; no accidental rasterization or content replacement. | NOT RUN |
| W04 Profile-aware export | Use the recorded tagged and untagged fixtures and working spaces from M02. Export sRGB with embedding on/off; compare preview, actual profile and pixels. | Conversion uses the captured effective source profile; tags match actual pixels; embedding off omits the profile; source pixels/profiles remain unchanged. | NOT RUN |
| W05 PSD exchange and rerender | Open the specified licensed Photoshop-authored layered fixture. Edit its supported type/mask/style/smart-filter properties to invalidate cached pixels; save native and PSD, reopen both, then reopen PSD in recorded Photoshop version. | Supported structure and editable properties survive; actual engine rerender meets its documented tolerance. PhotoCraft round-trip alone does not pass Photoshop acceptance. | NOT RUN |
| W06 Select and Mask and modal ownership | Open on a selection/mask; refine, Undo/Redo, Cancel, reopen and OK. Try background edit/menu routes while modal. Use the appendix for trusted native stale-target fixtures. | Cancel preserves document/history; OK produces one intended step; background edits refuse; stale targets never edit another layer/document. | NOT RUN |
| W07 Layer Style layout and Cancel | Execute the documented theme/scale/window matrix; edit independent Bevel highlight/shadow modes and gradient stops/methods; Cancel, then reopen and commit. | Controls/OK/Cancel remain reachable; dropdowns act independently; Cancel restores pixels/history; defaults and rich gradients preserve intended values. | NOT RUN |
| W08 Painting and brush load | Red/green canvas, blue foreground: Mixer Brush Alt-click each region, then stroke; test ordinary Brush sampling separately. Exercise Pencil/Smudge cases and the exact resize gesture from #4763. | Mixer sampling changes reservoir without painting or changing foreground; correct sampled stroke, Undo/Redo and gesture routing. Shift+Alt-right is not the resize gesture. | NOT RUN |
| W09 Polygonal Lasso | At high zoom, start a polygon and hover near each edge to scroll; place subsequent vertices, finish, then hover edges again. Repeat rotation/flip/aspect variants from the appendix; test Esc. | Vertices map through the panned target camera; one commit; no polygon-driven scrolling after finish; cancellation leaves no draft. | NOT RUN |
| W10 Crop Reset preserves off-canvas data | Move a filled layer partly outside the canvas; draw a small crop, Reset and Commit. Repeat Reset→edit→Undo→Commit. | Reset is a no-op on document/history and preserves hidden off-canvas pixels. A subsequently edited real crop still performs its intended operation. | NOT RUN |
| W11 Smart-filter rows and actions | On a two-filter smart object, right-click each name, reorder/delete, Undo/Redo; verify eye and double-click remain distinct. Replay a recorded named-layer action on a second document with different IDs. | Context actions affect the clicked filter, preserve selection identity and refuse stale menus. Eye/editor actions remain distinct; replay changes intended named targets only. | NOT RUN |
| W12 Lossless JPEG XL capability | Save a supported synthetic image as JXL; verify via the specified independent decoder and the appendix matrix. Check Open filters. | Supported samples decode losslessly; JXL is offered for export and remains absent from Open because import is not implemented. Unsupported 1×1 input refuses without crashing. | NOT RUN |

## Evidence and exit

For each case record: tested SHA/version, fixture hash/setup, variant, steps, Actual, result, screenshots/output/diff, tester/date, and defect link if applicable. Leave Actual/Evidence/Tester blank before execution. Record Photoshop version/platform, colour profile/depth and repository tolerance for each Photoshop oracle. An automated pass does not pass a manual subcase.

Exit requires applicable automated gates at the frozen source, all selected approved heads accounted for, no unresolved integration conflicts, P0 manual cases passed, and no crash/data-loss/wrong-target regression. List every remaining NOT RUN, BLOCKED and SKIPPED case explicitly. macOS droplets, Unix FIFO handling, Windows Intel/DX12 hardware, same-run PDB identity and Photoshop reopening need their specified environments; Windows portable testing does not pass those by implication.

Detailed plan: [integration gates and combined scenarios](approved-bugfix-parity-2026-10-29.md); [per-PR cases](per-pr-regressions.md).
