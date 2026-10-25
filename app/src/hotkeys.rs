//! Frei belegbare Tastenkürzel: alle Befehle mit ihrer Standardbelegung, eigene Belegungen aus den
//! Einstellungen und das Fenster *Bearbeiten → Tastenkürzel…* zum Ändern.

use crate::app::{Tool, VectorLumeApp};
use crate::i18n::{tr, trs};
use crate::theme;
use egui::{Align2, Button, Key, KeyboardShortcut, Modifiers, RichText, vec2};
use egui_phosphor::regular as icon;
use std::collections::BTreeMap;

const NONE: Modifiers = Modifiers::NONE;
const CTRL: Modifiers = Modifiers::COMMAND;
const CTRL_SHIFT: Modifiers = Modifiers { shift: true, ..Modifiers::COMMAND };
const ALT: Modifiers = Modifiers::ALT;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    File,
    Edit,
    Arrange,
    View,
    Tools,
}

impl Group {
    const ALL: [Group; 5] = [Group::File, Group::Edit, Group::Arrange, Group::View, Group::Tools];

    fn label(self) -> &'static str {
        match self {
            Group::File => tr("Datei"),
            Group::Edit => tr("Bearbeiten"),
            Group::Arrange => tr("Anordnen"),
            Group::View => tr("Ansicht"),
            Group::Tools => tr("Werkzeuge"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    New,
    Open,
    Save,
    SaveAs,
    Import,
    ExportGcode,
    Undo,
    Redo,
    History,
    Cut,
    Copy,
    Paste,
    PasteInPlace,
    Duplicate,
    Delete,
    SelectAll,
    InvertSelection,
    Group,
    Ungroup,
    BreakApart,
    Weld,
    ToFront,
    Forward,
    Backward,
    ToBack,
    CutOrder,
    MirrorH,
    MirrorV,
    RotateCw,
    RotateCcw,
    CenterOnBed,
    ZoomFit,
    Preview,
    SidePanel,
    Manual,
    Tool(Tool),
}

impl Action {
    pub const ALL: [Action; 47] = [
        Action::New,
        Action::Open,
        Action::Save,
        Action::SaveAs,
        Action::Import,
        Action::ExportGcode,
        Action::Undo,
        Action::Redo,
        Action::History,
        Action::Cut,
        Action::Copy,
        Action::Paste,
        Action::PasteInPlace,
        Action::Duplicate,
        Action::Delete,
        Action::SelectAll,
        Action::InvertSelection,
        Action::Group,
        Action::Ungroup,
        Action::BreakApart,
        Action::Weld,
        Action::ToFront,
        Action::Forward,
        Action::Backward,
        Action::ToBack,
        Action::CutOrder,
        Action::MirrorH,
        Action::MirrorV,
        Action::RotateCw,
        Action::RotateCcw,
        Action::CenterOnBed,
        Action::ZoomFit,
        Action::Preview,
        Action::SidePanel,
        Action::Manual,
        Action::Tool(Tool::Select),
        Action::Tool(Tool::Node),
        Action::Tool(Tool::Trim),
        Action::Tool(Tool::Tabs),
        Action::Tool(Tool::Rect),
        Action::Tool(Tool::Ellipse),
        Action::Tool(Tool::Line),
        Action::Tool(Tool::Polygon),
        Action::Tool(Tool::Star),
        Action::Tool(Tool::Text),
        Action::Tool(Tool::Barcode),
        Action::Tool(Tool::Measure),
    ];

    /// Fester Name in den Einstellungen (unabhängig von der Sprache).
    fn id(self) -> &'static str {
        match self {
            Action::New => "new",
            Action::Open => "open",
            Action::Save => "save",
            Action::SaveAs => "save_as",
            Action::Import => "import",
            Action::ExportGcode => "export_gcode",
            Action::Undo => "undo",
            Action::Redo => "redo",
            Action::History => "history",
            Action::Cut => "cut",
            Action::Copy => "copy",
            Action::Paste => "paste",
            Action::PasteInPlace => "paste_in_place",
            Action::Duplicate => "duplicate",
            Action::Delete => "delete",
            Action::SelectAll => "select_all",
            Action::InvertSelection => "invert_selection",
            Action::Group => "group",
            Action::Ungroup => "ungroup",
            Action::BreakApart => "break_apart",
            Action::Weld => "weld",
            Action::ToFront => "to_front",
            Action::Forward => "forward",
            Action::Backward => "backward",
            Action::ToBack => "to_back",
            Action::CutOrder => "cut_order",
            Action::MirrorH => "mirror_h",
            Action::MirrorV => "mirror_v",
            Action::RotateCw => "rotate_cw",
            Action::RotateCcw => "rotate_ccw",
            Action::CenterOnBed => "center_on_bed",
            Action::ZoomFit => "zoom_fit",
            Action::Preview => "preview",
            Action::SidePanel => "side_panel",
            Action::Manual => "manual",
            Action::Tool(t) => match t {
                Tool::Select => "tool_select",
                Tool::Rect => "tool_rect",
                Tool::Ellipse => "tool_ellipse",
                Tool::Line => "tool_line",
                Tool::Polygon => "tool_polygon",
                Tool::Star => "tool_star",
                Tool::Text => "tool_text",
                Tool::Barcode => "tool_barcode",
                Tool::Node => "tool_node",
                Tool::Measure => "tool_measure",
                Tool::Trim => "tool_trim",
                Tool::Tabs => "tool_tabs",
            },
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Action::New => tr("Neu"),
            Action::Open => tr("Öffnen…"),
            Action::Save => tr("Speichern"),
            Action::SaveAs => tr("Speichern unter…"),
            Action::Import => tr("Importieren…"),
            Action::ExportGcode => tr("G-Code speichern…"),
            Action::Undo => tr("Rückgängig"),
            Action::Redo => tr("Wiederholen"),
            Action::History => tr("Bearbeitungsverlauf…"),
            Action::Cut => tr("Ausschneiden"),
            Action::Copy => tr("Kopieren"),
            Action::Paste => tr("Einfügen"),
            Action::PasteInPlace => tr("An Ort und Stelle einfügen"),
            Action::Duplicate => tr("Duplizieren"),
            Action::Delete => tr("Löschen"),
            Action::SelectAll => tr("Alles auswählen"),
            Action::InvertSelection => tr("Auswahl umkehren"),
            Action::Group => tr("Gruppieren"),
            Action::Ungroup => tr("Gruppierung lösen"),
            Action::BreakApart => tr("Zerlegen"),
            Action::Weld => tr("Verschweißen"),
            Action::ToFront => tr("Ganz nach vorne"),
            Action::Forward => tr("Eins nach vorne"),
            Action::Backward => tr("Eins nach hinten"),
            Action::ToBack => tr("Ganz nach hinten"),
            Action::CutOrder => tr("Schnittreihenfolge per Klick…"),
            Action::MirrorH => tr("Horizontal spiegeln"),
            Action::MirrorV => tr("Vertikal spiegeln"),
            Action::RotateCw => tr("90° im Uhrzeigersinn drehen"),
            Action::RotateCcw => tr("90° gegen den Uhrzeigersinn drehen"),
            Action::CenterOnBed => tr("Auf Arbeitsfläche zentrieren"),
            Action::ZoomFit => tr("Arbeitsfläche einpassen"),
            Action::Preview => tr("Vorschau"),
            Action::SidePanel => tr("Seitenleiste ein-/ausblenden"),
            Action::Manual => tr("Handbuch"),
            Action::Tool(t) => t.label(),
        }
    }

    fn group(self) -> Group {
        match self {
            Action::New | Action::Open | Action::Save | Action::SaveAs | Action::Import | Action::ExportGcode => {
                Group::File
            }
            Action::Undo
            | Action::Redo
            | Action::History
            | Action::Cut
            | Action::Copy
            | Action::Paste
            | Action::PasteInPlace
            | Action::Duplicate
            | Action::Delete
            | Action::SelectAll
            | Action::InvertSelection => Group::Edit,
            Action::ZoomFit | Action::Preview | Action::SidePanel | Action::Manual => Group::View,
            Action::Tool(_) => Group::Tools,
            _ => Group::Arrange,
        }
    }

    fn defaults(self) -> &'static [(Modifiers, Key)] {
        match self {
            Action::New => &[(CTRL, Key::N)],
            Action::Open => &[(CTRL, Key::O)],
            Action::Save => &[(CTRL, Key::S)],
            Action::SaveAs => &[(CTRL_SHIFT, Key::S)],
            Action::Import => &[(CTRL, Key::I)],
            Action::ExportGcode => &[(CTRL, Key::E)],
            Action::Undo => &[(CTRL, Key::Z)],
            Action::Redo => &[(CTRL, Key::Y), (CTRL_SHIFT, Key::Z)],
            Action::Cut => &[(CTRL, Key::X)],
            Action::Copy => &[(CTRL, Key::C)],
            Action::Paste => &[(CTRL, Key::V)],
            Action::PasteInPlace => &[(CTRL_SHIFT, Key::V)],
            Action::Duplicate => &[(CTRL, Key::D)],
            Action::Delete => &[(NONE, Key::Delete), (NONE, Key::Backspace)],
            Action::SelectAll => &[(CTRL, Key::A)],
            Action::Group => &[(CTRL, Key::G)],
            Action::Ungroup => &[(CTRL_SHIFT, Key::G)],
            Action::ToFront => &[(CTRL, Key::PageUp)],
            Action::Forward => &[(NONE, Key::PageUp)],
            Action::Backward => &[(NONE, Key::PageDown)],
            Action::ToBack => &[(CTRL, Key::PageDown)],
            Action::ZoomFit => &[(NONE, Key::F), (CTRL, Key::Num0)],
            Action::Preview => &[(ALT, Key::P)],
            Action::Manual => &[(NONE, Key::F1)],
            Action::Tool(Tool::Select) => &[(NONE, Key::V)],
            Action::Tool(Tool::Rect) => &[(NONE, Key::R)],
            Action::Tool(Tool::Ellipse) => &[(NONE, Key::E)],
            Action::Tool(Tool::Line) => &[(NONE, Key::L)],
            Action::Tool(Tool::Text) => &[(NONE, Key::T)],
            Action::Tool(Tool::Barcode) => &[(NONE, Key::Q)],
            Action::Tool(Tool::Node) => &[(NONE, Key::N)],
            Action::Tool(Tool::Measure) => &[(NONE, Key::M)],
            _ => &[],
        }
    }

