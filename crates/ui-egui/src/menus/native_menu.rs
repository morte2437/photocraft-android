//! The macOS menu bar: the same menus as the in-window bar ([`crate::menus::menu_items`]), laid
//! out the way a Mac app's are and handed to a platform [`Backend`] (the desktop app's muda one).
//!
//! Pure and platform-free, so it is tested everywhere and builds for the web; only the backend
//! touches AppKit. What the layout does ([`mac_layout`]):
//! - an **app menu** named PhotoCraft: About (from Help), Settings (Edit › Preferences), Language,
//!   Appearance (Window › Theme), Services, Hide, Hide Others, Show All and Quit (File › Exit,
//!   still the app's own `file.exit` so the unsaved-changes prompt runs). The other Craft apps
//!   use the same app menu;
//! - **Window** gets Minimize, Zoom and Bring All to Front, with the open documents after it
//!   (where a Mac app lists its windows), and **Help** the system search field;
//! - **clashes** with the system's keys (⌘H, ⌘M, ⌘W, ⌘,) are resolved in PhotoCraft's favour, and
//!   reported; Hide and Minimize move to ⌃⌘H and ⌃⌘M, as Photoshop does.
//!
//! **Keys stay with egui.** AppKit runs a menu's key equivalents before the window sees the key,
//! which would skip PhotoCraft's key rules (text fields keep ⌘C/⌘V/⌘Z, dialogs and Camera Raw are
//! modal, Liquify has its own ⌘Z). So the backend reports a key equivalent as [`Event::Key`], and
//! [`NativeMenu::raw_input`] feeds it to egui as the key press it was: the shortcut runs through
//! [`crate::shortcuts::handle`] exactly as without a native menu. A click is [`Event::Click`] and
//! runs the command like an in-window menu click.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use egui::{Key, Modifiers};

use crate::PhotocraftApp;
use crate::i18n::{Lang, tr, tr_id};
use crate::menus::MenuItem;

pub const APP_NAME: &str = "PhotoCraft";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MenuBar {
    pub menus: Vec<Menu>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Menu {
    pub title: String,
    pub role: MenuRole,
    pub children: Vec<Node>,
}

/// What a top-level menu is for. macOS treats the app, Window and Help menus specially.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MenuRole {
    #[default]
    Normal,
    App,
    Window,
    Help,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Item(Item),
    Separator,
    Submenu {
        label: String,
        children: Vec<Node>,
        role: Option<ItemRole>,
    },
    /// An item AppKit provides and runs itself (Services, Zoom, …).
    Standard(Standard),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// The command id (`file.open`), or one of [`HIDE`] / [`MINIMIZE`].
    pub id: String,
    pub label: String,
    /// `Cmd+Shift+Z` form; `Ctrl` is the real Control key.
    pub shortcut: Option<String>,
    pub enabled: bool,
    /// `Some` for check items.
    pub checked: Option<bool>,
    pub role: Option<ItemRole>,
}

/// Items macOS expects in fixed places.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ItemRole {
    About,
    Settings,
    Quit,
    Hide,
    Minimize,
    /// Window › Theme and Next Theme, which become the app menu's Appearance submenu.
    Appearance,
    /// The app menu's Language submenu (its labels are final, never translated by id).
    Language,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Standard {
    Services,
    HideOthers,
    ShowAll,
    Minimize,
    Zoom,
    BringAllToFront,
    CloseWindow,
}

/// Hide PhotoCraft: a custom item (the predefined one always takes ⌘H, which is View › Extras).
pub const HIDE: &str = "app.hide";
/// Minimize: a custom item (the predefined one always takes ⌘M, which is Curves).
pub const MINIMIZE: &str = "window.minimize";

impl Item {
    fn new(id: &str, label: &str) -> Item {
        Item { id: id.into(), label: label.into(), shortcut: None, enabled: true, checked: None, role: None }
    }
}

