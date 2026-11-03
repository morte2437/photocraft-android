//! The native dock's state (#88): the panel groups of the end-side dock column, their order,
//! tab order, hidden tabs, heights and collapse.
//!
//! craft-ui's docking draws every layout (`crate::panel_docking`). Until the workspace is
//! customized, the default layout is built each frame from this state: one dock column of the
//! shown groups, top to bottom, where a group the user never resized gets its default height and
//! the last expanded group (Layers by default) fills what the others leave. The Window menu's
//! panel toggles, the icon rail, collapse and `ui.set` change this state.
//!
//! The layout is [`DockLayout`] in `UiState::dock` (serialisable, drivable with `ui.set`), saved
//! with Window › Workspace › New Workspace…, reset by Reset Workspace, and remembered across
//! launches in the preferences (`panelLayout`) while Remember Workspace Changes is on.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::state::{DockTabs, Panels};

/// A dock panel group (one tab strip).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Group {
    /// Color | Swatches | Gradients | Patterns.
    Color,
    /// Properties | Adjustments.
    Properties,
    /// Character | Paragraph (not in the default workspace; Window › Character opens it, #150).
    Character,
    /// Navigator | Histogram | Info.
    Navigator,
    /// History | Actions | Layer Comps.
    History,
    /// Layers | Channels | Paths.
    Layers,
    /// Brushes presets.
    Brushes,
}

impl Group {
    /// Photoshop Essentials order, top to bottom.
    pub const ALL: [Group; 7] = [Group::Color, Group::Properties, Group::Character, Group::Navigator, Group::History, Group::Layers, Group::Brushes];