    /// Ohne Auswahl wirkungslos (die Menüeinträge sind dann gesperrt).
    fn needs_selection(self) -> bool {
        matches!(
            self,
            Action::BreakApart
                | Action::Weld
                | Action::MirrorH
                | Action::MirrorV
                | Action::RotateCw
                | Action::RotateCcw
                | Action::CenterOnBed
        )
    }
}

/// Aktuelle Belegung aller Befehle.
pub struct Keymap {
    /// Je Befehl seine Kürzel in der festgelegten Reihenfolge (das erste steht im Menü).
    by_action: Vec<(Action, Vec<KeyboardShortcut>)>,
    /// Alle Kürzel, solche mit mehr Zusatztasten zuerst: egui übergeht beim Vergleich überzählige
    /// Umschalt- und Alt-Tasten, Strg+Umschalt+S muss also vor Strg+S geprüft werden.
    ordered: Vec<(KeyboardShortcut, Action)>,
}

impl Default for Keymap {
    fn default() -> Self {
        Self::new(&BTreeMap::new())
    }
}

impl Keymap {
    /// `overrides`: eigene Belegungen je Befehl (leere Liste = keine Taste), alles andere bleibt Standard.
    pub fn new(overrides: &BTreeMap<String, Vec<String>>) -> Self {
        let by_action: Vec<_> = Action::ALL
            .iter()
            .map(|&a| {
                let keys: Vec<_> = match overrides.get(a.id()) {
                    Some(specs) => specs.iter().filter_map(|s| parse(s)).collect(),
                    None => a.defaults().iter().map(|&(m, k)| KeyboardShortcut::new(m, k)).collect(),
                };
                (a, keys)
            })
            .collect();
        let mut ordered: Vec<_> = by_action.iter().flat_map(|(a, keys)| keys.iter().map(|&k| (k, *a))).collect();
        ordered.sort_by_key(|(k, _)| std::cmp::Reverse(modifier_count(k.modifiers)));
        Self { by_action, ordered }
    }