impl MenuBar {
    /// Every item, depth first.
    pub fn items(&self) -> Vec<&Item> {
        fn walk<'a>(nodes: &'a [Node], out: &mut Vec<&'a Item>) {
            for n in nodes {
                match n {
                    Node::Item(it) => out.push(it),
                    Node::Submenu { children, .. } => walk(children, out),
                    Node::Separator | Node::Standard(_) => {}
                }
            }
        }
        let mut out = Vec::new();
        for m in &self.menus {
            walk(&m.children, &mut out);
        }
        out
    }

    pub fn find(&self, id: &str) -> Option<&Item> {
        self.items().into_iter().find(|it| it.id == id)
    }

    pub fn menu(&self, role: MenuRole) -> Option<&Menu> {
        self.menus.iter().find(|m| m.role == role)
    }

    fn tag_item(&mut self, id: &str, role: ItemRole) {
        fn walk(nodes: &mut [Node], id: &str, role: ItemRole) {
            for n in nodes {
                match n {
                    Node::Item(it) if it.id == id => it.role = Some(role),
                    Node::Submenu { children, .. } => walk(children, id, role),
                    _ => {}
                }
            }
        }
        for m in &mut self.menus {
            walk(&mut m.children, id, role);
        }
    }

    fn tag_submenu(&mut self, top: &str, name: &str, role: ItemRole) {
        let Some(menu) = self.menus.iter_mut().find(|m| m.title == top) else { return };
        for n in &mut menu.children {
            if let Node::Submenu { label, role: r, .. } = n
                && label == name
            {
                *r = Some(role);
            }
        }
    }

    fn tag_menu(&mut self, title: &str, role: MenuRole) {
        if let Some(m) = self.menus.iter_mut().find(|m| m.title == title) {
            m.role = role;
        }
    }
}

/// The menus as a tree, from the flat rows [`crate::menus::menu_items`] returns, in the order the
/// in-window bar draws them: at each depth leaves and separators in order, and a submenu where its
/// first child is.
pub fn from_items(tops: &[&str], items: &[MenuItem]) -> MenuBar {
    fn level(rows: &[&MenuItem], depth: usize) -> Vec<Node> {
        let mut out = Vec::new();
        let mut shown: Vec<&str> = Vec::new();
        for it in rows {
            if it.path.len() == depth {
                if it.label == "---" {
                    out.push(Node::Separator);
                } else {
                    out.push(Node::Item(Item {
                        id: it.id.clone(),
                        label: it.label.clone(),
                        shortcut: it.shortcut.clone(),
                        enabled: it.enabled,
                        checked: it.checked,
                        role: None,
                    }));
                }
            } else if let Some(name) = it.path.get(depth) {
                let name = name.as_str();
                if shown.contains(&name) {
                    continue;
                }
                shown.push(name);
                let child: Vec<&MenuItem> = rows.iter().copied().filter(|c| c.path.get(depth).map(String::as_str) == Some(name)).collect();
                out.push(Node::Submenu { label: name.to_string(), children: level(&child, depth + 1), role: None });
            }
        }
        tidy(out)
    }
    let menus = tops
        .iter()
        .map(|top| {
            let rows: Vec<&MenuItem> = items.iter().filter(|i| i.path.first().map(String::as_str) == Some(*top)).collect();
            Menu { title: top.to_string(), role: MenuRole::Normal, children: level(&rows, 1) }
        })
        .collect();
    MenuBar { menus }
}

/// Drop separators at the start or end of a level and collapse runs of them, at every depth.
fn tidy(nodes: Vec<Node>) -> Vec<Node> {
    let mut out: Vec<Node> = Vec::with_capacity(nodes.len());
    for n in nodes {
        let n = match n {
            Node::Submenu { label, children, role } => Node::Submenu { label, children: tidy(children), role },
            other => other,
        };
        if matches!(n, Node::Separator) && matches!(out.last(), None | Some(Node::Separator)) {
            continue;
        }
        out.push(n);
    }
    while matches!(out.last(), Some(Node::Separator)) {
        out.pop();
    }
    out
}

