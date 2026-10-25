//! Eingebautes Handbuch (Hilfe → Handbuch, F1): `docs/Handbuch.md` wird ins Programm eingebettet und mit einem
//! kleinen Markdown-Leser angezeigt – Überschriften, Absätze, Listen, Hinweise sowie **fett**, *kursiv* und `Code`.
//! Links stehen das Inhaltsverzeichnis und eine Suche, die nur passende Kapitel zeigt.

use crate::app::VectorLumeApp;
use crate::i18n::{Language, tr, trs};
use crate::theme;
use egui::text::{LayoutJob, TextFormat};
use egui::{Align, Align2, FontFamily, FontId, Label, RichText, TextEdit, vec2};
use egui_phosphor::regular as icon;

const DE: &str = include_str!("../../docs/Handbuch.md");
const EN: &str = include_str!("../../docs/Manual.en.md");

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Title(String),
    Heading(String),
    Subheading(String),
    Paragraph(String),
    Bullet(String),
    Numbered(String, String),
    Note(String),
}

impl Block {
    fn text(&self) -> &str {
        match self {
            Block::Title(t)
            | Block::Heading(t)
            | Block::Subheading(t)
            | Block::Paragraph(t)
            | Block::Bullet(t)
            | Block::Note(t)
            | Block::Numbered(_, t) => t,
        }
    }
}

/// Zerlegt den Text in Blöcke; aufeinanderfolgende Zeilen eines Absatzes werden zusammengefügt.
pub fn parse(md: &str) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    let mut open = false;
    for raw in md.lines() {
        let line = raw.trim();
        if line.is_empty() {
            open = false;
            continue;
        }
        let numbered = line
            .split_once(". ")
            .filter(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
            .map(|(n, t)| Block::Numbered(n.to_string(), t.to_string()));
        let block = if let Some(t) = line.strip_prefix("### ") {
            Block::Subheading(t.into())
        } else if let Some(t) = line.strip_prefix("## ") {
            Block::Heading(t.into())
        } else if let Some(t) = line.strip_prefix("# ") {
            Block::Title(t.into())
        } else if let Some(t) = line.strip_prefix("- ") {
            Block::Bullet(t.into())
        } else if let Some(b) = numbered {
            b
        } else if let Some(t) = line.strip_prefix('>') {
            if open && let Some(Block::Note(prev)) = out.last_mut() {
                prev.push(' ');
                prev.push_str(t.trim());
                continue;
            }
            Block::Note(t.trim().into())
        } else {
            // Fortsetzung eines Absatzes oder Listenpunkts
            if open
                && let Some(Block::Paragraph(prev) | Block::Bullet(prev) | Block::Numbered(_, prev)) = out.last_mut()
            {
                prev.push(' ');
                prev.push_str(line);
                continue;
            }
            Block::Paragraph(line.into())
        };
        open = !matches!(block, Block::Title(_) | Block::Heading(_) | Block::Subheading(_));
        out.push(block);
    }
    out
}

/// Kapitel: Einleitung (alles vor der ersten `##`-Überschrift) und je eines pro `##`.
pub fn chapters(blocks: &[Block]) -> Vec<(String, &[Block])> {
    let mut starts: Vec<usize> =
        blocks.iter().enumerate().filter(|(_, b)| matches!(b, Block::Heading(_))).map(|(i, _)| i).collect();
    starts.insert(0, 0);
    starts.dedup();
    let mut out = Vec::new();
    for (k, &s) in starts.iter().enumerate() {
        let e = starts.get(k + 1).copied().unwrap_or(blocks.len());
        if s == e {
            continue;
        }
        let title = match &blocks[s] {
            Block::Heading(t) | Block::Title(t) => t.clone(),
            _ => String::new(),
        };
        out.push((title, &blocks[s..e]));
    }
    out
}

/// Inline-Auszeichnung: `**fett**`, `*kursiv*`, `` `Code` ``.
pub fn spans(text: &str) -> Vec<(String, bool, bool, bool)> {
    let (mut bold, mut italic, mut code) = (false, false, false);
    let mut out: Vec<(String, bool, bool, bool)> = Vec::new();
    let mut cur = String::new();
    let mut chars = text.chars().peekable();
    let flush = |cur: &mut String, out: &mut Vec<(String, bool, bool, bool)>, b, i, c| {
        if !cur.is_empty() {
            out.push((std::mem::take(cur), b, i, c));
        }
    };
    while let Some(ch) = chars.next() {
        match ch {
            '`' => {
                flush(&mut cur, &mut out, bold, italic, code);
                code = !code;
            }
            '*' if !code && chars.peek() == Some(&'*') => {
                chars.next();
                flush(&mut cur, &mut out, bold, italic, code);
                bold = !bold;
            }
            '*' if !code => {
                flush(&mut cur, &mut out, bold, italic, code);
                italic = !italic;
            }
            _ => cur.push(ch),
        }
    }
    flush(&mut cur, &mut out, bold, italic, code);
    out
}

#[derive(Default)]
pub struct Manual {
    pub open: bool,
    pub query: String,
    /// Kapitel, zu dem beim nächsten Zeichnen gescrollt wird.
    pub jump: Option<usize>,
}

fn job(text: &str, size: f32, pal: &theme::Palette, strong: bool) -> LayoutJob {
    let mut job = LayoutJob::default();
    for (t, bold, italic, code) in spans(text) {
        let font_id = if code {
            FontId::monospace(size - 1.0)
        } else if bold || strong {
            theme::strong_font(size)
        } else {
            FontId::new(size, FontFamily::Proportional)
        };
        let format = TextFormat {
            font_id,
            color: pal.text,
            italics: italic,
            background: if code { pal.widget } else { egui::Color32::TRANSPARENT },
            ..Default::default()
        };
        job.append(&t, 0.0, format);
    }
    job
}