    pub fn of(&self, a: Action) -> &[KeyboardShortcut] {
        self.by_action.iter().find(|(b, _)| *b == a).map_or(&[], |(_, keys)| keys)
    }

    /// Erstes Kürzel des Befehls für Menüs, leer ohne Belegung.
    pub fn text(&self, a: Action) -> String {
        self.of(a).first().map(|&k| format(k)).unwrap_or_default()
    }

    /// Hinweis für Knöpfe: „Neu (Strg+N)“ bzw. nur „Neu“ ohne Belegung.
    pub fn hint(&self, label: &str, a: Action) -> String {
        match self.of(a).first() {
            Some(&k) => format!("{label} ({})", format(k)),
            None => label.to_string(),
        }
    }

    fn owner(&self, k: KeyboardShortcut) -> Option<Action> {
        self.ordered.iter().find(|(o, _)| *o == k).map(|(_, a)| *a)
    }

    /// Befehle, deren Kürzel in diesem Bild gedrückt wurden (die Tastenereignisse werden verbraucht).
    pub fn pressed(&self, ctx: &egui::Context) -> Vec<Action> {
        ctx.input_mut(|i| self.ordered.iter().filter(|(k, _)| i.consume_shortcut(k)).map(|(_, a)| *a).collect())
    }
}