/// A hash of everything a native menu can only change by rebuilding: menus, roles, item ids,
/// kinds (plain or check), shortcuts and submenu labels. Labels, enabled and checked are updated
/// in place. Window's list of open documents is left out: opening or closing a document rebuilds
/// only the Window menu ([`window_key`]).
pub fn structure_key(bar: &MenuBar) -> u64 {
    let mut h = DefaultHasher::new();
    for m in &bar.menus {
        (&m.title, m.role).hash(&mut h);
        if m.role == MenuRole::Window {
            let mut rest: Vec<Node> = m.children.iter().filter(|n| !is_document(n)).cloned().collect();
            while matches!(rest.last(), Some(Node::Separator)) {
                rest.pop();
            }
            hash_nodes(&rest, &mut h);
        } else {
            hash_nodes(&m.children, &mut h);
        }
    }
    h.finish()
}

/// The Window menu's structure, its open documents included: when it changes and nothing else
/// did, a native menu rebuilds just the Window menu.
pub fn window_key(bar: &MenuBar) -> u64 {
    let mut h = DefaultHasher::new();
    if let Some(m) = bar.menu(MenuRole::Window) {
        hash_nodes(&m.children, &mut h);
    }
    h.finish()
}

/// Window › <document> (see [`crate::menus::DOCUMENT_ITEM`]).
fn is_document(n: &Node) -> bool {
    matches!(n, Node::Item(it) if it.id.starts_with(crate::menus::DOCUMENT_ITEM))
}

fn hash_nodes(nodes: &[Node], h: &mut DefaultHasher) {
    for n in nodes {
        match n {
            Node::Item(it) => ('i', &it.id, it.checked.is_some(), &it.shortcut, it.role).hash(h),
            Node::Separator => '-'.hash(h),
            Node::Submenu { label, children, role } => {
                ('[', label, role).hash(h);
                hash_nodes(children, h);
                ']'.hash(h);
            }
            Node::Standard(s) => ('s', s).hash(h),
        }
    }
}

// ----------------------------------------------------------------------------- shortcuts

/// A shortcut as PhotoCraft writes it (`Cmd+Shift+Z`). `Cmd` is ⌘ and `Ctrl` the Control key,
/// never folded together: ⌃⌘M and ⌘M are different shortcuts.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Chord {
    pub cmd: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// The key as written: `Z`, `1`, `'`, `F5`, `Delete`, `Enter`.
    pub key: String,
}

impl Chord {
    pub fn parse(s: &str) -> Option<Chord> {
        let mut c = Chord { cmd: false, ctrl: false, alt: false, shift: false, key: String::new() };
        for part in s.split('+') {
            match part {
                "Cmd" => c.cmd = true,
                "Ctrl" => c.ctrl = true,
                "Alt" => c.alt = true,
                "Shift" => c.shift = true,
                // `Cmd++` splits into an empty last part.
                "" => c.key = "+".into(),
                k if c.key.is_empty() => c.key = k.into(),
                _ => return None,
            }
        }
        (!c.key.is_empty()).then_some(c)
    }

    /// The written form, modifiers in a fixed order.
    pub fn portable(&self) -> String {
        let mut parts = Vec::new();
        for (on, name) in [(self.ctrl, "Ctrl"), (self.cmd, "Cmd"), (self.alt, "Alt"), (self.shift, "Shift")] {
            if on {
                parts.push(name);
            }
        }
        parts.push(&self.key);
        parts.join("+")
    }

    pub fn is_function_key(&self) -> bool {
        self.key.strip_prefix('F').and_then(|n| n.parse::<u8>().ok()).is_some_and(|n| (1..=24).contains(&n))
    }

