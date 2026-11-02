// ----------------------------------------------------------------------------- panel state

/// The persisted [`egui::PanelState`] key for the panel named `salt` in this context's root UI.
///
/// egui 0.37 derives a panel's id from its parent [`egui::Ui`]:
/// `panel_ui.scope_id().with_salt(salt)`. For the root Ui that is
/// `ctx.viewport_id().root_ui_id().with(salt)`, whereas 0.36 stored the same panels under
/// `Id::new(salt)` (`Id::unique(salt)` has the same hash). Use this in tests and migrations; never
/// substitute [`egui::Ui::unique_id`], which is unstable.
pub fn panel_state_id(ctx: &egui::Context, salt: impl egui::AsIdSalt) -> egui::Id {
    ctx.viewport_id().root_ui_id().with(salt)
}

/// The panels whose saved sizes moved to a parent-scoped key in 0.37.
const PANEL_STATE_KEYS: &[&str] = &["dock", "title_bar", "options_bar", "status_bar", "toolbar", "rail", "shared-panel-dock"];

/// What the two keys held when this build last looked at a panel, so a later startup can tell
/// which one a 0.36 rollback or a 0.37 session wrote last.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
struct PanelStamp {
    old: [f32; 4],
    new: [f32; 4],
}

fn rect_key(state: egui::containers::panel::PanelState) -> [f32; 4] {
    let r = state.outer_rect;
    [r.min.x, r.min.y, r.max.x, r.max.y]
}

fn stamp_id(name: &str) -> egui::Id {
    egui::Id::unique(format!("photocraft-panel-state-stamp:{name}"))
}

/// Copy 0.36 `PanelState` entries to their 0.37 parent-scoped keys.
///
/// The old `Id::unique(name)` entry is left in place so a 0.36 build still reads the same saved
/// widths. On a later startup, a stamp of both keys decides which one is newer: if a 0.36 build
/// resized the panel after the migration (the old key differs from the stamp, the new one does
/// not), the old key wins; a 0.37 resize (the new key differs) is kept. If both changed the order
/// is unknown; the new key is kept, so a 0.37 resize *after* a 0.36 rollback resize can still be
/// lost (documented in the plan's rollback section). Data-only: call it before any panel is shown.
pub fn migrate_panel_state(ctx: &egui::Context) {
    use egui::containers::panel::PanelState;
    let done = egui::Id::unique("photocraft-panel-state-migrated");
    if ctx.data(|d| d.get_temp::<bool>(done).unwrap_or(false)) {
        return;
    }
    for name in PANEL_STATE_KEYS {
        let old_id = egui::Id::unique(*name);
        let new_id = panel_state_id(ctx, *name);
        let id = stamp_id(name);
        let old = PanelState::load(ctx, old_id);
        let new = PanelState::load(ctx, new_id);
        let stamp = ctx.data_mut(|d| d.get_persisted::<PanelStamp>(id));
        let Some(old) = old else { continue };
        let old_key = rect_key(old);
        match (new, stamp) {
            (None, _) => {
                // First migration: copy, and remember what both keys held.
                ctx.data_mut(|d| d.insert_persisted(new_id, old));
                ctx.data_mut(|d| d.insert_persisted(id, PanelStamp { old: old_key, new: old_key }));
            }
            (Some(new), None) => {
                // A partial migration or a pre-existing 0.37 value: keep it and start stamping.
                ctx.data_mut(|d| d.insert_persisted(id, PanelStamp { old: old_key, new: rect_key(new) }));
            }
            (Some(new), Some(stamp)) => {
                let old_changed = old_key != stamp.old;
                let new_changed = rect_key(new) != stamp.new;
                if old_changed && !new_changed {
                    // A 0.36 rollback resized the panel after the migration: its key is newer.
                    ctx.data_mut(|d| d.insert_persisted(new_id, old));
                    ctx.data_mut(|d| d.insert_persisted(id, PanelStamp { old: old_key, new: old_key }));
                } else if new_changed {
                    ctx.data_mut(|d| d.insert_persisted(id, PanelStamp { old: old_key, new: rect_key(new) }));
                }
            }
        }
    }
    ctx.data_mut(|d| d.insert_temp(done, true));
}

/// Remember the 0.37 panel sizes this session ends a pass with, so a later 0.36 rollback's newer
/// old-key values can be recognized. Cheap when nothing changed (no write).
pub fn remember_panel_state(ctx: &egui::Context) {
    use egui::containers::panel::PanelState;
    for name in PANEL_STATE_KEYS {
        let Some(new) = PanelState::load(ctx, panel_state_id(ctx, *name)) else { continue };
        let id = stamp_id(name);
        let Some(stamp) = ctx.data_mut(|d| d.get_persisted::<PanelStamp>(id)) else { continue };
        let new_key = rect_key(new);
        if new_key != stamp.new {
            ctx.data_mut(|d| d.insert_persisted(id, PanelStamp { old: stamp.old, new: new_key }));
        }
    }
}
