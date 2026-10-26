//! Abgebrochenen Job fortsetzen: Stelle wählen (aus dem Abbruch vorgeschlagen, per Regler oder über die Y-Lage), auf
//! der Arbeitsfläche markieren und nur den Rest senden – mit wiederhergestelltem Zustand und ausgeschaltetem Laser
//! bei der Anfahrt.

use crate::app::{JobPurpose, VectorLumeApp};
use crate::i18n::{tr, trs};
use crate::theme;
use crate::units::UnitField;
use egui::{Align2, Button, Color32, DragValue, Frame, Margin, RichText, Stroke, vec2};
use egui_phosphor::regular as icon;
use std::sync::Arc;
use vl_cam::Job;
use vl_cam::gcode::fmt_mm;
use vl_cam::resume::{ResumePoint, point_before, resume_gcode, resume_points, running_line};
use vl_core::{LineBuf, Point};
use vl_device::{Command, MachineState};

/// Laufender Job, damit bei einem Abbruch die Stelle bekannt ist.
pub struct RunningJob {
    pub job: Arc<Job>,
    /// Bei einer Fortsetzung: erste Zeile im ursprünglichen Job und Länge des vorangestellten Zustandsblocks.
    pub offset: usize,
    pub preamble: usize,
}

impl RunningJob {
    pub fn new(job: Arc<Job>) -> Self {
        Self { job, offset: 0, preamble: 0 }
    }

    /// Bestätigte Zeilen der Sendung → Zeile im ursprünglichen Job.
    pub fn original_line(&self, acked: usize) -> usize {
        self.offset + acked.saturating_sub(self.preamble)
    }
}

/// Abgebrochener Job.
pub struct Aborted {
    pub job: Arc<Job>,
    /// Bestätigte Zeilen, bezogen auf den ursprünglichen Job ohne Kommentare.
    pub acked: usize,
    /// Kopfposition beim Abbruch (G-Code-Koordinaten).
    pub head: Option<Point>,
}

#[derive(Default)]
pub struct ResumeDialog {
    pub open: bool,
    /// Wartet auf einen aus dem Dokument berechneten Job.
    pub building: bool,
    pub state: Option<ResumeState>,
}

pub struct ResumeState {
    pub job: Arc<Job>,
    /// Gesendete Zeilen des vollständigen Jobs (ohne Kommentare).
    pub lines: LineBuf,
    pub points: Vec<ResumePoint>,
    /// Gewählte Stelle (Index in `points`).
    pub pick: usize,
    /// Zeile, die beim Abbruch lief (falls bekannt).
    pub aborted_at: Option<usize>,
    /// Eingabefeld „Y“ (G-Code-Koordinaten).
    pub y: f64,
}

impl ResumeState {
    pub fn new(job: Arc<Job>, aborted: Option<(usize, Option<Point>)>) -> Self {
        let lines = vl_cam::strip_for_streaming(&job.gcode);
        let points = resume_points(&lines);
        let aborted_at = aborted.map(|(acked, head)| running_line(&lines, acked, head));
        let pick = aborted_at.map_or(0, |l| point_before(&points, l));
        let y = points.get(pick).map_or(0.0, |p| p.burn.y);
        Self { job, lines, points, pick, aborted_at, y }
    }

    pub fn point(&self) -> Option<&ResumePoint> {
        self.points.get(self.pick)
    }

    /// Stelle, deren erste gelaserte Linie der Lage `y` am nächsten liegt (bei gleichem Abstand die frühere).
    pub fn pick_by_y(&mut self, y: f64) {
        let mut best: Option<(f64, usize)> = None;
        for (i, p) in self.points.iter().enumerate() {
            let d = (p.burn.y - y).abs();
            if best.is_none_or(|(bd, _)| d < bd - 1e-6) {
                best = Some((d, i));
            }
        }
        if let Some((_, i)) = best {
            self.pick = i;
        }
    }

    /// Anteil des Jobs, der übersprungen wird.
    pub fn done_fraction(&self) -> f64 {
        match self.point() {
            Some(p) if !self.lines.is_empty() => p.line as f64 / self.lines.len() as f64,
            _ => 0.0,
        }
    }

    /// Markierung auf der Arbeitsfläche: wo als Erstes wieder gelasert wird (Dokument).
    pub fn marker(&self) -> Option<Point> {
        self.job.mapping.to_doc(self.point()?.burn)
    }
}