    /// May the native menu show this as a key equivalent? Only with ⌘ or ⌃, or a function key.
    /// Bare letters and ⌥/⇧-only keys are tool keys: as key equivalents AppKit would take a `b`
    /// typed into a text field.
    pub fn native_ok(&self) -> bool {
        self.cmd || self.ctrl || self.is_function_key()
    }

    /// The key press egui would have received for this chord.
    pub fn key_event(&self, pressed: bool) -> Option<egui::Event> {
        let key = match self.key.as_str() {
            "+" => Key::Plus,
            "'" => Key::Quote,
            ";" => Key::Semicolon,
            "=" => Key::Equals,
            "-" => Key::Minus,
            "[" => Key::OpenBracket,
            "]" => Key::CloseBracket,
            "\\" => Key::Backslash,
            "/" => Key::Slash,
            "," => Key::Comma,
            "." => Key::Period,
            k => Key::from_name(k)?,
        };
        let modifiers = Modifiers { alt: self.alt, ctrl: self.ctrl, shift: self.shift, mac_cmd: self.cmd, command: self.cmd };
        Some(egui::Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers })
    }
}

// ----------------------------------------------------------------------------- macOS layout

/// A system key equivalent PhotoCraft already uses for something else.
#[derive(Clone, Debug, PartialEq)]
pub struct Clash {
    pub shortcut: &'static str,
    /// What macOS uses it for.
    pub system: &'static str,
    /// The PhotoCraft command bound to it.
    pub command: String,
    /// What the layout did about it.
    pub resolution: &'static str,
}

pub struct Layout {
    pub bar: MenuBar,
    pub clashes: Vec<Clash>,
}

