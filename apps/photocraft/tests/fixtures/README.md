These files are **schema probes**, not recorded machine saves.

`ui-0.14.ron`, `ui-0.15.ron` and `ui-0.37.ron` are minimal payloads reconstructed from the
documented eframe/egui layouts. They exercise `apps/photocraft/src/ui_state.rs` (the flat
window entry versus the nested `normal_geometry` entry) only. They do not prove that eframe
and egui deserialize a real `Memory` or `PanelState`; a captured 0.14/0.15/0.37 `ui.ron`
would be needed for that, with the machine-specific data reviewed first.
