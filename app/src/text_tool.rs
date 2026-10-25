//! Text: Einfügen und Bearbeiten, Text auf Pfad und variable Texte (Zähler, Datum, Uhrzeit, Datenliste).
//! QR- und Strichcodes laufen über denselben Dialog, ihr Inhalt kann dieselben Platzhalter enthalten.

use crate::app::{ToastKind, VectorLumeApp};
use crate::i18n::{tr, trs};
use crate::theme;
use crate::units::UnitField;
use egui::{Align2, Button, DragValue, RichText, vec2};
use egui_phosphor::regular as icon;
use vl_core::DataList;
use vl_core::text::{Now, PLACEHOLDERS, ROW_PLACEHOLDER, TextData, TextOnPath, TextVars};
use vl_core::{Affine, BarcodeKind, BarcodeSpec, ObjectId, Point, QrLevel, Shape, TextSpec};

/// Zustand des Textdialogs.
pub struct TextDialog {
    pub open: bool,
    pub spec: TextSpec,
    /// Einfügepunkt (linke obere Ecke) für neue Texte.
    pub at: Point,
    pub filter: String,
    pub error: Option<String>,
    /// Bearbeitetes Objekt (None = neuer Text).
    pub edit: Option<ObjectId>,
    pub vars: TextVars,
    pub on_path: Option<TextOnPath>,
    /// Strichcode statt Text.
    pub barcode: Option<BarcodeSpec>,
    /// Zuletzt verwendete Code-Einstellungen für den nächsten neuen Code.
    pub last_barcode: BarcodeSpec,
    /// Inhalt der jeweils anderen Art (Text bzw. Code), damit beide beim Wechsel erhalten bleiben.
    pub other_text: String,
}

impl TextDialog {
    /// Zwischen Text und Strichcode wechseln.
    fn set_barcode_mode(&mut self, on: bool) {
        if self.barcode.is_some() == on {
            return;
        }
        std::mem::swap(&mut self.spec.text, &mut self.other_text);
        match self.barcode.take() {
            Some(b) => self.last_barcode = b,
            None => self.barcode = Some(self.last_barcode.clone()),
        }
    }
}

impl Default for TextDialog {
    fn default() -> Self {
        Self {
            open: false,
            spec: TextSpec::default(),
            at: Point::ZERO,
            filter: String::new(),
            error: None,
            edit: None,
            vars: TextVars::default(),
            on_path: None,
            barcode: None,
            last_barcode: BarcodeSpec::default(),
            other_text: "https://example.com".into(),
        }
    }
}

/// Aktuelle Ortszeit für die Platzhalter.
pub fn now() -> Now {
    use chrono::{Datelike, Timelike};
    let t = chrono::Local::now();
    Now { year: t.year(), month: t.month(), day: t.day(), hour: t.hour(), minute: t.minute() }
}

impl VectorLumeApp {
    /// Neuer Text an `at`; Schrift und Stil vom letzten Text werden beibehalten.
    pub fn open_text_new(&mut self, at: Point) {
        let d = &mut self.text_dialog;
        d.set_barcode_mode(false);
        d.open = true;
        d.at = at;
        d.error = None;
        d.edit = None;
        d.on_path = None;
    }

    /// Neuer QR- oder Strichcode an `at` mit den zuletzt verwendeten Einstellungen.
    pub fn open_barcode_new(&mut self, at: Point) {
        let d = &mut self.text_dialog;
        d.set_barcode_mode(true);
        d.open = true;
        d.at = at;
        d.error = None;
        d.edit = None;
        d.on_path = None;
    }

    /// Text eines vorhandenen Textobjekts bearbeiten.
    pub fn open_text_edit(&mut self, id: ObjectId) -> bool {
        let Some(data) = self.doc.get(id).and_then(|o| o.text.clone()) else { return false };
        let d = &mut self.text_dialog;
        d.open = true;
        d.error = None;
        d.edit = Some(id);
        d.spec = data.spec;
        d.vars = data.vars;
        d.on_path = data.on_path;
        d.barcode = data.barcode;
        true
    }