/// Turn PhotoCraft's menus (tagged with roles) into a Mac menu bar. See the module docs.
pub fn mac_layout(bar: &MenuBar, lang: Lang) -> Layout {
    let mut bar = bar.clone();
    let mut clashes = Vec::new();
    let bound: HashMap<String, String> = bar.items().iter().filter_map(|it| Some((Chord::parse(it.shortcut.as_deref()?)?.portable(), it.id.clone()))).collect();
    let taken = |sc: &str, own: Option<&str>| bound.get(sc).filter(|cmd| Some(cmd.as_str()) != own).cloned();
    let named = |s: &str| crate::i18n::fmt(tr(lang, s), &[("app", APP_NAME)]);

    let about = take(&mut bar, |n| matches!(n, Node::Item(it) if it.role == Some(ItemRole::About)));
    let settings = take(&mut bar, |n| match n {
        Node::Item(it) => it.role == Some(ItemRole::Settings),
        Node::Submenu { role, .. } => *role == Some(ItemRole::Settings),
        _ => false,
    });
    let quit = take(&mut bar, |n| matches!(n, Node::Item(it) if it.role == Some(ItemRole::Quit)));
    let appearance = appearance_node(&mut bar, lang);

    let mut menu = Vec::new();
    if let Some(Node::Item(mut it)) = about {
        it.label = named("About {app}");
        menu.extend([Node::Item(it), Node::Separator]);
    }
    if let Some(s) = settings {
        let own = node_ids(&s);
        let comma = bound.get("Cmd+,").filter(|cmd| !own.contains(*cmd)).cloned();
        menu.push(settings_node(s, comma, lang, &mut clashes));
    }
    menu.extend(appearance);
    if !menu.is_empty() && menu.last() != Some(&Node::Separator) {
        menu.push(Node::Separator);
    }
    menu.extend([Node::Standard(Standard::Services), Node::Separator]);
    let mut hide = Item::new(HIDE, &named("Hide {app}"));
    hide.role = Some(ItemRole::Hide);
    match taken("Cmd+H", None) {
        Some(command) => {
            let free = taken("Ctrl+Cmd+H", None).is_none();
            if free {
                hide.shortcut = Some("Ctrl+Cmd+H".into());
            }
            let resolution = if free { "Hide moves to ⌃⌘H" } else { "Hide has no shortcut" };
            clashes.push(Clash { shortcut: "Cmd+H", system: "Hide", command, resolution });
        }
        None => hide.shortcut = Some("Cmd+H".into()),
    }
    menu.push(Node::Item(hide));
    if let Some(command) = taken("Cmd+Alt+H", None) {
        clashes.push(Clash { shortcut: "Cmd+Alt+H", system: "Hide Others", command, resolution: "AppKit's Hide Others keeps ⌥⌘H" });
    }
    menu.extend([Node::Standard(Standard::HideOthers), Node::Standard(Standard::ShowAll), Node::Separator]);
    if let Some(Node::Item(mut quit)) = quit {
        quit.label = named("Quit {app}");
        match taken("Cmd+Q", Some(&quit.id)) {
            Some(command) => {
                clashes.push(Clash { shortcut: "Cmd+Q", system: "Quit", command, resolution: "Quit has no shortcut" });
                quit.shortcut = None;
            }
            None => quit.shortcut = Some("Cmd+Q".into()),
        }
        menu.push(Node::Item(quit));
    }
    bar.menus.insert(0, Menu { title: APP_NAME.into(), role: MenuRole::App, children: menu });

    match taken("Cmd+W", None) {
        Some(command) => clashes.push(Clash { shortcut: "Cmd+W", system: "Close Window", command, resolution: "no system Close Window item" }),
        None => {
            if let Some(file) = bar.menus.iter_mut().find(|m| m.title == "File") {
                file.children.extend([Node::Separator, Node::Standard(Standard::CloseWindow)]);
            }
        }
    }

    if bar.menu(MenuRole::Window).is_none() {
        let at = bar.menus.iter().position(|m| m.role == MenuRole::Help).unwrap_or(bar.menus.len());
        bar.menus.insert(at, Menu { title: "Window".into(), role: MenuRole::Window, children: Vec::new() });
    }
    let minimize = match taken("Cmd+M", None) {
        Some(command) => {
            let mut it = Item::new(MINIMIZE, tr(lang, "Minimize"));
            it.role = Some(ItemRole::Minimize);
            let free = taken("Ctrl+Cmd+M", None).is_none();
            if free {
                it.shortcut = Some("Ctrl+Cmd+M".into());
            }
            let resolution = if free { "Minimize moves to ⌃⌘M" } else { "Minimize has no shortcut" };
            clashes.push(Clash { shortcut: "Cmd+M", system: "Minimize", command, resolution });
            Node::Item(it)
        }
        None => Node::Standard(Standard::Minimize),
    };
    if let Some(window) = bar.menus.iter_mut().find(|m| m.role == MenuRole::Window) {
        let (docs, rest): (Vec<Node>, Vec<Node>) = std::mem::take(&mut window.children).into_iter().partition(is_document);
        let mut children = vec![minimize, Node::Standard(Standard::Zoom), Node::Separator];
        children.extend(rest);
        children.extend([Node::Separator, Node::Standard(Standard::BringAllToFront), Node::Separator]);
        children.extend(docs);
        window.children = children;
    }

    for m in &mut bar.menus {
        m.children = tidy(std::mem::take(&mut m.children));
    }
    // A menu emptied by the moves goes away (the Help menu stays: it has the search field).
    bar.menus.retain(|m| !m.children.is_empty() || m.role == MenuRole::Help);
    Layout { bar, clashes }
}

