//! Drucken & Schneiden: zwei mitgedruckte Passermarken im Entwurf auf dieselben Marken auf dem Material legen.

use crate::app::{ToastKind, VectorLumeApp};
use crate::canvas::ViewTf;
use crate::i18n::{tr, trs};
use crate::theme;
use egui::{Align2, Button, Color32, DragValue, RichText, Stroke, Ui, vec2};
use egui_phosphor::regular as icon;
use vl_cam::{MachineProfile, StartFrom};
use vl_core::{Affine, ObjectId, Point, VectorPath};

/// Was der nächste Klick auf die Arbeitsfläche setzt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    Design(usize),
    Target(usize),
}

pub struct PrintCut {
    pub open: bool,
    /// Passermarken im Entwurf (Dokumentkoordinaten).
    pub design: [Option<Point>; 2],
    /// Dieselben Marken auf dem Material (Dokumentkoordinaten).
    pub target: [Option<Point>; 2],
    /// Gleichmäßig skalieren, damit beide Marken genau treffen (gleicht den Druckmaßstab aus).
    pub scale: bool,
    pub picking: Option<Pick>,
}

impl Default for PrintCut {
    fn default() -> Self {
        Self { open: false, design: [None; 2], target: [None; 2], scale: true, picking: None }
    }
}

impl PrintCut {
    pub fn set(&mut self, pick: Pick, p: Point) {
        match pick {
            Pick::Design(i) => self.design[i] = Some(p),
            Pick::Target(i) => self.target[i] = Some(p),
        }
        self.picking = None;
    }

    pub fn alignment(&self) -> Option<Affine> {
        let [Some(d0), Some(d1)] = self.design else { return None };
        let [Some(t0), Some(t1)] = self.target else { return None };
        Affine::from_point_pairs([d0, d1], [t0, t1], self.scale)
    }
}

/// Passermarke: Kreuz mit Kreis, Mittelpunkt `c`.
fn mark(c: Point) -> VectorPath {
    let mut p = VectorPath::ellipse(c, 3.0, 3.0);
    p.append(VectorPath::line(Point::new(c.x - 5.0, c.y), Point::new(c.x + 5.0, c.y)));
    p.append(VectorPath::line(Point::new(c.x, c.y - 5.0), Point::new(c.x, c.y + 5.0)));
    p
}

impl VectorLumeApp {
    pub fn open_print_cut(&mut self) {
        self.print_cut.open = true;
        self.right_tab = crate::ui_side::RightTab::Jog;
    }

    /// Fügt zwei Passermarken diagonal neben der Auswahl (sonst dem ganzen Entwurf) auf einer Ebene ohne Ausgabe ein.
    fn insert_registration_marks(&mut self) {
        let b = if self.selection.is_empty() { self.doc.bounds() } else { self.selection_bounds() };
        if b.is_empty() {
            self.toast(ToastKind::Info, tr("Zuerst den Entwurf anlegen oder auswählen."));
            return;
        }
        let name = tr("Passermarken");
        let layer = self
            .doc
            .layers
            .iter()
            .position(|l| l.name == name)
            .or_else(|| (0..self.doc.layers.len()).rev().find(|&l| self.doc.objects_on_layer(l).next().is_none()));
        let Some(layer) = layer else {
            self.toast(ToastKind::Error, tr("Keine freie Ebene für die Passermarken."));
            return;
        };
        self.checkpoint();
        let l = &mut self.doc.layers[layer];
        l.name = name.to_string();
        l.settings.output = false;
        l.visible = true;
        let centers = [Point::new(b.min.x - 8.0, b.min.y - 8.0), Point::new(b.max.x + 8.0, b.max.y + 8.0)];
        let ids: Vec<ObjectId> = centers
            .iter()
            .enumerate()
            .map(|(i, c)| self.doc.add_path(trs(&format!("Passermarke {}", i + 1)), layer, mark(*c)))
            .collect();
        self.selection.extend(ids);
        self.print_cut.design = centers.map(Some);
        self.mark_changed();
        self.toast(
            ToastKind::Success,
            tr("Passermarken eingefügt. Sie werden mitgedruckt, aber nicht gelasert (Ebene ohne Ausgabe)."),
        );
    }