    /// Text entlang des ausgewählten Pfades (längste Teilkontur).
    pub fn open_text_on_path(&mut self) {
        let line = self
            .selection
            .iter()
            .filter_map(|id| self.doc.get(*id))
            .filter(|o| !o.is_image() && o.text.is_none())
            .flat_map(|o| o.polylines(0.05))
            .max_by(|a, b| a.length().total_cmp(&b.length()));
        let Some(line) = line.filter(|l| l.points.len() >= 2) else {
            self.toast(ToastKind::Info, tr("Bitte zuerst einen Pfad (Linie, Kreis, Kurve …) auswählen."));
            return;
        };
        let d = &mut self.text_dialog;
        d.open = true;
        d.set_barcode_mode(false);
        d.error = None;
        d.edit = None;
        d.spec.align = vl_core::TextAlign::Center;
        d.on_path = Some(TextOnPath { points: line.points, closed: line.closed, ..TextOnPath::default() });
    }

    /// Text aus dem Dialog einfügen bzw. das bearbeitete Objekt ersetzen.
    fn apply_text_dialog(&mut self) {
        let d = &self.text_dialog;
        let mut data = TextData {
            spec: d.spec.clone(),
            vars: d.vars.clone(),
            on_path: if d.barcode.is_some() { None } else { d.on_path.clone() },
            rendered: String::new(),
            barcode: d.barcode.clone(),
        };
        let text = data.expand(now());
        let db = vl_io::system_fonts();
        let path = match vl_io::render_text(&db, &data, &text) {
            Ok(p) => p,
            Err(e) => {
                self.text_dialog.error = Some(e);
                return;
            }
        };
        data.rendered = text;
        if let Some(b) = &data.barcode {
            self.text_dialog.last_barcode = b.clone();
        }
        let name = data.display_name();
        self.checkpoint();
        match self.text_dialog.edit.filter(|id| self.doc.get(*id).is_some()) {
            Some(id) => {
                if let Some(o) = self.doc.get_mut(id) {
                    o.shape = Shape::Path(path);
                    o.name = name;
                    o.text = Some(Box::new(data));
                }
                self.geom.invalidate(id);
                self.selection = vec![id];
            }
            None => {
                let at = self.text_dialog.at;
                let transform = if data.on_path.is_some() { Affine::IDENTITY } else { Affine::translate(at.x, at.y) };
                let id = self.doc.add(name, self.current_layer, transform, Shape::Path(path));
                if let Some(o) = self.doc.get_mut(id) {
                    o.text = Some(Box::new(data));
                }
                self.selection = vec![id];
            }
        }
        self.mark_changed();
        self.tool = crate::app::Tool::Select;
        self.text_dialog.open = false;
    }

    /// Erzeugt variable Texte mit aktuellem Zähler/Datum neu. Liefert, ob sich etwas geändert hat.
    pub fn refresh_variable_texts(&mut self) -> bool {
        if !self.doc.objects.iter().any(|o| o.text.as_ref().is_some_and(|t| t.is_variable())) {
            return false;
        }
        let now = now();
        let db = vl_io::system_fonts();
        let mut changed = Vec::new();
        for o in &mut self.doc.objects {
            let Some(t) = o.text.as_mut().filter(|t| t.is_variable()) else { continue };
            let text = t.expand(now);
            if text == t.rendered {
                continue;
            }
            if let Ok(p) = vl_io::render_text(&db, t, &text) {
                o.shape = Shape::Path(p);
                t.rendered = text;
                changed.push(o.id);
            }
        }
        for id in &changed {
            self.geom.invalidate(*id);
        }
        if !changed.is_empty() {
            self.revision += 1;
        }
        !changed.is_empty()
    }

    /// Nach einem erfolgreich gelaserten Job: Zähler und Datenzeile weiterschalten.
    pub fn advance_text_counters(&mut self) {
        let mut any = false;
        let mut list_done = false;
        for o in &mut self.doc.objects {
            let Some(t) = o.text.as_mut().filter(|t| t.vars.advance_after_job) else { continue };
            if t.uses_counter() {
                t.vars.counter += t.vars.step;
                any = true;
            }
            if t.uses_data()
                && let Some(d) = &mut t.vars.data
            {
                if d.is_last_row() {
                    list_done = true;
                } else {
                    d.row += 1;
                    any = true;
                }
            }
        }
        if any {
            self.refresh_variable_texts();
            self.dirty = true;
        }
        if list_done {
            self.toast(ToastKind::Info, tr("Datenliste: Die letzte Zeile wurde gelasert."));
        }
    }