/// Settings in the app menu: Edit › Preferences, a submenu of settings pages, becomes "Settings"
/// with ⌘, on its first page unless PhotoCraft binds ⌘, elsewhere.
fn settings_node(node: Node, comma_taken: Option<String>, lang: Lang, clashes: &mut Vec<Clash>) -> Node {
    if let Some(command) = &comma_taken {
        clashes.push(Clash { shortcut: "Cmd+,", system: "Settings", command: command.clone(), resolution: "Settings has no shortcut" });
    }
    let give = |it: &mut Item| {
        if comma_taken.is_none() && it.shortcut.is_none() {
            it.shortcut = Some("Cmd+,".into());
        }
    };
    match node {
        Node::Item(mut it) => {
            it.label = tr(lang, "Settings…").into();
            give(&mut it);
            Node::Item(it)
        }
        Node::Submenu { mut children, role, .. } => {
            if let Some(Node::Item(first)) = children.iter_mut().find(|n| matches!(n, Node::Item(i) if i.enabled)) {
                give(first);
            }
            Node::Submenu { label: tr(lang, "Settings").into(), children, role }
        }
        other => other,
    }
}

/// Appearance in the app menu: the themes (Window › Theme, tagged [`ItemRole::Appearance`]), then
/// Next Theme, both leaving the Window menu. None when there are no themes.
fn appearance_node(bar: &mut MenuBar, lang: Lang) -> Option<Node> {
    let Some(Node::Submenu { mut children, .. }) = take(bar, |n| matches!(n, Node::Submenu { role, .. } if *role == Some(ItemRole::Appearance))) else {
        return None;
    };
    if let Some(Node::Item(mut next)) = take(bar, |n| matches!(n, Node::Item(it) if it.role == Some(ItemRole::Appearance))) {
        // Placed now: translated by id like the themes above it.
        next.role = None;
        children.extend([Node::Separator, Node::Item(next)]);
    }
    Some(Node::Submenu { label: tr(lang, "Appearance").into(), children, role: Some(ItemRole::Appearance) })
}

/// Command ids of the app menu's Language items: `app.language.<code>`, where `auto` follows the
/// system (the `interface.language` preference values).
pub const LANGUAGE_PREFIX: &str = "app.language.";

/// Language in the app menu, after Settings: Auto, then every UI language in its own name, with
/// the current preference checked.
fn language_node(pref: &str, lang: Lang) -> Node {
    let item = |code: &str, label: &str| Item {
        id: format!("{LANGUAGE_PREFIX}{code}"),
        label: label.into(),
        shortcut: None,
        enabled: true,
        checked: Some(pref == code),
        role: Some(ItemRole::Language),
    };
    let mut children = vec![Node::Item(item("auto", tr(lang, "Auto"))), Node::Separator];
    children.extend(Lang::for_picker().into_iter().map(|l| Node::Item(item(l.code(), l.name()))));
    Node::Submenu { label: tr(lang, "Language").into(), children, role: Some(ItemRole::Language) }
}

fn node_ids(node: &Node) -> Vec<String> {
    match node {
        Node::Item(it) => vec![it.id.clone()],
        Node::Submenu { children, .. } => children.iter().flat_map(node_ids).collect(),
        _ => Vec::new(),
    }
}

/// Remove and return the first node matching `pred`, at any depth.
fn take(bar: &mut MenuBar, pred: impl Fn(&Node) -> bool + Copy) -> Option<Node> {
    fn walk(nodes: &mut Vec<Node>, pred: impl Fn(&Node) -> bool + Copy) -> Option<Node> {
        if let Some(i) = nodes.iter().position(pred) {
            return Some(nodes.remove(i));
        }
        nodes.iter_mut().find_map(|n| match n {
            Node::Submenu { children, .. } => walk(children, pred),
            _ => None,
        })
    }
    bar.menus.iter_mut().find_map(|m| walk(&mut m.children, pred))
}

/// Translate menu titles, submenus and items the layout didn't name itself.
fn translate(bar: &mut MenuBar, lang: Lang) {
    fn walk(nodes: &mut [Node], lang: Lang) {
        for n in nodes {
            match n {
                // A document's name is shown as it is.
                Node::Item(it) if it.role.is_none() && !it.id.starts_with(crate::menus::DOCUMENT_ITEM) => it.label = tr_id(lang, &it.id, &it.label).to_string(),
                Node::Submenu { label, children, role } => {
                    if role.is_none() {
                        *label = tr(lang, label).to_string();
                    }
                    walk(children, lang);
                }
                _ => {}
            }
        }
    }
    for m in &mut bar.menus {
        if m.role != MenuRole::App {
            m.title = tr(lang, &m.title).to_string();
        }
        walk(&mut m.children, lang);
    }
}

