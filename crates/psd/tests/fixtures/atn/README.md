# Synthetic ATN parser fixtures

These four tiny files were independently encoded field by field from the version-16 layout
documented in `src/atn.rs`, without calling PhotoCraft's writer. They are original test data,
licensed MIT OR Apache-2.0 under the repository's licenses. They contain no user artwork,
Photoshop program code or proprietary assets. They are not Photoshop-authored interoperability
oracles.

| File | Bytes | Purpose | SHA-256 |
|---|---:|---|---|
| `empty.atn` | 13 | Empty set and explicit big-endian version | `6fd936f91296a13b0e7456a38b139a100fd17407d1c582d4502ce76413d01539` |
| `metadata.atn` | 214 | UTF-16 surrogate pairs/unpaired units, action order, function key, modifiers, panel flags, disabled step, unknown string event, dialog options | `cec2ff5a25e003aa29e790ea6b110f06ba848f0afd7901f3f8ad4740f782ede2` |
| `descriptors.atn` | 859 | Every descriptor value variant and every reference form currently represented by the parser | `6c76502822c1f36c8060a71adb227c4540663518785bbab1b70951dac0ef58ad` |
| `float-bits.atn` | 428 | NaN payload, negative zero, infinities, and floats inside lists, objects and arrays | `0aa6278a569c777f249b305e889896132c2a87e44a24cb1a55d1d3fc0928fd0c` |

`tests/it/atn_contract.rs` checks the independently encoded bytes and expected public structures.
A fixture change should explain
the changed wire fields and update this table; do not regenerate expectations merely to make a
failing parser pass.