    /// Ausgewähltes Objekt, falls es ein bearbeitbarer Text ist.
    pub fn selected_text(&self) -> Option<ObjectId> {
        match self.selection[..] {
            [id] => self.doc.get(id).filter(|o| o.text.is_some()).map(|o| o.id),
            _ => None,
        }
    }
}

pub fn show(app: &mut VectorLumeApp, ctx: &egui::Context) {
    if !app.text_dialog.open {
        return;
    }
    let families = app.font_families.lock().unwrap().clone();
    let mut open = true;
    let mut insert = false;
    let mut load_csv = false;
    let editing = app.text_dialog.edit.is_some();
    let is_code = app.text_dialog.barcode.is_some();
    let title = match (editing, is_code, app.text_dialog.on_path.is_some()) {
        (true, true, _) => trs(&format!("{} Code bearbeiten", icon::QR_CODE)),
        (false, true, _) => trs(&format!("{} QR-/Barcode einfügen", icon::QR_CODE)),
        (true, false, _) => trs(&format!("{} Text bearbeiten", icon::TEXT_T)),
        (false, false, true) => trs(&format!("{} Text auf Pfad", icon::PATH)),
        (false, false, false) => trs(&format!("{} Text einfügen", icon::TEXT_T)),
    };
    // Codes werden nur auf Füll-Ebenen als Fläche graviert
    let fill_layer = matches!(
        app.doc.layers.get(app.current_layer).map(|l| l.settings.mode),
        Some(vl_core::LayerMode::Fill | vl_core::LayerMode::Image)
    );
    egui::Window::new(title)
        .id(egui::Id::new("text_dialog"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(400.0)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            let pal = theme::of(ui);
            let d = &mut app.text_dialog;
            if let Some(b) = &mut d.barcode {
                ui.horizontal(|ui| {
                    ui.label(tr("Art"));
                    egui::ComboBox::from_id_salt("barcode_kind").width(160.0).selected_text(tr(b.kind.label())).show_ui(
                        ui,
                        |ui| {
                            for k in BarcodeKind::ALL {
                                ui.selectable_value(&mut b.kind, k, tr(k.label()));
                            }
                        },
                    );
                });
                ui.label(RichText::new(tr(b.kind.hint())).small().color(pal.text_weak));
                ui.add_space(2.0);
            }
            let one_line = d.barcode.as_ref().is_some_and(|b| !b.kind.is_2d());
            ui.add(
                egui::TextEdit::multiline(&mut d.spec.text)
                    .desired_rows(if one_line { 1 } else { 3 })
                    .desired_width(f32::INFINITY)
                    .hint_text(if is_code { tr("Inhalt") } else { tr("Text") }),
            );
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(tr("Platzhalter:")).small().color(pal.text_weak));
                for (p, help) in PLACEHOLDERS {
                    if ui.small_button(p).on_hover_text(help).clicked() {
                        d.spec.text.push_str(p);
                    }
                }
                if ui
                    .small_button(trs(&format!("{} CSV…", icon::TABLE)))
                    .on_hover_text(tr(
                        "Datenliste (CSV) laden: jede Spalte wird ein Platzhalter, z. B. für Namensschilder",
                    ))
                    .clicked()
                {
                    load_csv = true;
                }
                if d.barcode.as_ref().is_some_and(|b| b.kind == BarcodeKind::Qr)
                    && ui
                        .small_button(trs(&format!("{} WLAN", icon::WIFI_HIGH)))
                        .on_hover_text(tr("Vorlage für einen WLAN-Zugang: Netzwerkname und Passwort ersetzen"))
                        .clicked()
                {
                    d.spec.text = "WIFI:T:WPA;S:Netzwerkname;P:Passwort;;".into();
                }
            });
            ui.add_space(6.0);
            if let Some(b) = &mut d.barcode {
                barcode_options(ui, b, &mut d.spec.family, &mut d.filter, families.as_deref());
            } else {
            egui::Grid::new("text_grid").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
                ui.label(tr("Schrift"));
                font_combo(ui, &mut d.spec.family, &mut d.filter, families.as_deref());
                ui.end_row();
                let single_line = vl_io::text::is_single_line(&d.spec.family);
                if single_line {
                    ui.label("");
                    ui.label(
                        RichText::new(tr("Einlinienschrift: offene Striche – im Modus Linie gravieren oder ritzen"))
                            .small()
                            .color(theme::of(ui).text_weak),
                    );
                    ui.end_row();
                }
                ui.label(tr("Stil"));
                ui.horizontal(|ui| {
                    ui.toggle_value(&mut d.spec.bold, RichText::new("F").strong());
                    ui.toggle_value(&mut d.spec.italic, RichText::new("K").italics());
                    ui.toggle_value(&mut d.spec.uppercase, "ABC").on_hover_text(tr("In Großbuchstaben setzen"));
                    ui.add_enabled(!single_line, egui::Checkbox::new(&mut d.spec.weld, tr("Verschweißen"))).on_hover_text(tr(
                        "Überlappende Buchstaben (z. B. Schreibschrift) zu einer Kontur vereinen, damit beim Schneiden \
                         und Füllen nichts doppelt gebrannt wird",
                    ));
                });
                ui.end_row();
                ui.label(tr("Höhe"));
                ui.add(DragValue::new(&mut d.spec.height_mm).speed(0.1).range(0.5..=500.0).mm());
                ui.end_row();
                ui.label(tr("Zeichenabstand"));
                ui.add(DragValue::new(&mut d.spec.letter_spacing).speed(0.05).range(-20.0..=100.0).mm());
                ui.end_row();
                ui.label(tr("Breite"));
                ui.add(DragValue::new(&mut d.spec.width_pct).speed(0.5).range(10.0..=400.0).suffix(" %"))
                    .on_hover_text(tr("Buchstaben schmaler (unter 100 %) oder breiter setzen, die Höhe bleibt"));
                ui.end_row();
                if d.on_path.is_none() {
                    ui.label(tr("Zeilenabstand"));
                    ui.add(DragValue::new(&mut d.spec.line_spacing).speed(0.01).range(0.5..=4.0).fixed_decimals(2));
                    ui.end_row();
                }
                ui.label(tr("Ausrichtung"));
                ui.horizontal(|ui| {
                    use vl_core::TextAlign::*;
                    ui.selectable_value(&mut d.spec.align, Left, icon::TEXT_ALIGN_LEFT);
                    ui.selectable_value(&mut d.spec.align, Center, icon::TEXT_ALIGN_CENTER);
                    ui.selectable_value(&mut d.spec.align, Right, icon::TEXT_ALIGN_RIGHT);
                });
                ui.end_row();
                if let Some(on) = &mut d.on_path {
                    ui.label(tr("Versatz entlang"));
                    ui.add(DragValue::new(&mut on.offset).speed(0.2).range(-10000.0..=10000.0).mm());
                    ui.end_row();
                    ui.label(tr("Abstand zum Pfad"));
                    ui.add(DragValue::new(&mut on.distance).speed(0.05).range(-500.0..=500.0).mm());
                    ui.end_row();
                    ui.label("");
                    ui.checkbox(&mut on.reverse, tr("Richtung umkehren (andere Seite)"));
                    ui.end_row();
                }
            });
            }
            let data = TextData { spec: d.spec.clone(), vars: d.vars.clone(), ..Default::default() };
            if data.uses_counter() {
                ui.add_space(4.0);
                ui.label(RichText::new(tr("Fortlaufende Nummer {n}")).strong());
                egui::Grid::new("text_counter").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
                    ui.label(tr("Nächster Wert"));
                    ui.horizontal(|ui| {
                        ui.add(DragValue::new(&mut d.vars.counter).speed(1.0));
                        ui.label(tr("Schritt"));
                        ui.add(DragValue::new(&mut d.vars.step).speed(1.0).range(-1000..=1000));
                        ui.label(tr("Stellen"));
                        ui.add(DragValue::new(&mut d.vars.digits).range(1..=12));
                    });
                    ui.end_row();
                    ui.label("");
                    ui.checkbox(&mut d.vars.advance_after_job, tr("Nach jedem fertigen Job weiterzählen"));
                    ui.end_row();
                });
            }
            let mut remove_list = false;
            if let Some(list) = &mut d.vars.data {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(tr("Datenliste")).strong());
                    ui.label(
                        RichText::new(trs(&format!("{} · {} Zeilen", list.source, list.rows.len())))
                            .small()
                            .color(pal.text_weak),
                    );
                    if ui.small_button(icon::X).on_hover_text(tr("Datenliste entfernen")).clicked() {
                        remove_list = true;
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(tr("Spalten:")).small().color(pal.text_weak));
                    for h in &list.headers {
                        let p = DataList::placeholder(h);
                        if ui.small_button(&p).clicked() {
                            d.spec.text.push_str(&p);
                        }
                    }
                    if ui.small_button(ROW_PLACEHOLDER).on_hover_text(tr("Zeilennummer")).clicked() {
                        d.spec.text.push_str(ROW_PLACEHOLDER);
                    }
                });
                ui.horizontal(|ui| {
                    let n = list.rows.len();
                    ui.label(tr("Zeile"));
                    if ui.add_enabled(list.row > 0, Button::new(icon::CARET_LEFT).small()).clicked() {
                        list.row -= 1;
                    }
                    let mut r = list.row.min(n - 1) + 1;
                    if ui.add(DragValue::new(&mut r).range(1..=n)).changed() {
                        list.row = r - 1;
                    }
                    if ui.add_enabled(list.row + 1 < n, Button::new(icon::CARET_RIGHT).small()).clicked() {
                        list.row += 1;
                    }
                    ui.label(trs(&format!("von {n}")));
                });
                if !data.uses_counter() {
                    ui.checkbox(&mut d.vars.advance_after_job, tr("Nach jedem fertigen Job zur nächsten Zeile"));
                }
            }
            if remove_list {
                d.vars.data = None;
            }
            if data.is_variable() {
                ui.label(
                    RichText::new(trs(&format!("Ergebnis jetzt: {}", data.expand(now()).lines().next().unwrap_or(""))))
                        .small()
                        .color(pal.text_weak),
                );
            }
            // Codes: Größe bzw. Fehler sofort anzeigen
            let mut code_ok = true;
            if let Some(b) = &d.barcode {
                match vl_io::barcode_info(b, &data.expand(now())) {
                    Ok(info) => {
                        let (w, h) = info.size_mm;
                        let text = if b.kind.is_2d() {
                            trs(&format!("{} × {} Module · {} × {}", info.modules.0, info.modules.1, crate::units::num(w, 1), crate::units::len(h, 1)))
                        } else {
                            trs(&format!("{} Module · {} × {}", info.modules.0, crate::units::num(w, 1), crate::units::len(h, 1)))
                        };
                        ui.label(RichText::new(text).small().color(pal.text_weak));
                    }
                    Err(e) => {
                        code_ok = false;
                        ui.label(RichText::new(trs(&e)).color(pal.danger));
                    }
                }
                if !fill_layer {
                    ui.label(
                        RichText::new(tr(
                            "Tipp: Codes auf einer Ebene im Modus „Füllen“ gravieren, sonst werden nur die Umrisse abgefahren.",
                        ))
                        .small()
                        .color(pal.warning),
                    );
                }
            }
            if let Some(e) = &d.error {
                ui.label(RichText::new(trs(e)).color(pal.danger));
            }
            ui.add_space(6.0);
            let needs_font = d.barcode.as_ref().is_none_or(|b| !b.kind.is_2d() && b.show_text);
            let ready = (families.is_some() || !needs_font) && code_ok && !d.spec.text.trim().is_empty();
            let label = if editing {
                trs(&format!("{} Übernehmen", icon::CHECK))
            } else {
                trs(&format!("{} Einfügen", icon::PLUS))
            };
            if ui.add_enabled(ready, Button::new(label).min_size(vec2(120.0, 30.0))).clicked() {
                insert = true;
            }
        });
    if load_csv {
        load_data_list(app);
    }
    if insert {
        app.apply_text_dialog();
    } else if !open {
        app.text_dialog.open = false;
    }
}

