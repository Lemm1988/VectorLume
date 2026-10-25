//! Motiv-Bibliothek: häufig gebrauchte Formen als kleine Projektdateien im Einstellungsordner (`library`).

use crate::app::{ToastKind, VectorLumeApp};
use crate::i18n::{tr, trs};
use crate::theme;
use egui::{Button, Pos2, RichText, Sense, Stroke, Ui, vec2};
use egui_phosphor::regular as icon;
use std::path::{Path, PathBuf};
use vl_core::{Document, Object, Point, Polyline, Rect};

pub struct LibraryItem {
    pub path: PathBuf,
    pub name: String,
    /// Vereinfachte Umrisse für die Vorschau (Dokumentkoordinaten des Motivs).
    preview: Vec<Polyline>,
    bounds: Rect,
}

#[derive(Default)]
pub struct Library {
    /// `None` = noch nicht (oder neu) zu laden.
    items: Option<Vec<LibraryItem>>,
    pub filter: String,
    /// Name für das Ablegen der Auswahl (Eingabe offen).
    adding: Option<String>,
    renaming: Option<(PathBuf, String)>,
}

pub fn dir() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("library"))
}

/// Dateiname aus einem frei eingegebenen Namen (ohne in Windows verbotene Zeichen).
pub fn sanitize(name: &str) -> String {
    let clean: String = name
        .trim()
        .chars()
        .map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() { '_' } else { c })
        .take(60)
        .collect();
    let clean = clean.trim().trim_end_matches('.').to_string();
    if clean.is_empty() { "Motiv".into() } else { clean }
}

/// Freier Pfad `name.vlume`, `name 2.vlume`, … im Ordner.
fn free_path(dir: &Path, name: &str) -> PathBuf {
    let mut path = dir.join(format!("{name}.vlume"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{name} {n}.vlume"));
        n += 1;
    }
    path
}

fn load_item(path: &Path) -> Option<LibraryItem> {
    let doc = vl_core::project::load_project(path).ok()?;
    let bounds = doc.bounds();
    if bounds.is_empty() {
        return None;
    }
    let tol = (bounds.width().max(bounds.height()) / 150.0).max(0.01);
    let preview = doc.objects.iter().flat_map(|o| o.polylines(tol)).collect();
    let name = path.file_stem()?.to_string_lossy().into_owned();
    Some(LibraryItem { path: path.to_path_buf(), name, preview, bounds })
}

impl Library {
    fn ensure_loaded(&mut self) {
        if self.items.is_some() {
            return;
        }
        let mut items: Vec<LibraryItem> = dir()
            .and_then(|d| std::fs::read_dir(d).ok())
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("vlume")))
            .filter_map(|p| load_item(&p))
            .collect();
        items.sort_by_key(|i| i.name.to_lowercase());
        self.items = Some(items);
    }

    pub fn invalidate(&mut self) {
        self.items = None;
    }
}

impl VectorLumeApp {
    /// Legt die Auswahl samt Ebenen-Einstellungen als Motiv ab.
    fn add_selection_to_library(&mut self, name: &str) {
        let Some(dir) = dir() else { return };
        let objects: Vec<Object> = self.selected_objects();
        if objects.is_empty() {
            return;
        }
        let mut item = Document::new();
        item.layers = self.doc.layers.clone();
        item.objects = objects;
        let path = free_path(&dir, &sanitize(name));
        let result = std::fs::create_dir_all(&dir)
            .map_err(|e| e.to_string())
            .and_then(|_| vl_core::project::save_project(&item, &path));
        match result {
            Ok(()) => {
                self.library.invalidate();
                let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                self.toast(ToastKind::Success, trs(&format!("„{name}“ in der Bibliothek abgelegt.")));
            }
            Err(e) => self.toast(ToastKind::Error, trs(&format!("Ablegen fehlgeschlagen: {e}"))),
        }
    }