fn modifier_count(m: Modifiers) -> usize {
    m.alt as usize + m.shift as usize + m.command as usize
}

/// Nur Strg (bzw. Befehlstaste), Umschalt und Alt zählen.
fn normalize(m: Modifiers) -> Modifiers {
    Modifiers { alt: m.alt, ctrl: false, shift: m.shift, mac_cmd: false, command: m.command || m.ctrl || m.mac_cmd }
}

/// Speicherform, z. B. `Ctrl+Shift+S` oder `PageUp`.
fn spec(k: KeyboardShortcut) -> String {
    let m = k.modifiers;
    let mut parts = Vec::new();
    for (on, name) in [(m.command, "Ctrl"), (m.shift, "Shift"), (m.alt, "Alt")] {
        if on {
            parts.push(name);
        }
    }
    parts.push(k.logical_key.name());
    parts.join("+")
}

fn parse(s: &str) -> Option<KeyboardShortcut> {
    let (mods, key) = s.rsplit_once('+').unwrap_or(("", s));
    let mut m = Modifiers::NONE;
    for part in mods.split('+').filter(|p| !p.is_empty()) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "cmd" => m.command = true,
            "shift" => m.shift = true,
            "alt" => m.alt = true,
            _ => return None,
        }
    }
    let key = Key::from_name(key)?;
    (!reserved(key)).then(|| KeyboardShortcut::new(m, key))
}

/// Fest belegt: Pfeiltasten verschieben, Esc bricht ab, Eingabe übernimmt, Leertaste verschiebt die Ansicht,
/// Tab wechselt das Eingabefeld.
fn reserved(key: Key) -> bool {
    matches!(
        key,
        Key::ArrowUp
            | Key::ArrowDown
            | Key::ArrowLeft
            | Key::ArrowRight
            | Key::Escape
            | Key::Enter
            | Key::Space
            | Key::Tab
    )
}