    pub fn key(self) -> &'static str {
        match self {
            Group::Color => "color",
            Group::Properties => "properties",
            Group::Character => "character",
            Group::Navigator => "navigator",
            Group::History => "history",
            Group::Layers => "layers",
            Group::Brushes => "brushes",
        }
    }

    /// Height (points, tab strip included) a group gets until the user resizes it, when the
    /// column has room.
    pub fn default_height(self) -> f32 {
        match self {
            Group::Color => 190.0,
            Group::Properties => 250.0,
            Group::Character => 270.0,
            Group::Navigator => 210.0,
            Group::History => 200.0,
            Group::Layers => 320.0,
            Group::Brushes => 200.0,
        }
    }

    /// What a group left at its default height gives way down to so the filler (Layers) keeps
    /// [`Group::preferred_fill`] in a short column (#147). Content taller than this scrolls.
    pub fn compact_height(self) -> f32 {
        match self {
            Group::Color => 130.0,
            Group::Properties => 160.0,
            Group::Character => 160.0,
            Group::Navigator => 140.0,
            Group::History => 130.0,
            Group::Layers => 200.0,
            Group::Brushes => 150.0,
        }
    }

    /// The height the filling group asks for before default-sized groups above it get their
    /// full defaults: Layers wants room for about ten rows at 900 pt (#147).
    pub fn preferred_fill(self) -> f32 {
        match self {
            Group::Layers => 500.0,
            Group::Brushes => 100.0,
            g => g.min_height(),
        }
    }

    /// The smallest height an expanded group can be dragged or squeezed to.
    pub fn min_height(self) -> f32 {
        match self {
            Group::Layers => 140.0,
            Group::Brushes => 100.0,
            _ => 80.0,
        }
    }

    pub fn tabs(self, pro: bool) -> &'static [&'static str] {
        match self {
            // Photoshop Essentials: Color | Swatches | Gradients | Patterns.
            Group::Color if pro => &["Color", "Swatches", "Gradients", "Patterns"],
            Group::Color => &["Swatches", "Color", "Gradients", "Patterns"],
            Group::Properties => &["Properties", "Adjustments"],
            Group::Character => &["Character", "Paragraph"],
            Group::Navigator => &["Navigator", "Histogram", "Info"],
            Group::History => &["History", "Actions", "Layer Comps"],
            Group::Layers => &["Layers", "Channels", "Paths"],
            Group::Brushes => &["Brushes"],
        }
    }

    /// Tabs that lay out their own scrolling list and footer, or size themselves to the group
    /// (the Navigator's preview grows with it, as in Photoshop: #2726). Channels and Paths are
    /// Layers' siblings and do the same: wrapped in the dock's scroll area as well, the two nested
    /// areas shared the wheel, and where the outer content overflowed by one physical pixel (at
    /// 125 % scaling for some window heights) the footer and list moved a pixel as you scrolled (#6238).
    pub fn scrolls_itself(self, tab: usize) -> bool {
        matches!((self, tab), (Group::Layers, 0..=2) | (Group::History, 0) | (Group::History, 1) | (Group::Navigator, 0))
    }

    pub(crate) fn tab_mut(self, tabs: &mut DockTabs) -> &mut usize {
        match self {
            Group::Color => &mut tabs.color,
            Group::Properties => &mut tabs.properties,
            Group::Character => &mut tabs.character,
            Group::Navigator => &mut tabs.navigator,
            Group::History => &mut tabs.history,
            Group::Layers => &mut tabs.layers,
            Group::Brushes => &mut tabs.brushes,
        }
    }

    /// The group for a `panels` / `dockTabs` key ("color", "properties", …).
    pub fn from_key(key: &str) -> Option<Group> {
        Group::ALL.into_iter().find(|g| g.key() == key)
    }

    pub fn shown(self, panels: &Panels) -> bool {
        match self {
            Group::Color => panels.color,
            Group::Properties => panels.properties,
            Group::Character => panels.character,
            Group::Navigator => panels.navigator,
            Group::History => panels.history,
            Group::Layers => panels.layers,
            Group::Brushes => panels.brushes,
        }
    }

    pub(crate) fn shown_mut(self, panels: &mut Panels) -> &mut bool {
        match self {
            Group::Color => &mut panels.color,
            Group::Properties => &mut panels.properties,
            Group::Character => &mut panels.character,
            Group::Navigator => &mut panels.navigator,
            Group::History => &mut panels.history,
            Group::Layers => &mut panels.layers,
            Group::Brushes => &mut panels.brushes,
        }
    }
}

/// Order, heights, collapsed state and visibility of individual dock tabs.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DockLayout {
    /// Top-to-bottom order; a group missing here (say one added after the layout was saved)
    /// goes just below the nearest group that precedes it by default (or last).
    pub order: Vec<Group>,
    /// Heights the user dragged groups to (points, tab strip included). Unset = default.
    pub heights: BTreeMap<Group, f32>,
    /// Groups collapsed to their tab strip.
    pub collapsed: Vec<Group>,
    /// Names of tabs hidden by Close (not positions: Color/Swatches swap with the theme).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub hidden_tabs: BTreeMap<Group, Vec<String>>,
    /// Tab order per group, left to right, by name like `hidden_tabs` (#2272). A group missing
    /// here keeps its default order; unknown names are ignored and tabs missing from a list
    /// follow it in their default order. Anything unreadable falls back to the defaults.
    #[serde(rename = "tabOrder", alias = "tab_order", deserialize_with = "lenient_tab_order")]
    pub tab_order: BTreeMap<Group, Vec<String>>,
}

/// Read `tabOrder` keeping what makes sense: unknown groups, non-list values and non-string names
/// are dropped instead of failing the whole saved layout.
fn lenient_tab_order<'de, D: serde::Deserializer<'de>>(d: D) -> Result<BTreeMap<Group, Vec<String>>, D::Error> {
    let v = Value::deserialize(d)?;
    let mut out = BTreeMap::new();
    for (k, names) in v.as_object().into_iter().flatten() {
        let (Some(g), Some(names)) = (Group::from_key(k), names.as_array()) else { continue };
        out.insert(g, names.iter().filter_map(Value::as_str).take(16).map(str::to_owned).collect());
    }
    Ok(out)
}

