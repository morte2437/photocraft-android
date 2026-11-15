# Security overview

PhotoCraft processes complex attacker-controlled files and exposes automation that can act with the user's filesystem permissions. Security is therefore part of parser, engine, I/O, automation, release, and dependency design.

## Current posture

| Area | Implemented protections | Known limitations / future work |
|---|---|---|
| PSD/PSB | Header, channel, decoded-byte, nesting, and pattern limits; malformed/property tests; fuzz targets | Broader hostile corpus, aggregate work budgets, embedded-object coverage |
| Raster codecs | Default dimension/pixel/allocation limits; decoder-specific budgets; malformed/property tests; fuzz targets | Continuous fuzzing and more format-specific semantic cases |
| `.pcraft` | Manifest/blob/total-byte limits; bounded decompression; CRC/hash and version checks; fuzz target | Tighter complexity/entry limits and directory-bundle path-race analysis |
| Engine commands | Error-return contract, focused bad-parameter tests, adversarial `panic_hunt` test | Continue eliminating input-derived panic and exhaustion paths |
| Automation filesystem | Separate read roots and one write root held as directory capabilities; traversal and symlink escapes refused; writes are atomic replacements | A hardlink inside a root still reads the inode it names outside the root; writing over it replaces only the link, leaving the outside name unchanged |
| Control TCP | Loopback binding; bearer-token authentication; rooted read/write authority; 1 MiB request lines; 8 MiB replies; 16 active connections; I/O timeouts | No general method capabilities, encryption, client identity, explicit JSON-depth policy, aggregate session-memory/command-duration budgets, or audit trail |
| MCP | Stdio; typed tools; authenticated and bounded bridge; rooted read/write authority; 256-step batches with aggregate reply budgets; headless preview/output ceilings | No general tool authorization scopes, aggregate session-memory accounting, or command cancellation; desktop screenshot capture is admission-bounded (4 outstanding requests, 128 MiB per image, 256 MiB aggregate, 8 s deadline) but that admission is far above the 8 MiB reply cap |
| Dependencies | Cargo lockfile and normal CI build/test/clippy | No `cargo-audit` or `cargo-deny` job/configuration confirmed in current tree |
| Releases | Checksums and platform signing/notarization paths when secrets are configured | Signing can be absent; no SBOM generation confirmed in current tree |

The table is a snapshot of the current repository, not a guarantee about deployed artifacts. Verify source and release evidence for high-risk use.

## Security priorities

1. Add capability-scoped automation sessions on top of the authenticated transport.
2. Maintain the implemented workspace-handle boundary as file-bearing commands evolve.
3. Extend the preview/reply budgets to JSON complexity, aggregate document/session memory,
   desktop import/export, command duration/cancellation, and the gap between screenshot
   admission and the reply ceiling.
4. Expand PSD/PSB, codec, and `.pcraft` fuzzing with regression corpora.
5. Add dependency advisories/license policy checks and release SBOMs.

Start with the [Threat model](threat-model.md), then use the focused pages for file handling, parsers, fuzzing, and automation.