    /// Fügt ein Motiv in der Mitte der Ansicht ein. Unbenutzte Ebenen übernehmen die Einstellungen des Motivs.
    fn insert_library_item(&mut self, path: &Path) {
        let item = match vl_core::project::load_project(path) {
            Ok(d) => d,
            Err(e) => {
                self.toast(ToastKind::Error, trs(&e));
                return;
            }
        };
        let b = item.bounds();
        if b.is_empty() {
            return;
        }
        let target = self.view.visible.map(|r| r.center()).unwrap_or_else(|| self.settings.profile.bed_rect().center());
        self.checkpoint();
        for layer in item.used_layers() {
            if layer < self.doc.layers.len() && self.doc.objects_on_layer(layer).next().is_none() {
                let index = self.doc.layers[layer].index;
                self.doc.layers[layer] = item.layers[layer].clone();
                self.doc.layers[layer].index = index;
            }
        }
        let offset = target - b.center();
        self.selection = self.doc.duplicate(&item.objects, Point::new(offset.x, offset.y));
        if let Some(o) = self.selection.first().and_then(|id| self.doc.get(*id)) {
            self.current_layer = o.layer;
        }
        self.mark_changed();
    }
}

/// Reiter „Bibliothek“ im rechten Bereich.
pub fn library_ui(app: &mut VectorLumeApp, ui: &mut Ui) {
    let pal = theme::of(ui);
    app.library.ensure_loaded();
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut app.library.filter)
                .hint_text(trs(&format!("{} Suchen…", icon::MAGNIFYING_GLASS)))
                .desired_width(150.0),
        );
        let has = !app.selection.is_empty();
        if ui
            .add_enabled(has, Button::new(trs(&format!("{} Auswahl", icon::PLUS))))
            .on_hover_text(tr("Die ausgewählten Formen als Motiv ablegen"))
            .on_disabled_hover_text(tr("Erst Formen auswählen"))
            .clicked()
        {
            let name = app.selected_objects().first().map(|o| o.name.clone()).unwrap_or_default();
            app.library.adding = Some(name);
        }
        if ui.button(icon::FOLDER_OPEN).on_hover_text(tr("Bibliotheksordner öffnen")).clicked()
            && let Some(d) = dir()
        {
            let _ = std::fs::create_dir_all(&d);
            let _ = std::process::Command::new("explorer").arg(d).spawn();
            app.library.invalidate();
        }
    });
    if let Some(name) = &mut app.library.adding {
        let mut save = false;
        let mut cancel = false;
        ui.horizontal(|ui| {
            let resp = ui.add(egui::TextEdit::singleline(name).hint_text(tr("Name des Motivs")).desired_width(150.0));
            if ui.memory(|m| m.focused().is_none()) {
                resp.request_focus();
            }
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                save = true;
            }
            if ui.button(trs(&format!("{} Ablegen", icon::CHECK))).clicked() {
                save = true;
            }
            if ui.button(icon::X).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                cancel = true;
            }
        });
        if save {
            let name = name.clone();
            app.library.adding = None;
            app.add_selection_to_library(&name);
        } else if cancel {
            app.library.adding = None;
        }
    }
    ui.add_space(6.0);

    let filter = app.library.filter.trim().to_lowercase();
    let mut insert: Option<PathBuf> = None;
    let mut delete: Option<PathBuf> = None;
    let mut rename_done: Option<(PathBuf, String)> = None;
    let items = app.library.items.as_deref().unwrap_or_default();
    if items.is_empty() {
        ui.label(
            RichText::new(tr("Noch keine Motive. Formen auswählen und mit „+ Auswahl“ ablegen – sie stehen dann in \
                              jedem Projekt mit einem Klick bereit."))
            .small()
            .color(pal.text_weak),
        );
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        for item in items.iter().filter(|i| filter.is_empty() || i.name.to_lowercase().contains(&filter)) {
            let (rect, resp) = ui.allocate_exact_size(vec2(86.0, 104.0), Sense::click());
            let hovered = resp.hovered();
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 6.0, if hovered { pal.widget_hover } else { pal.panel_alt });
            let thumb = egui::Rect::from_min_size(rect.min + vec2(7.0, 7.0), vec2(72.0, 72.0));
            draw_preview(&painter, thumb, item, Stroke::new(1.0, pal.text));
            let name = if item.name.chars().count() > 13 {
                format!("{}…", item.name.chars().take(12).collect::<String>())
            } else {
                item.name.clone()
            };
            painter.text(
                Pos2::new(rect.center().x, rect.max.y - 10.0),
                egui::Align2::CENTER_CENTER,
                name,
                egui::FontId::proportional(11.0),
                pal.text,
            );
            let resp = resp.on_hover_text(format!(
                "{} – {} × {}\n{}",
                item.name,
                crate::units::num(item.bounds.width(), 1),
                crate::units::len(item.bounds.height(), 1),
                tr("Klicken zum Einfügen, Rechtsklick für mehr")
            ));
            if resp.clicked() {
                insert = Some(item.path.clone());
            }
            resp.context_menu(|ui| {
                if ui.button(trs(&format!("{} Umbenennen", icon::PENCIL_SIMPLE))).clicked() {
                    rename_done = Some((item.path.clone(), String::new()));
                    ui.close();
                }
                ui.menu_button(trs(&format!("{} Löschen", icon::TRASH)), |ui| {
                    if ui.button(RichText::new(tr("Endgültig löschen")).color(pal.danger)).clicked() {
                        delete = Some(item.path.clone());
                        ui.close();
                    }
                });
            });
        }
    });
    // Umbenennen beginnen bzw. Eingabe zeigen
    if let Some((path, _)) = rename_done.take() {
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        app.library.renaming = Some((path, name));
    }
    if let Some((path, name)) = &mut app.library.renaming {
        let mut done = None;
        egui::Window::new(trs(&format!("{} Motiv umbenennen", icon::PENCIL_SIMPLE)))
            .id(egui::Id::new("library_rename"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ui.ctx(), |ui| {
                let resp = ui.text_edit_singleline(name);
                ui.horizontal(|ui| {
                    if ui.button(trs(&format!("{} Speichern", icon::CHECK))).clicked()
                        || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                    {
                        done = Some(true);
                    }
                    if ui.button(tr("Abbrechen")).clicked() {
                        done = Some(false);
                    }
                });
            });
        match done {
            Some(true) => {
                let target = path.parent().map(|d| free_path(d, &sanitize(name)));
                if let Some(t) = target
                    && let Err(e) = std::fs::rename(&*path, t)
                {
                    let msg = trs(&format!("Umbenennen fehlgeschlagen: {e}"));
                    app.library.renaming = None;
                    app.toast(ToastKind::Error, msg);
                } else {
                    app.library.renaming = None;
                }
                app.library.invalidate();
            }
            Some(false) => app.library.renaming = None,
            None => {}
        }
    }
    if let Some(path) = insert {
        app.insert_library_item(&path);
    }
    if let Some(path) = delete {
        if let Err(e) = std::fs::remove_file(&path) {
            app.toast(ToastKind::Error, trs(&format!("Löschen fehlgeschlagen: {e}")));
        }
        app.library.invalidate();
    }
}