    /// Richtet die Auswahl (ohne Auswahl den ganzen Entwurf) an den Marken auf dem Material aus.
    fn apply_print_cut(&mut self) {
        let Some(t) = self.print_cut.alignment() else { return };
        let ids: Vec<ObjectId> = if self.selection.is_empty() {
            self.doc.objects.iter().map(|o| o.id).collect()
        } else {
            self.selection.clone()
        };
        self.checkpoint();
        self.doc.transform_objects(&ids, &t);
        // Die Entwurfsmarken wandern mit dem Entwurf, erneutes Ausrichten ändert dann nichts mehr.
        for p in self.print_cut.design.iter_mut().flatten() {
            *p = t.apply(*p);
        }
        self.mark_changed();
        if self.settings.job.start_from != StartFrom::Absolute {
            self.settings.job.start_from = StartFrom::Absolute;
            self.settings.save();
            self.toast(
                ToastKind::Info,
                tr("Startposition auf „Absolute Koordinaten“ gestellt, sonst stimmt die Ausrichtung nicht."),
            );
        }
        self.toast(ToastKind::Success, tr("Entwurf an den Marken ausgerichtet."));
    }

    /// Fährt den Laser zur Entwurfsmarke – als Startpunkt für die Feinjustierung auf die gedruckte Marke.
    fn jog_to(&mut self, p: Point) {
        let m = self.settings.profile.doc_to_machine(p);
        let axes = format!("X{} Y{}", vl_cam::gcode::fmt_mm(m.x), vl_cam::gcode::fmt_mm(m.y));
        self.send_jog(false, &axes);
    }
}

/// Aufgeschobene Änderung am Programmzustand nach dem Zeichnen des Fensters.
type Action = Box<dyn FnOnce(&mut VectorLumeApp)>;