impl VectorLumeApp {
    /// Öffnet „Job fortsetzen“: mit dem zuletzt abgebrochenen Job, sonst mit einem frisch aus dem Dokument berechneten.
    pub fn open_resume(&mut self) {
        if let Some(a) = &self.machine.aborted {
            let st = ResumeState::new(a.job.clone(), Some((a.acked, a.head)));
            self.resume = ResumeDialog { open: true, building: false, state: Some(st) };
        } else {
            self.resume = ResumeDialog { open: true, building: true, state: None };
            self.request_job(JobPurpose::Resume);
        }
    }

    /// Sendet den Rest des Jobs ab der gewählten Stelle.
    pub fn start_resume(&mut self) {
        if !self.ready_to_start() {
            return;
        }
        let Some(st) = self.resume.state.take() else { return };
        let Some(p) = st.point().copied() else { return };
        let lines = resume_gcode(&st.lines, p.line);
        let preamble = lines.len() - (st.lines.len() - p.line);
        let rest = 1.0 - st.done_fraction();
        let job = st.job.clone();
        let name = trs(&format!("{} (Fortsetzung)", self.job_name()));
        self.machine.document_job = false;
        self.machine.job_estimate = job.estimate.seconds * rest;
        self.machine.running = Some(RunningJob { job, offset: p.line, preamble });
        self.device.send(Command::StartJob { name, lines });
        self.resume = ResumeDialog::default();
    }
}