/// Anzeige in der Sprache der Oberfläche, z. B. „Strg+Umschalt+S“ bzw. „Ctrl+Shift+S“.
pub fn format(k: KeyboardShortcut) -> String {
    let m = k.modifiers;
    let mut parts: Vec<&str> = Vec::new();
    for (on, name) in [(m.command, "Strg"), (m.shift, "Umschalt"), (m.alt, "Alt")] {
        if on {
            parts.push(name);
        }
    }
    let key = match k.logical_key {
        Key::PageUp => "Bild↑",
        Key::PageDown => "Bild↓",
        Key::Delete => "Entf",
        Key::Backspace => "Rücktaste",
        Key::Insert => "Einfg",
        Key::Home => "Pos1",
        Key::End => "Ende",
        other => other.symbol_or_name(),
    };
    parts.push(key);
    crate::i18n::shortcut(&parts.join("+"))
}

pub fn run(app: &mut VectorLumeApp, a: Action) {
    if a.needs_selection() && app.selection.is_empty() {
        return;
    }
    match a {
        Action::New => app.new_document(false),
        Action::Open => app.open_dialog(false, None),
        Action::Save => {
            app.save(false);
        }
        Action::SaveAs => {
            app.save(true);
        }
        Action::Import => app.import_dialog(),
        Action::ExportGcode => app.export_gcode_dialog(),
        Action::Undo => app.undo(),
        Action::Redo => app.redo(),
        Action::History => app.history_window.open = !app.history_window.open,
        Action::Cut => app.cut(),
        Action::Copy => app.copy(),
        Action::Paste => app.paste(),
        Action::PasteInPlace => app.paste_in_place(),
        Action::Duplicate => app.duplicate(),
        Action::Delete => {
            if app.tool == Tool::Node {
                app.delete_selected_node();
            } else {
                app.delete_selection();
            }
        }
        Action::SelectAll => app.select_all(),
        Action::InvertSelection => app.invert_selection(),
        Action::Group => app.group(),
        Action::Ungroup => app.ungroup(),
        Action::BreakApart => app.break_apart(),
        Action::Weld => app.weld_selection(),
        Action::ToFront => app.reorder_selection(vl_core::ZOrder::Front),
        Action::Forward => app.reorder_selection(vl_core::ZOrder::Forward),
        Action::Backward => app.reorder_selection(vl_core::ZOrder::Backward),
        Action::ToBack => app.reorder_selection(vl_core::ZOrder::Back),
        Action::CutOrder => app.start_cut_order(),
        Action::MirrorH => app.mirror(true),
        Action::MirrorV => app.mirror(false),
        Action::RotateCw => app.rotate_selection(90.0),
        Action::RotateCcw => app.rotate_selection(-90.0),
        Action::CenterOnBed => app.center_on_bed(),
        Action::ZoomFit => app.zoom_to_fit(),
        Action::Preview => app.preview.active = !app.preview.active,
        Action::SidePanel => {
            app.settings.show_side_panel = !app.settings.show_side_panel;
            app.settings.save();
        }
        Action::Manual => app.manual.open = !app.manual.open,
        Action::Tool(t) => app.tool = t,
    }
}

#[derive(Default)]
pub struct HotkeyEditor {
    pub open: bool,
    pub filter: String,
    /// Befehl, für den gerade eine Tastenkombination erwartet wird.
    pub recording: Option<Action>,
    message: Option<String>,
}

/// Neue Belegung eines Befehls übernehmen (`None` = Standard) und speichern.
fn assign(app: &mut VectorLumeApp, a: Action, keys: Option<Vec<KeyboardShortcut>>) {
    let defaults: Vec<_> = a.defaults().iter().map(|&(m, k)| KeyboardShortcut::new(m, k)).collect();
    match keys {
        Some(keys) if keys != defaults => {
            app.settings.hotkeys.insert(a.id().to_string(), keys.into_iter().map(spec).collect());
        }
        _ => {
            app.settings.hotkeys.remove(a.id());
        }
    }
    app.keys = Keymap::new(&app.settings.hotkeys);
    app.settings.save();
}