/// Auswahl der Schriftfamilie mit Suchfeld.
fn font_combo(ui: &mut egui::Ui, family: &mut String, filter: &mut String, families: Option<&[String]>) {
    match families {
        Some(list) => {
            egui::ComboBox::from_id_salt("font").width(220.0).selected_text(family.as_str()).height(320.0).show_ui(
                ui,
                |ui| {
                    ui.add(egui::TextEdit::singleline(filter).hint_text(tr("Suchen…")));
                    let f = filter.to_lowercase();
                    for name in list.iter().filter(|n| f.is_empty() || n.to_lowercase().contains(&f)) {
                        let label = if vl_io::text::is_single_line(name) {
                            format!("{name}  ·  {}", tr("Einlinie"))
                        } else {
                            name.clone()
                        };
                        ui.selectable_value(family, name.clone(), label);
                    }
                },
            );
        }
        None => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(tr("Schriften werden geladen…"));
            });
        }
    }
}

/// Einstellungen eines QR- oder Strichcodes.
fn barcode_options(
    ui: &mut egui::Ui,
    b: &mut BarcodeSpec,
    family: &mut String,
    filter: &mut String,
    families: Option<&[String]>,
) {
    egui::Grid::new("barcode_grid").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
        ui.label(tr("Modulgröße")).on_hover_text(tr("Breite eines Punktes bzw. des schmalsten Strichs"));
        ui.add(DragValue::new(&mut b.module_mm).speed(0.01).range(0.05..=10.0).fixed_decimals(2).mm());
        ui.end_row();
        if b.kind == BarcodeKind::Qr {
            ui.label(tr("Fehlerkorrektur")).on_hover_text(tr(
                "Höhere Stufen bleiben auch mit Kratzern oder Brandflecken lesbar, der Code wird aber größer.",
            ));
            egui::ComboBox::from_id_salt("qr_level").selected_text(b.level.label()).show_ui(ui, |ui| {
                for l in QrLevel::ALL {
                    ui.selectable_value(&mut b.level, l, l.label());
                }
            });
            ui.end_row();
        }
        if !b.kind.is_2d() {
            ui.label(tr("Strichhöhe"));
            ui.add(DragValue::new(&mut b.height_mm).speed(0.1).range(1.0..=300.0).mm());
            ui.end_row();
            ui.label(tr("Klartext"));
            ui.checkbox(&mut b.show_text, tr("unter den Strichen"));
            ui.end_row();
            if b.show_text {
                ui.label(tr("Schrift"));
                font_combo(ui, family, filter, families);
                ui.end_row();
            }
        }
        let help = tr(
            "Gravierte Flächen um diesen Betrag schmaler machen, weil der Laserpunkt über die Kontur hinaus brennt. Richtwert: Breite des Laserpunkts (etwa 0,05–0,1 mm).",
        );
        ui.label(tr("Strichbreiten-Ausgleich")).on_hover_text(help);
        ui.add(DragValue::new(&mut b.shrink_mm).speed(0.005).range(0.0..=1.0).fixed_decimals(3).mm())
            .on_hover_text(help);
        ui.end_row();
        ui.label("");
        ui.checkbox(&mut b.invert, tr("Invertieren (helle Flächen gravieren)")).on_hover_text(tr(
            "Für Material, das beim Lasern hell wird, z. B. eloxiertes Aluminium, Schiefer oder dunkel lackierte Flächen. Die Ruhezone um den Code wird mitgraviert.",
        ));
        ui.end_row();
    });
}