pub fn show(app: &mut VectorLumeApp, ctx: &egui::Context) {
    if !app.print_cut.open {
        app.print_cut.picking = None;
        return;
    }
    let mut open = true;
    let mut action: Option<Action> = None;
    let profile = app.settings.profile.clone();
    let can_move = app.machine.connected && !app.machine.job_running;
    let head = app.machine.head_position().map(|p| profile.machine_to_doc(p));
    let has_selection = !app.selection.is_empty();
    let selection_center = has_selection.then(|| app.selection_bounds().center());
    egui::Window::new(trs(&format!("{} Drucken & Schneiden", icon::PRINTER)))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(470.0)
        .pivot(Align2::RIGHT_TOP)
        .default_pos(ctx.content_rect().right_top() + vec2(-330.0, 80.0))
        .show(ctx, |ui| {
            let pal = theme::of(ui);
            let pc = &mut app.print_cut;
            ui.label(
                RichText::new(tr(
                    "Für Aufkleber und bedrucktes Material: zwei Passermarken mitdrucken, den Laser im Bereich \
                     „Bewegen“ genau auf beide Marken fahren und die Position übernehmen. Der Entwurf wird dann \
                     gedreht, verschoben und auf Wunsch skaliert, bis er auf dem Druck liegt.",
                ))
                .small()
                .color(pal.text_weak),
            );
            ui.add_space(4.0);
            if ui
                .button(trs(&format!("{} Passermarken einfügen", icon::CROSSHAIR)))
                .on_hover_text(tr("Zwei Kreuze diagonal neben die Auswahl bzw. den Entwurf setzen und als Entwurfsmarken übernehmen"))
                .clicked()
            {
                action = Some(Box::new(|app| app.insert_registration_marks()));
            }
            ui.add_space(6.0);
            egui::Grid::new("print_cut").num_columns(3).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
                ui.label("");
                ui.label(RichText::new(tr("Im Entwurf")).strong());
                ui.label(RichText::new(tr("Auf dem Material")).strong());
                ui.end_row();
                for i in 0..2 {
                    ui.label(RichText::new(trs(&format!("Marke {}", i + 1))).color(mark_color(pal, i)));
                    ui.vertical(|ui| {
                        point_fields(ui, &profile, &mut pc.design[i]);
                        ui.horizontal(|ui| {
                            if ui
                                .add_enabled(has_selection, Button::new(icon::SELECTION).small())
                                .on_hover_text(tr("Mitte der Auswahl übernehmen (die Marke anklicken)"))
                                .on_disabled_hover_text(tr("Erst die Marke im Entwurf auswählen"))
                                .clicked()
                            {
                                pc.design[i] = selection_center;
                            }
                            pick_button(ui, pc, Pick::Design(i), tr("Punkt auf der Arbeitsfläche anklicken (fängt an Mitten und Knoten)"));
                        });
                    });
                    ui.vertical(|ui| {
                        point_fields(ui, &profile, &mut pc.target[i]);
                        ui.horizontal(|ui| {
                            if ui
                                .add_enabled(head.is_some(), Button::new(icon::CROSSHAIR_SIMPLE).small())
                                .on_hover_text(tr("Aktuelle Laserposition übernehmen"))
                                .on_disabled_hover_text(tr("Position unbekannt – ist der Laser verbunden?"))
                                .clicked()
                            {
                                pc.target[i] = head;
                            }
                            if let Some(d) = pc.design[i]
                                && ui
                                    .add_enabled(can_move, Button::new(icon::NAVIGATION_ARROW).small())
                                    .on_hover_text(tr("Laser zur Entwurfsmarke fahren, danach auf die gedruckte Marke nachjustieren"))
                                    .on_disabled_hover_text(tr("Erst mit dem Laser verbinden"))
                                    .clicked()
                            {
                                action = Some(Box::new(move |app| app.jog_to(d)));
                            }
                            pick_button(ui, pc, Pick::Target(i), tr("Punkt auf der Arbeitsfläche anklicken, z. B. im Kamerabild"));
                        });
                    });
                    ui.end_row();
                }
            });
            if pc.picking.is_some() {
                ui.label(RichText::new(tr("Jetzt auf die Arbeitsfläche klicken (Esc bricht ab).")).color(pal.accent));
            }
            ui.add_space(4.0);
            ui.checkbox(&mut pc.scale, tr("Maßstab angleichen"))
                .on_hover_text(tr("Gleicht einen abweichenden Druckmaßstab aus. Ohne Haken wird nur gedreht und verschoben, die Mitte zwischen den Marken trifft genau."));
            ui.add_space(4.0);
            let t = pc.alignment();
            match &t {
                Some(t) => {
                    let s = t.mean_scale();
                    let moved = pc.design[0].zip(pc.design[1]).map(|(a, b)| t.apply(a.lerp(b, 0.5)) - a.lerp(b, 0.5));
                    let moved = moved.unwrap_or(Point::ZERO);
                    ui.label(trs(&format!(
                        "Verschiebung {:.2} mm, Drehung {:.2}°, Maßstab {:.2} %",
                        moved.length(),
                        t.rotation_deg(),
                        s * 100.0
                    )));
                    let marks = pc.design[0].zip(pc.design[1]).map(|(a, b)| a.distance(b)).unwrap_or(0.0);
                    if (s - 1.0).abs() > 0.02 {
                        ui.label(
                            RichText::new(tr(
                                "Der Abstand der Marken weicht über 2 % ab – richtige Marken angefahren? \
                                 Beim Drucken „In Originalgröße“ statt „An Seite anpassen“ wählen.",
                            ))
                            .small()
                            .color(pal.warning),
                        );
                    } else if !pc.scale && let [Some(a), Some(b)] = pc.target {
                        let err = (a.distance(b) - marks).abs() / 2.0;
                        if err > 0.2 {
                            ui.label(
                                RichText::new(trs(&format!("Ohne Skalieren bleibt je Marke {err:.2} mm Abweichung.")))
                                    .small()
                                    .color(pal.warning),
                            );
                        }
                    }
                }
                None => {
                    let hint = if pc.design.iter().chain(&pc.target).all(Option::is_some) {
                        tr("Die beiden Marken dürfen nicht zusammenfallen.")
                    } else {
                        tr("Beide Marken im Entwurf und auf dem Material setzen.")
                    };
                    ui.label(RichText::new(hint).color(pal.text_weak));
                }
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let label = if has_selection { tr("Auswahl ausrichten") } else { tr("Ganzen Entwurf ausrichten") };
                if ui
                    .add_enabled(t.is_some(), Button::new(format!("{} {label}", icon::CHECK)))
                    .on_hover_text(tr("Danach mit „Rahmen fahren“ prüfen und den Job mit absoluten Koordinaten starten"))
                    .clicked()
                {
                    action = Some(Box::new(|app| app.apply_print_cut()));
                }
                if ui.button(trs(&format!("{} Zurücksetzen", icon::ARROW_COUNTER_CLOCKWISE))).clicked() {
                    let scale = pc.scale;
                    *pc = PrintCut { open: true, scale, ..Default::default() };
                }
            });
        });
    if let Some(f) = action {
        f(app);
    }
    if !open {
        app.print_cut.open = false;
        app.print_cut.picking = None;
    }
}