pub fn show(app: &mut VectorLumeApp, ctx: &egui::Context) {
    if !app.manual.open {
        return;
    }
    let md = if crate::i18n::language() == Language::English { EN } else { DE };
    let blocks = parse(md);
    let chapters = chapters(&blocks);
    let mut open = true;
    egui::Window::new(trs(&format!("{} Handbuch", icon::BOOK_OPEN)))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size(vec2(900.0, 640.0))
        .pivot(Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            let pal = theme::of(ui);
            let m = &mut app.manual;
            let q = m.query.trim().to_lowercase();
            let matches =
                |blocks: &[Block]| q.is_empty() || blocks.iter().any(|b| b.text().to_lowercase().contains(&q));
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(210.0);
                    ui.add(TextEdit::singleline(&mut m.query).hint_text(tr("Suchen…")).desired_width(200.0));
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical().id_salt("manual_toc").show(ui, |ui| {
                        for (i, (title, blocks)) in chapters.iter().enumerate() {
                            if !matches(blocks) {
                                continue;
                            }
                            if ui.selectable_label(false, RichText::new(title).size(13.0)).clicked() {
                                m.jump = Some(i);
                            }
                        }
                    });
                });
                ui.separator();
                egui::ScrollArea::vertical().id_salt("manual_text").auto_shrink([false, false]).show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.set_max_width(640.0);
                        let mut shown = 0;
                        for (i, (_, blocks)) in chapters.iter().enumerate() {
                            if !matches(blocks) {
                                continue;
                            }
                            shown += 1;
                            for b in *blocks {
                                let resp = match b {
                                    Block::Title(t) => ui.add(Label::new(job(t, 24.0, pal, true)).wrap()),
                                    Block::Heading(t) => {
                                        ui.add_space(14.0);
                                        ui.add(Label::new(job(t, 19.0, pal, true)).wrap())
                                    }
                                    Block::Subheading(t) => {
                                        ui.add_space(6.0);
                                        ui.add(Label::new(job(t, 16.0, pal, true)).wrap())
                                    }
                                    Block::Paragraph(t) => ui.add(Label::new(job(t, 14.0, pal, false)).wrap()),
                                    Block::Bullet(t) | Block::Numbered(_, t) => {
                                        let marker = match b {
                                            Block::Numbered(n, _) => format!("{n}."),
                                            _ => "•".into(),
                                        };
                                        ui.horizontal_top(|ui| {
                                            ui.add_sized(
                                                vec2(18.0, 18.0),
                                                Label::new(RichText::new(marker).color(pal.accent)),
                                            );
                                            ui.add(Label::new(job(t, 14.0, pal, false)).wrap());
                                        })
                                        .response
                                    }
                                    Block::Note(t) => {
                                        egui::Frame::NONE
                                            .fill(pal.panel_alt)
                                            .stroke(egui::Stroke::new(1.0, pal.warning))
                                            .corner_radius(6.0)
                                            .inner_margin(10.0)
                                            .show(ui, |ui| ui.add(Label::new(job(t, 14.0, pal, false)).wrap()))
                                            .response
                                    }
                                };
                                if m.jump == Some(i) && std::ptr::eq(b, &blocks[0]) {
                                    resp.scroll_to_me(Some(Align::TOP));
                                    m.jump = None;
                                }
                                ui.add_space(4.0);
                            }
                        }
                        if shown == 0 {
                            ui.label(RichText::new(tr("Nichts gefunden.")).color(pal.text_weak));
                        }
                    });
                });
            });
        });
    if !open {
        app.manual.open = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_and_inline_marks() {
        let md = "# Titel\n\nEin Absatz\nüber zwei Zeilen.\n\n## Kapitel\n\n- Punkt **fett**\n  weiter\n1. Erstens\n\n> Achtung\n> zweite Zeile\n";
        let b = parse(md);
        assert_eq!(
            b,
            vec![
                Block::Title("Titel".into()),
                Block::Paragraph("Ein Absatz über zwei Zeilen.".into()),
                Block::Heading("Kapitel".into()),
                Block::Bullet("Punkt **fett** weiter".into()),
                Block::Numbered("1".into(), "Erstens".into()),
                Block::Note("Achtung zweite Zeile".into()),
            ]
        );
        let ch = chapters(&b);
        assert_eq!(ch.len(), 2);
        assert_eq!(ch[0].0, "Titel");
        assert_eq!(ch[1].1.len(), 4);
        assert_eq!(
            spans("a **b** *c* `d*e`"),
            vec![
                ("a ".into(), false, false, false),
                ("b".into(), true, false, false),
                (" ".into(), false, false, false),
                ("c".into(), false, true, false),
                (" ".into(), false, false, false),
                ("d*e".into(), false, false, true),
            ]
        );
    }

    /// Beide Handbücher haben dieselbe Gliederung und keine offenen Auszeichnungen.
    #[test]
    fn manuals_parse_cleanly() {
        let (de, en) = (parse(DE), parse(EN));
        let heads = |b: &[Block]| b.iter().filter(|x| matches!(x, Block::Heading(_))).count();
        assert!(heads(&de) >= 10);
        assert_eq!(heads(&de), heads(&en), "Kapitelzahl Deutsch/Englisch");
        for b in de.iter().chain(&en) {
            let t = b.text();
            assert_eq!(t.matches('`').count() % 2, 0, "{t}");
            assert_eq!(t.matches("**").count() % 2, 0, "{t}");
        }
    }
}