/// Nimmt die nächste gedrückte Tastenkombination für `a` auf.
fn record(app: &mut VectorLumeApp, ctx: &egui::Context, a: Action) {
    let event = ctx.input_mut(|i| {
        let pos = i.events.iter().position(|e| matches!(e, egui::Event::Key { pressed: true, repeat: false, .. }))?;
        Some(i.events.remove(pos))
    });
    let Some(egui::Event::Key { key, modifiers, .. }) = event else { return };
    app.hotkey_editor.recording = None;
    if key == Key::Escape {
        return;
    }
    let k = KeyboardShortcut::new(normalize(modifiers), key);
    if reserved(key) {
        app.hotkey_editor.message = Some(trs(&format!("„{}“ ist fest belegt.", format(k))));
        return;
    }
    let mut message = None;
    if let Some(other) = app.keys.owner(k).filter(|&o| o != a) {
        let rest: Vec<_> = app.keys.of(other).iter().copied().filter(|&o| o != k).collect();
        assign(app, other, Some(rest));
        message = Some(trs(&format!("{} war „{}“ zugewiesen und wurde dort entfernt.", format(k), other.label())));
    }
    assign(app, a, Some(vec![k]));
    app.hotkey_editor.message = message;
}

pub fn show(app: &mut VectorLumeApp, ctx: &egui::Context) {
    if !app.hotkey_editor.open {
        app.hotkey_editor.recording = None;
        return;
    }
    if let Some(a) = app.hotkey_editor.recording {
        record(app, ctx, a);
    }
    let mut open = true;
    // (Befehl, neue Belegung oder None = Standard)
    let mut change: Option<(Action, Option<Vec<KeyboardShortcut>>)> = None;
    let mut reset_all = false;
    let editor = &mut app.hotkey_editor;
    let (keys, overrides) = (&app.keys, &app.settings.hotkeys);
    egui::Window::new(trs(&format!("{} Tastenkürzel", icon::KEYBOARD)))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        // oben links verankert: beim Filtern ändert sich die Höhe, das Fenster soll dabei nicht springen
        .pivot(Align2::LEFT_TOP)
        .default_pos(ctx.content_rect().center() - vec2(300.0, 300.0))
        .show(ctx, |ui| {
            let pal = theme::of(ui);
            ui.label(
                RichText::new(tr(
                    "„Ändern“ klicken und die gewünschte Tastenkombination drücken (Esc bricht ab). Pfeiltasten, Esc, \
                     Eingabe, Leertaste und Tab sind fest belegt.",
                ))
                .small()
                .color(pal.text_weak),
            );
            ui.horizontal(|ui| {
                ui.label(icon::MAGNIFYING_GLASS);
                ui.add(egui::TextEdit::singleline(&mut editor.filter).hint_text(tr("Befehl oder Taste suchen")));
            });
            if let Some(m) = &editor.message {
                ui.label(RichText::new(m).color(pal.warning));
            }
            ui.separator();
            let needle = editor.filter.trim().to_lowercase();
            let matches = |a: Action| {
                needle.is_empty()
                    || a.label().to_lowercase().contains(&needle)
                    || keys.of(a).iter().any(|&k| format(k).to_lowercase().contains(&needle))
            };
            // feste Listenhöhe: sonst wächst das Fenster beim ersten Aufbau auf alle Zeilen
            let list_height = (ctx.content_rect().height() - 300.0).clamp(160.0, 440.0);
            egui::ScrollArea::vertical().max_height(list_height).min_scrolled_height(list_height).show(ui, |ui| {
                // eine gemeinsame Tabelle, damit die Spalten aller Gruppen fluchten
                egui::Grid::new("hotkeys").num_columns(3).min_col_width(80.0).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
                    for group in Group::ALL {
                        let actions: Vec<Action> =
                            Action::ALL.iter().copied().filter(|&a| a.group() == group && matches(a)).collect();
                        if actions.is_empty() {
                            continue;
                        }
                        ui.label(RichText::new(group.label()).strong().color(pal.accent));
                        ui.end_row();
                        for a in actions {
                            ui.label(a.label().trim_end_matches('…'));
                            if editor.recording == Some(a) {
                                ui.label(RichText::new(tr("Taste drücken …")).italics().color(pal.accent));
                            } else {
                                let text: Vec<String> = keys.of(a).iter().map(|&k| format(k)).collect();
                                let text = if text.is_empty() { "–".to_string() } else { text.join(", ") };
                                ui.label(RichText::new(text).monospace());
                            }
                            ui.horizontal(|ui| {
                                if ui.small_button(tr("Ändern")).clicked() {
                                    editor.recording = Some(a);
                                    editor.message = None;
                                }
                                let has = !keys.of(a).is_empty();
                                if ui.add_enabled(has, Button::new(tr("Entfernen")).small()).clicked() {
                                    change = Some((a, Some(Vec::new())));
                                }
                                let custom = overrides.contains_key(a.id());
                                if ui.add_enabled(custom, Button::new(tr("Standard")).small()).clicked() {
                                    change = Some((a, None));
                                }
                            });
                            ui.end_row();
                        }
                    }
                });
            });
            ui.separator();
            if ui.add_enabled(!overrides.is_empty(), Button::new(tr("Alle auf Standard zurücksetzen"))).clicked() {
                reset_all = true;
            }
        });
    if let Some((a, keys)) = change {
        app.hotkey_editor.message = None;
        assign(app, a, keys);
    }
    if reset_all {
        app.settings.hotkeys.clear();
        app.keys = Keymap::new(&app.settings.hotkeys);
        app.settings.save();
        app.hotkey_editor.message = None;
    }
    app.hotkey_editor.open &= open;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_defaults_do_not_collide() {
        let mut ids: Vec<_> = Action::ALL.iter().map(|a| a.id()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), Action::ALL.len());
        let map = Keymap::default();
        let mut all: Vec<_> = map.ordered.iter().map(|(k, _)| spec(*k)).collect();
        let n = all.len();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), n, "doppelt belegt");
        // mehr Zusatztasten zuerst
        let pos = |s: &str| map.ordered.iter().position(|(k, _)| spec(*k) == s).unwrap();
        assert!(pos("Ctrl+Shift+S") < pos("Ctrl+S"));
        assert!(pos("Ctrl+PageUp") < pos("PageUp"));
    }

    #[test]
    fn specs_round_trip_and_overrides_apply() {
        for a in Action::ALL {
            for &(m, k) in a.defaults() {
                let s = KeyboardShortcut::new(m, k);
                assert_eq!(parse(&spec(s)), Some(s), "{}", spec(s));
            }
        }
        assert_eq!(parse("Ctrl+Shift+F12"), Some(KeyboardShortcut::new(CTRL_SHIFT, Key::F12)));
        assert_eq!(parse("Hyper+X"), None);
        assert_eq!(parse("ArrowUp"), None, "fest belegte Taste");
        let mut over = BTreeMap::new();
        over.insert("save".to_string(), vec!["Alt+S".to_string()]);
        over.insert("tool_rect".to_string(), vec![]);
        over.insert("mirror_h".to_string(), vec!["H".to_string(), "kaputt+Q".to_string()]);
        let map = Keymap::new(&over);
        assert_eq!(map.of(Action::Save), [KeyboardShortcut::new(ALT, Key::S)]);
        assert!(map.of(Action::Tool(Tool::Rect)).is_empty());
        assert_eq!(map.text(Action::Tool(Tool::Rect)), "");
        assert_eq!(map.of(Action::MirrorH), [KeyboardShortcut::new(NONE, Key::H)]);
        assert_eq!(map.owner(KeyboardShortcut::new(NONE, Key::H)), Some(Action::MirrorH));
        assert_eq!(map.hint("Neu", Action::New), format!("Neu ({})", map.text(Action::New)));
    }
}