/// PhotoCraft's menus as a Mac menu bar, in the UI language; `language` is the
/// `interface.language` preference (`auto` or a code), checked in the app menu's Language.
pub fn photocraft_layout(items: &[MenuItem], lang: Lang, language: &str) -> Layout {
    let mut bar = from_items(&crate::menus::TOP_MENUS, items);
    bar.tag_item("help.about", ItemRole::About);
    bar.tag_item("file.exit", ItemRole::Quit);
    bar.tag_submenu("Edit", "Preferences", ItemRole::Settings);
    bar.tag_submenu("Window", "Theme", ItemRole::Appearance);
    bar.tag_item("window.theme.toggle", ItemRole::Appearance);
    bar.tag_menu("Window", MenuRole::Window);
    bar.tag_menu("Help", MenuRole::Help);
    let mut layout = mac_layout(&bar, lang);
    // Language goes right after Settings (before Appearance), as in the other Craft apps.
    if let Some(app_menu) = layout.bar.menus.iter_mut().find(|m| m.role == MenuRole::App) {
        let settings = |n: &Node| match n {
            Node::Item(it) => it.role == Some(ItemRole::Settings),
            Node::Submenu { role, .. } => *role == Some(ItemRole::Settings),
            _ => false,
        };
        let at = app_menu.children.iter().position(settings).map_or(0, |i| i + 1);
        app_menu.children.insert(at, language_node(language, lang));
    }
    translate(&mut layout.bar, lang);
    layout
}

// ----------------------------------------------------------------------------- the app's side

/// What the native menu reports.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// An item was chosen with the mouse (or the keyboard inside an open menu).
    Click(String),
    /// A key equivalent was pressed: the shortcut, to be handled as the key press it was.
    Key(Chord),
}

/// A platform's native menu bar (the desktop app implements it for macOS with muda).
pub trait Backend {
    /// Bring the menu in step with `bar`: update labels, enabled and checked in place, or
    /// rebuild when its structure changed.
    fn sync(&mut self, bar: &MenuBar);
    /// Events since the last call. Platform items (Hide, Services, Zoom, …) are handled by the
    /// backend and not reported.
    fn drain(&mut self) -> Vec<Event>;
}

/// The native menu bar, while PhotoCraft uses one (the in-window menus are hidden then).
pub struct NativeMenu {
    backend: Box<dyn Backend>,
    /// Hash of the state the menus were last built from.
    state: Option<u64>,
    clicks: Vec<String>,
    /// AppKit toggled a check item itself: re-sync.
    dirty: bool,
    logged: bool,
}

impl NativeMenu {
    pub fn new(backend: Box<dyn Backend>) -> NativeMenu {
        NativeMenu { backend, state: None, clicks: Vec::new(), dirty: true, logged: false }
    }

    /// Before egui sees this frame's input: key equivalents become the key presses they were,
    /// and clicks wait for [`NativeMenu::run`].
    ///
    /// AppKit takes only the key-down: winit still delivers the key-up (it forwards key-ups while
    /// ⌘ is held). So a key equivalent adds just its press, ahead of its key-up when that is
    /// already queued. A second, made-up release made ⌘V paste three times (#1638): a ⌘V release
    /// without a press is how an image paste reaches us (see [`crate::shortcuts::clipboard_keys`]).
    pub fn raw_input(&mut self, raw: &mut egui::RawInput) {
        for e in self.backend.drain() {
            match e {
                Event::Key(chord) => {
                    if let Some(press @ egui::Event::Key { key: pressed_key, .. }) = chord.key_event(true) {
                        let at = raw
                            .events
                            .iter()
                            .position(|e| matches!(e, egui::Event::Key { key, pressed: false, .. } if *key == pressed_key))
                            .unwrap_or(raw.events.len());
                        raw.events.insert(at, press);
                    }
                }
                Event::Click(id) => self.clicks.push(id),
            }
            self.dirty = true;
        }
    }
}