/// CSV-Datei als Datenliste für den Text im Dialog laden.
fn load_data_list(app: &mut VectorLumeApp) {
    let Some(path) = crate::system::FileDialog::new().add_filter(tr("CSV-Datei"), &["csv", "txt", "tsv"]).pick_file()
    else {
        return;
    };
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let result = std::fs::read(&path).map_err(|e| e.to_string()).and_then(|b| vl_io::parse_csv(&b, &name));
    let d = &mut app.text_dialog;
    match result {
        Ok(list) => {
            // Ohne Spalten-Platzhalter im Text die erste Spalte vorschlagen
            if !list.headers.iter().any(|h| d.spec.text.contains(&DataList::placeholder(h))) {
                let first = DataList::placeholder(&list.headers[0]);
                if d.spec.text.trim().is_empty() || d.spec.text == TextSpec::default().text {
                    d.spec.text = first;
                } else {
                    d.spec.text.push_str(&first);
                }
            }
            d.vars.data = Some(list);
            d.error = None;
        }
        Err(e) => d.error = Some(format!("{name}: {e}")),
    }
}

/// Text in einen reinen Pfad umwandeln (Textdaten verwerfen, z. B. um Knoten zu bearbeiten).
pub fn text_to_curves(app: &mut VectorLumeApp) {
    let ids: Vec<ObjectId> =
        app.selection.iter().copied().filter(|id| app.doc.get(*id).is_some_and(|o| o.text.is_some())).collect();
    if ids.is_empty() {
        return;
    }
    app.checkpoint();
    for id in ids {
        if let Some(o) = app.doc.get_mut(id) {
            o.text = None;
            o.name = o.name.replacen(tr("Text"), tr("Kurven"), 1);
        }
    }
    app.mark_changed();
}