/// Upper bound on a stored height (guards against absurd values from `ui.set`).
const MAX_HEIGHT: f32 = 4000.0;

impl DockLayout {
    /// Every tab of `group` in the user's order (#2272), with its original index (the one
    /// `dockTabs`, commands and panel bodies use).
    pub fn ordered_tabs(&self, group: Group, pro: bool) -> Vec<(usize, &'static str)> {
        let all: Vec<(usize, &'static str)> = group.tabs(pro).iter().copied().enumerate().collect();
        let mut out: Vec<(usize, &'static str)> = Vec::with_capacity(all.len());
        for name in self.tab_order.get(&group).into_iter().flatten() {
            if let Some(&tab) = all.iter().find(|(_, n)| *n == name.as_str())
                && !out.contains(&tab)
            {
                out.push(tab);
            }
        }
        for tab in all {
            if !out.contains(&tab) {
                out.push(tab);
            }
        }
        out
    }

    /// Visible tabs in display order, retaining their original indices for commands and panel
    /// bodies.
    pub fn visible_tabs(&self, group: Group, pro: bool) -> Vec<(usize, &'static str)> {
        let hidden = self.hidden_tabs.get(&group);
        self.ordered_tabs(group, pro).into_iter().filter(|(_, name)| !hidden.is_some_and(|xs| xs.iter().any(|x| x.as_str() == *name))).collect()
    }

    /// Hide one tab; it can be reopened through Window › Panel.
    pub fn hide_tab(&mut self, group: Group, tab: usize, pro: bool) {
        let Some(&name) = group.tabs(pro).get(tab) else { return };
        let hidden = self.hidden_tabs.entry(group).or_default();
        if !hidden.iter().any(|x| x.as_str() == name) {
            hidden.push(name.to_owned());
        }
    }

    /// Restore the tab requested by the Window menu or icon rail.
    pub fn show_tab(&mut self, group: Group, tab: usize, pro: bool) {
        let Some(&name) = group.tabs(pro).get(tab) else { return };
        if let Some(hidden) = self.hidden_tabs.get_mut(&group) {
            hidden.retain(|n| n != name);
            if hidden.is_empty() {
                self.hidden_tabs.remove(&group);
            }
        }
    }

    /// Every group once, in display order.
    pub fn order(&self) -> Vec<Group> {
        let mut out: Vec<Group> = Vec::with_capacity(Group::ALL.len());
        for g in &self.order {
            if !out.contains(g) {
                out.push(*g);
            }
        }
        // Missing groups slot in below their default predecessor, so a group new to an old
        // saved layout (Character) never lands below Layers and takes over as the filler.
        for (i, g) in Group::ALL.iter().enumerate() {
            if out.contains(g) {
                continue;
            }
            let prev = Group::ALL.iter().take(i).rev().find_map(|p| out.iter().position(|x| x == p));
            match prev {
                Some(at) => out.insert(at + 1, *g),
                None => out.push(*g),
            }
        }
        out
    }

    pub fn is_collapsed(&self, g: Group) -> bool {
        self.collapsed.contains(&g)
    }

    pub fn set_collapsed(&mut self, g: Group, on: bool) {
        self.collapsed.retain(|c| *c != g);
        if on {
            self.collapsed.push(g);
        }
    }

    /// The stored (or default) height, sanitised.
    pub fn height(&self, g: Group) -> f32 {
        match self.heights.get(&g) {
            Some(h) if h.is_finite() => h.clamp(g.min_height(), MAX_HEIGHT),
            _ => g.default_height(),
        }
    }
}

/// Show `g` and expand it (Window › <panel>, the icon rail): a panel asked for is always
/// brought back, whatever state it was left in (#129).
pub fn reveal(app: &mut PhotocraftApp, g: Group) {
    *g.shown_mut(&mut app.ui.panels) = true;
    let selected = *g.tab_mut(&mut app.ui.dock_tabs);
    let pro = app.ui.theme.is_pro();
    app.ui.dock.show_tab(g, selected, pro);
    app.ui.dock.set_collapsed(g, false);
}

/// Icon rail click: a hidden group is shown, a collapsed one expanded and an expanded one
/// collapsed to its tab strip. A docked group is never hidden from the rail (it used to
/// toggle visibility, so one stray click made a panel vanish: #129); `docked` is false for
/// Studio's floating Properties card, which the rail shows and hides.
pub fn rail_click(app: &mut PhotocraftApp, g: Group, docked: bool) {
    if !g.shown(&app.ui.panels) {
        reveal(app, g);
    } else if !docked {
        *g.shown_mut(&mut app.ui.panels) = false;
    } else {
        let collapse = !app.ui.dock.is_collapsed(g);
        app.ui.dock.set_collapsed(g, collapse);
    }
}

/// A tab strip's Close: hide `tab` of `g`, and the group with its last tab. When the selected
/// tab closes, the next one on the strip (or the previous one, for the last) is selected.
pub(crate) fn close_tab(app: &mut PhotocraftApp, g: Group, tab: usize, pro: bool) {
    let shown_before = app.ui.dock.visible_tabs(g, pro);
    app.ui.dock.hide_tab(g, tab, pro);
    let remaining = app.ui.dock.visible_tabs(g, pro);
    if remaining.is_empty() {
        *g.shown_mut(&mut app.ui.panels) = false;
    } else if *g.tab_mut(&mut app.ui.dock_tabs) == tab {
        // Prefer the next tab on the strip, or the previous one when closing the last.
        let at = shown_before.iter().position(|(i, _)| *i == tab).unwrap_or(0);
        let next = shown_before.iter().skip(at + 1).find(|x| remaining.contains(x)).or_else(|| remaining.last());
        if let Some(&(next, _)) = next {
            *g.tab_mut(&mut app.ui.dock_tabs) = next;
        }
    }
}

/// What `prefs.panelLayout` holds: the live layout and open panels.
pub(crate) fn snapshot(app: &PhotocraftApp) -> Value {
    let mut layout = json!({"workspace": app.ui.workspace, "panels": app.ui.panels, "dockTabs": app.ui.dock_tabs, "dock": app.ui.dock, "timelineOpen": app.ui.timeline.open, "docking": app.ui.docking, "dockingReturns": app.ui.docking_returns, "dockingCollapsed": app.ui.docking_collapsed});
    keep_newer(app, &mut layout);
    layout
}

/// A saved layout's workspace from a newer version stays as saved while the default layout
/// shows in its place (craft-ui: keep it on disk, don't overwrite it).
pub(crate) fn keep_newer(app: &PhotocraftApp, layout: &mut Value) {
    if let (None, Some(kept), Some(layout)) = (&app.ui.docking, &app.ui.docking_kept, layout.as_object_mut()) {
        layout.extend(kept.clone());
    }
}

/// Remember the layout in the preferences once the user lets go of the mouse (Workspace ›
/// Remember Workspace Changes), and once panel windows the OS moves or resizes keep still. Cheap:
/// a small JSON compare per frame.
pub fn persist(app: &mut PhotocraftApp, ctx: &egui::Context) {
    // Once the layout changes, the newer workspace is replaced.
    if app.ui.docking.is_some() {
        app.ui.docking_kept = None;
    }
    if !app.session.prefs().workspace.remember_workspace_changes || ctx.input(|i| i.pointer.any_down()) {
        return;
    }
    if native_moved(app, ctx) {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
        return;
    }
    let now = snapshot(app);
    if app.session.prefs().panel_layout != now {
        app.session.prefs.edit(|p| p.panel_layout = now);
    }
}

/// A floating panel's own window has another place than last frame. A move or resize by its OS
/// title bar or edges changes the place each frame with no button down in any of the app's
/// windows.
fn native_moved(app: &PhotocraftApp, ctx: &egui::Context) -> bool {
    let places: Vec<(u64, [f32; 4])> =
        app.ui.docking.iter().flat_map(|workspace| workspace.layout.floating.iter().filter_map(|window| Some((window.id, window.native?)))).collect();
    let id = egui::Id::unique("dock-native-places");
    ctx.data_mut(|data| {
        let moved = data.get_temp::<Vec<(u64, [f32; 4])>>(id).is_some_and(|last| last != places);
        data.insert_temp(id, places);
        moved
    })
}

/// Restore the remembered layout at launch. Unreadable parts keep their defaults.
pub fn restore(app: &mut PhotocraftApp) {
    if !app.session.prefs().workspace.remember_workspace_changes {
        return;
    }
    let saved = app.session.prefs().panel_layout.clone();
    apply(app, &saved);
    // The floating Brush Settings window is transient, not a dock panel: remembering the
    // workspace must not leave it open on a fresh launch, especially with no document.
    // Explicit workspace switches still use `apply` and keep their stored panel visibility.
    app.ui.panels.brush_settings = false;
    if let Some(ws) = saved.get("workspace").and_then(Value::as_str) {
        app.ui.workspace = ws.to_string();
    }
}

/// Apply the `panels`, `dockTabs`, `dock` and `timelineOpen` parts of a saved layout
/// (a workspace or `panelLayout`). Missing or invalid dock parts are left alone; old
/// layouts without Timeline visibility restore it closed.
///
/// `craft_ui::docking::DockArea` keeps only transient drag/focus data in `ctx.data`; the
/// split/tab arrangement is the app's own `craft_ui::docking::Layout` JSON stored here. If a
/// layout saved by a different craft-ui revision no longer deserializes, it is dropped and the
/// legacy layout is rebuilt (the split/tab arrangement resets). Panel widths are a separate
/// `egui::PanelState` migration and still carry over.
pub fn apply(app: &mut PhotocraftApp, v: &Value) {
    app.ui.docking_reveal = None;
    app.ui.docking_generation = app.ui.docking_generation.wrapping_add(1);
    (app.ui.docking, app.ui.docking_returns) = crate::panel_docking::load(v).map_or((None, Default::default()), |(w, r)| (Some(w), r));
    app.ui.docking_kept = (app.ui.docking.is_none() && crate::panel_docking::newer(v))
        .then(|| ["docking", "dockingReturns", "dockingCollapsed"].into_iter().filter_map(|key| Some((key.to_owned(), v.get(key)?.clone()))).collect());
    // Collapsed custom groups belong to the workspace; the native dock keeps its own flags.
    app.ui.docking_collapsed = match app.ui.docking {
        Some(_) => v.get("dockingCollapsed").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default(),
        None => Vec::new(),
    };
    if let Some(mut p) = v.get("panels").and_then(|p| serde_json::from_value::<Panels>(p.clone()).ok()) {
        // The embedding host owns these flags, including for layouts that serialize them.
        p.menu_bar = app.ui.panels.menu_bar;
        p.rail = app.ui.panels.rail;
        app.ui.panels = p;
    }
    if let Some(t) = v.get("dockTabs").and_then(|t| serde_json::from_value(t.clone()).ok()) {
        app.ui.dock_tabs = t;
    }
    if let Some(d) = v.get("dock").and_then(|d| serde_json::from_value(d.clone()).ok()) {
        app.ui.dock = d;
    }
    crate::panel_docking::sync_visibility(app);
    app.ui.timeline.open = v.get("timelineOpen").and_then(Value::as_bool).unwrap_or(false);
    if !app.ui.timeline.open {
        app.ui.timeline.playing = false;
    }
}

#[cfg(test)]
mod tests;