/// Umrisse eines Motivs eingepasst in `rect`.
fn draw_preview(painter: &egui::Painter, rect: egui::Rect, item: &LibraryItem, stroke: Stroke) {
    let b = item.bounds;
    let scale = (rect.width() as f64 / b.width().max(1e-6)).min(rect.height() as f64 / b.height().max(1e-6));
    let (w, h) = (b.width() * scale, b.height() * scale);
    let origin = rect.center() - vec2(w as f32 / 2.0, h as f32 / 2.0);
    let map =
        |p: Point| Pos2::new(origin.x + ((p.x - b.min.x) * scale) as f32, origin.y + ((p.y - b.min.y) * scale) as f32);
    for poly in &item.preview {
        let pts: Vec<Pos2> = poly.points.iter().map(|p| map(*p)).collect();
        if pts.len() < 2 {
            continue;
        }
        painter.add(if poly.closed { egui::Shape::closed_line(pts, stroke) } else { egui::Shape::line(pts, stroke) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_become_safe_file_names() {
        assert_eq!(sanitize("  Herz <groß>?  "), "Herz _groß__");
        assert_eq!(sanitize("a/b\\c"), "a_b_c");
        assert_eq!(sanitize("   "), "Motiv");
        assert_eq!(sanitize("Ende..."), "Ende");
        assert_eq!(sanitize(&"x".repeat(100)).len(), 60);
    }
}