/// Run the native menu clicks that arrived since the last frame, like in-window menu clicks.
pub fn run(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(menu) = app.services.native_menu.as_mut() else { return };
    let clicks = std::mem::take(&mut menu.clicks);
    for id in clicks {
        if id == MINIMIZE {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            continue;
        }
        // The rows are disabled while a dialog is open; a click that raced the update is dropped.
        if !crate::menus::modal_allows(app, &id) {
            continue;
        }
        if let Err(e) = crate::menus::invoke(app, ctx, &id, serde_json::json!({})) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
}

/// Bring the native menu up to date when anything it shows may have changed: a command ran, the
/// document or selection changed, or the user clicked or pressed a key. Building the rows walks
/// every command's enabled state, so it doesn't run on frames that only animate or scroll.
pub fn sync(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(menu) = app.services.native_menu.as_ref() else { return };
    let state = state_hash(app, had_input(ctx).then_some(app.frame));
    if menu.state == Some(state) && !menu.dirty {
        return;
    }
    let lang = crate::i18n::current();
    let mut items = crate::menus::menu_items(app);
    for it in &mut items {
        it.enabled &= crate::menus::modal_allows(app, &it.id);
    }
    let layout = photocraft_layout(&items, lang, &app.session.prefs().interface.language);
    let Some(menu) = app.services.native_menu.as_mut() else { return };
    if !menu.logged {
        for c in &layout.clashes {
            log::info!("menu: {} is {} on macOS and {} in PhotoCraft; {}", c.shortcut, c.system, c.command, c.resolution);
        }
        menu.logged = true;
    }
    menu.backend.sync(&layout.bar);
    menu.state = Some(state);
    menu.dirty = false;
}

/// A click or key press this frame: it may have changed state [`state_hash`] doesn't list.
pub(crate) fn had_input(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. } | egui::Event::PointerButton { pressed: false, .. })))
}

/// What the menus' rows depend on, cheaply: commands run, the documents (their names, in tab
/// order, for the Window menu) and the active one's revision,
/// selection, recent files, panels and view state, the language. `input` forces a change on
/// frames with a click or key press, which covers state the hash doesn't list.
pub(crate) fn state_hash(app: &PhotocraftApp, input: Option<u64>) -> u64 {
    let mut h = DefaultHasher::new();
    input.hash(&mut h);
    let s = &app.session;
    s.journal.len().hash(&mut h);
    s.active_index().hash(&mut h);
    for d in s.documents() {
        (d.doc.id.0, &d.doc.name).hash(&mut h);
    }
    s.clipboard.is_some().hash(&mut h);
    // A running job greys the commands that would change its document.
    s.has_jobs().hash(&mut h);
    if let Some(d) = s.active() {
        (d.doc.id.0, d.revision, d.active_layer.map(|l| l.0), d.selected_layers.len(), d.isolated_layers.len()).hash(&mut h);
    }
    let ui = &app.ui;
    (&ui.recent_files, &ui.workspace, ui.palette_open, ui.transform.is_some(), ui.text_edit.is_some(), ui.dialogs.len()).hash(&mut h);
    (app.discard.is_some(), app.camera_raw.is_some(), crate::distort_ui::modal_workspace(app)).hash(&mut h);
    serde_json::to_string(&(&ui.panels, &ui.extras, &ui.view, ui.theme, ui.tool)).unwrap_or_default().hash(&mut h);
    s.prefs().interface.language.hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests;