fn mark_color(pal: &theme::Palette, i: usize) -> Color32 {
    if i == 0 { pal.accent } else { pal.success }
}

/// X/Y einer Marke in Maschinenkoordinaten bearbeiten; ungesetzt nur ein Hinweis.
fn point_fields(ui: &mut Ui, profile: &MachineProfile, p: &mut Option<Point>) {
    ui.horizontal(|ui| match p {
        Some(doc) => {
            let mut m = profile.doc_to_machine(*doc);
            let mut changed = false;
            for (axis, v) in [("X", &mut m.x), ("Y", &mut m.y)] {
                changed |= ui
                    .add(DragValue::new(v).speed(0.05).max_decimals(2).prefix(format!("{axis} ")).suffix(" mm"))
                    .changed();
            }
            if changed {
                *doc = profile.machine_to_doc(m);
            }
        }
        None => {
            ui.label(RichText::new(tr("nicht gesetzt")).color(theme::of(ui).text_weak));
        }
    });
}

fn pick_button(ui: &mut Ui, pc: &mut PrintCut, pick: Pick, hint: &str) {
    let active = pc.picking == Some(pick);
    if ui.add(Button::new(icon::CURSOR_CLICK).small().selected(active)).on_hover_text(hint).clicked() {
        pc.picking = if active { None } else { Some(pick) };
    }
}

/// Marken auf der Arbeitsfläche: Entwurf als Ring, Material als Kreuz, verbunden durch eine gestrichelte Linie.
pub fn draw(pc: &PrintCut, painter: &egui::Painter, tf: &ViewTf, pal: &theme::Palette) {
    for i in 0..2 {
        let c = mark_color(pal, i);
        let stroke = Stroke::new(1.5, c);
        let design = pc.design[i].map(|p| tf.to_screen(p));
        let target = pc.target[i].map(|p| tf.to_screen(p));
        if let Some(p) = design {
            painter.circle_stroke(p, 9.0, stroke);
            painter.text(
                p + vec2(10.0, -10.0),
                Align2::LEFT_BOTTOM,
                format!("{}", i + 1),
                egui::FontId::proportional(12.0),
                c,
            );
        }
        if let Some(p) = target {
            for d in [vec2(7.0, 7.0), vec2(7.0, -7.0)] {
                painter.line_segment([p - d, p + d], stroke);
            }
        }
        if let (Some(a), Some(b)) = (design, target)
            && a.distance(b) > 12.0
        {
            painter.extend(egui::Shape::dashed_line(&[a, b], Stroke::new(1.0, c), 5.0, 4.0));
        }
    }
}