pub fn show(app: &mut VectorLumeApp, ctx: &egui::Context) {
    if !app.resume.open {
        return;
    }
    let mut open = true;
    let mut start = false;
    let mut cancel = false;
    let mut goto = None;
    let alarm = app.machine.status.as_ref().is_some_and(|s| s.state == MachineState::Alarm);
    let connected = app.machine.connected;
    let running = app.machine.job_running;
    egui::Window::new(trs(&format!("{} Job fortsetzen", icon::ARROW_CLOCKWISE)))
        .id(egui::Id::new("resume_window"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(420.0)
        .pivot(Align2::RIGHT_TOP)
        .default_pos(ctx.content_rect().right_top() + vec2(-380.0, 90.0))
        .show(ctx, |ui| {
            let pal = theme::of(ui);
            ui.set_max_width(420.0);
            let Some(st) = &mut app.resume.state else {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(tr("Job wird berechnet …"));
                });
                return;
            };
            ui.label(tr("Lasert einen abgebrochenen Job ab einer gewählten Stelle zu Ende – bereits Gelasertes wird \
                 übersprungen. Fortgesetzt wird immer am Anfang einer Linie bzw. Füllzeile."));
            ui.add_space(6.0);
            if st.points.is_empty() {
                ui.label(
                    RichText::new(tr(
                        "Dieser Job lässt sich nicht fortsetzen (relative Koordinaten, Drehachse mit A-Achse oder \
                     verschobener Nullpunkt).",
                    ))
                    .color(pal.warning),
                );
                ui.add_space(6.0);
                if ui.button(tr("Schließen")).clicked() {
                    cancel = true;
                }
                return;
            }
            match st.aborted_at {
                Some(l) => ui.label(trs(&format!(
                    "Beim Abbruch lief etwa Zeile {} von {} – die Stelle davor ist vorgewählt.",
                    l + 1,
                    st.lines.len()
                ))),
                None => ui.label(
                    RichText::new(tr(
                        "Kein Abbruch gespeichert (z. B. nach einem Neustart): Stelle über Y oder den Regler wählen – \
                         Y dort, wo die Gravur auf dem Werkstück aufhört.",
                    ))
                    .color(pal.text_weak),
                ),
            };
            ui.add_space(6.0);
            let n = st.points.len();
            let before = st.pick;
            let mut y_changed = false;
            egui::Grid::new("resume_grid").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
                ui.label(tr("Stelle"));
                ui.horizontal(|ui| {
                    if ui.add_enabled(st.pick > 0, Button::new(icon::CARET_LEFT)).clicked() {
                        st.pick -= 1;
                    }
                    ui.add(egui::Slider::new(&mut st.pick, 0..=n - 1).show_value(false));
                    if ui.add_enabled(st.pick + 1 < n, Button::new(icon::CARET_RIGHT)).clicked() {
                        st.pick += 1;
                    }
                });
                ui.end_row();
                ui.label(tr("Y (erste Linie)"));
                let resp = ui.add(DragValue::new(&mut st.y).speed(0.1).max_decimals(2).mm()).on_hover_text(tr(
                    "Lage wie in der Statusleiste; gewählt wird die Füllzeile, die am nächsten liegt",
                ));
                if resp.changed() {
                    y_changed = true;
                    let y = st.y;
                    st.pick_by_y(y);
                }
                ui.end_row();
            });
            if st.pick != before && !y_changed {
                st.y = st.points[st.pick].burn.y;
            }
            let p = st.points[st.pick];
            let done = st.done_fraction();
            ui.add_space(4.0);
            ui.label(trs(&format!(
                "Zeile {} von {} · {} % erledigt · Rest ca. {}",
                p.line + 1,
                st.lines.len(),
                (done * 100.0).round(),
                vl_cam::format_duration(st.job.estimate.seconds * (1.0 - done))
            )));
            ui.label(
                RichText::new(trs(&format!(
                    "Es geht weiter bei X {} Y {} mm (grün markiert).",
                    fmt_mm(p.burn.x),
                    fmt_mm(p.burn.y)
                )))
                .color(pal.success),
            );
            ui.add_space(8.0);
            Frame::new().stroke(Stroke::new(1.0, pal.warning)).corner_radius(6.0).inner_margin(Margin::same(8)).show(
                ui,
                |ui| {
                    ui.label(
                        RichText::new(trs(&format!("{} Vorher prüfen", icon::WARNING))).strong().color(pal.warning),
                    );
                    for t in [
                        tr("Nach einem Alarm (z. B. Endschalter) eine Referenzfahrt machen – nur Entsperren reicht \
                            nicht, die Position kann verloren sein."),
                        tr("Das Werkstück nicht verschieben und am Design nichts ändern."),
                        tr("Mit „Laser dorthin fahren“ prüfen, ob die Stelle zur Kante der Gravur passt. Im Zweifel \
                            etwas früher ansetzen: eine Zeile doppelt fällt weniger auf als ein freier Streifen."),
                    ] {
                        ui.label(format!("• {t}"));
                    }
                },
            );
            ui.add_space(8.0);
            let why = if !connected {
                Some(tr("Kein Laser verbunden."))
            } else if running {
                Some(tr("Es läuft bereits ein Job."))
            } else if alarm {
                Some(tr("Der Laser meldet Alarm – zuerst Referenzfahrt."))
            } else {
                None
            };
            ui.horizontal(|ui| {
                let ready = why.is_none();
                let resp = ui.add_enabled(
                    ready,
                    Button::new(
                        RichText::new(trs(&format!("{} Ab hier fortsetzen", icon::PLAY)))
                            .color(Color32::WHITE)
                            .strong(),
                    )
                    .fill(pal.success)
                    .min_size(vec2(160.0, 30.0)),
                );
                let jog = ui
                    .add_enabled(ready, Button::new(tr("Laser dorthin fahren")))
                    .on_hover_text(tr("Fährt mit ausgeschaltetem Laser an den Anfang der ersten Linie"));
                if let Some(why) = why {
                    resp.on_disabled_hover_text(why);
                    jog.on_disabled_hover_text(why);
                } else if resp.clicked() {
                    start = true;
                } else if jog.clicked() {
                    goto = Some(p.burn);
                }
                if ui.button(tr("Abbrechen")).clicked() {
                    cancel = true;
                }
            });
            if alarm {
                ui.label(
                    RichText::new(tr("Der Laser meldet Alarm – zuerst Referenzfahrt (Laser → Referenzfahrt)."))
                        .small()
                        .color(pal.danger),
                );
            }
        });
    if let Some(q) = goto {
        app.send_jog(false, &format!("X{} Y{}", fmt_mm(q.x), fmt_mm(q.y)));
    }
    if start {
        app.start_resume();
    } else if cancel || !open {
        app.resume = ResumeDialog::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vl_cam::{JobSettings, MachineProfile};
    use vl_core::{CutSettings, Document, LayerMode, VectorPath};

    /// Ohne gespeicherten Abbruch: über die Y-Lage die Füllzeile wählen, mit der es weitergeht.
    #[test]
    fn y_picks_the_nearest_fill_line() {
        let profile = MachineProfile { bed_width: 400.0, bed_height: 400.0, ..Default::default() };
        let mut doc = Document::new();
        // Ursprung vorne links: Maschine Y 10 … 30
        doc.add_path("F", 0, VectorPath::rect(5.0, 370.0, 20.0, 20.0, 0.0));
        doc.layers[0].settings = CutSettings { interval: 0.5, ..CutSettings::default_for(LayerMode::Fill) };
        let job = vl_cam::build_job(&doc, &[], &profile, &JobSettings::default()).unwrap();
        let mut st = ResumeState::new(Arc::new(job), None);
        assert_eq!(st.pick, 0);
        st.pick_by_y(20.1);
        let p = *st.point().unwrap();
        assert!((p.burn.y - 20.1).abs() <= 0.26, "{p:?}");
        assert!(st.done_fraction() > 0.3 && st.done_fraction() < 0.7, "{}", st.done_fraction());
        assert!((st.marker().unwrap().y - (400.0 - p.burn.y)).abs() < 1e-9);
    }
}
