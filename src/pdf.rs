//! The report as a PDF: the same text the report has always been, set as a
//! document someone can print or attach to a complaint.
//!
//! The report is written as plain text first, so its content and its tests do
//! not depend on this module. Here each line is read for what it is (the
//! title, a section heading, a table row, a sentence) and set accordingly.
//! Rows that line up in columns stay in a monospaced face, because their
//! alignment is what makes them readable.
//!
//! The faces come from Windows itself (Verdana and Lucida Console, with Segoe
//! UI, Arial, Consolas and Courier New behind them), which every supported
//! system has and which carry Polish letters.

use printpdf::{
    Color, FontId, Line, LinePoint, Op, ParsedFont, PdfDocument, PdfFontHandle, PdfPage,
    PdfSaveOptions, Point, Pt, Rgb, TextItem,
};

const MM: f32 = 2.834_646;
const PAGE_W: f32 = 210.0 * MM;
const PAGE_H: f32 = 297.0 * MM;
const MARGIN_X: f32 = 18.0 * MM;
const MARGIN_TOP: f32 = 18.0 * MM;
const MARGIN_BOTTOM: f32 = 20.0 * MM;

const BODY: f32 = 9.5;
const MONO: f32 = 7.6;
const HEADING: f32 = 11.5;
const TITLE: f32 = 18.0;
const FOOTER: f32 = 8.0;

/// What a line of the report is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Title,
    Subtitle,
    Heading,
    /// Lined up in columns: a table row or a label padded to a colon.
    Mono,
    /// The line that opens one outage: "#3  24.09 2026 16:46:08  6 s  ...".
    Entry,
    Body,
    Gap,
}

/// Reads the report's text into typed lines.
fn classify(text: &str) -> Vec<(Kind, String)> {
    let mut out = Vec::new();
    let mut titled = false;
    for raw in text.lines() {
        let line = raw.trim_end();
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            // One gap for any run of blank lines.
            if !matches!(out.last(), Some((Kind::Gap, _)) | None) {
                out.push((Kind::Gap, String::new()));
            }
            continue;
        }
        // Underlines drawn with a repeated character belong to plain text.
        if trimmed.len() > 3 && trimmed.chars().all(|c| c == '=' || c == '-') {
            continue;
        }
        if !titled {
            titled = true;
            // "Title, date" on the first line.
            match line.rsplit_once(", ") {
                Some((title, when)) => {
                    out.push((Kind::Title, title.to_string()));
                    out.push((Kind::Subtitle, when.to_string()));
                }
                None => out.push((Kind::Title, line.to_string())),
            }
            continue;
        }
        let indented = line.len() != trimmed.len();
        if !indented && is_heading(trimmed) {
            out.push((Kind::Heading, trimmed.to_string()));
        } else if trimmed.starts_with('#') && trimmed[1..].starts_with(|c: char| c.is_ascii_digit())
        {
            out.push((Kind::Entry, trimmed.to_string()));
        } else if trimmed.contains("  ") {
            out.push((Kind::Mono, line.to_string()));
        } else {
            out.push((Kind::Body, trimmed.to_string()));
        }
    }
    out
}

/// A section heading: its first word is in capitals, like "CONNECTION" or
/// "POMIARY (ostatnia godzina)". A line opening with a date or a number is
/// an entry, not a heading.
fn is_heading(line: &str) -> bool {
    let first = line.split_whitespace().next().unwrap_or("");
    let letters: Vec<char> = first.chars().filter(|c| c.is_alphabetic()).collect();
    letters.len() >= 3 && letters.iter().all(|c| c.is_uppercase())
}

/// A face and what it needs to measure text.
struct Face {
    id: FontId,
    font: ParsedFont,
}

impl Face {
    /// Width of `text` at `size`, in points.
    fn width(&self, text: &str, size: f32) -> f32 {
        let upem = f32::from(self.font.units_per_em.max(1));
        let units: u32 = text
            .chars()
            .map(|c| {
                self.font
                    .lookup_glyph_index(c as u32)
                    .and_then(|g| self.font.glyph_widths.get(&g).copied())
                    .map_or(upem as u32 / 2, u32::from)
            })
            .sum();
        units as f32 / upem * size
    }
}

/// The first of `names` that loads from the Windows fonts folder.
fn load(doc: &mut PdfDocument, names: &[&str]) -> Option<Face> {
    let dir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".to_string());
    names.iter().find_map(|name| {
        let bytes = std::fs::read(std::path::Path::new(&dir).join("Fonts").join(name)).ok()?;
        let font = ParsedFont::from_bytes(&bytes, 0, &mut Vec::new())?;
        let id = doc.add_font(&font);
        Some(Face { id, font })
    })
}

/// Breaks `text` into lines no wider than `max` points. An indented line (a
/// table row) continues a little further in, so the continuation reads as
/// part of its row; a sentence continues flush, as sentences do.
fn wrap(face: &Face, text: &str, size: f32, max: f32) -> Vec<String> {
    if face.width(text, size) <= max {
        return vec![text.to_string()];
    }
    let indent: String = text.chars().take_while(|c| *c == ' ').collect();
    let hang = if indent.is_empty() { String::new() } else { format!("{indent}  ") };
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.trim_start().split(' ') {
        let lead = if lines.is_empty() { &indent } else { &hang };
        let candidate =
            if current.is_empty() { format!("{lead}{word}") } else { format!("{current} {word}") };
        if face.width(&candidate, size) > max && !current.is_empty() {
            lines.push(std::mem::take(&mut current));
            current = format!("{hang}{word}");
        } else {
            current = candidate;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

fn rgb(r: f32, g: f32, b: f32) -> Color {
    Color::Rgb(Rgb::new(r, g, b, None))
}

fn text_op(ops: &mut Vec<Op>, face: &Face, size: f32, colour: Color, x: f32, y: f32, s: &str) {
    ops.extend([
        Op::StartTextSection,
        Op::SetFont { font: PdfFontHandle::External(face.id.clone()), size: Pt(size) },
        Op::SetFillColor { col: colour },
        Op::SetTextCursor { pos: Point { x: Pt(x), y: Pt(y) } },
        Op::ShowText { items: vec![TextItem::Text(s.to_string())] },
        Op::EndTextSection,
    ]);
}

fn rule(ops: &mut Vec<Op>, y: f32, colour: Color, thickness: f32) {
    let at = |x: f32| LinePoint { p: Point { x: Pt(x), y: Pt(y) }, bezier: false };
    ops.extend([
        Op::SetOutlineColor { col: colour },
        Op::SetOutlineThickness { pt: Pt(thickness) },
        Op::DrawLine {
            line: Line { points: vec![at(MARGIN_X), at(PAGE_W - MARGIN_X)], is_closed: false },
        },
    ]);
}

/// The three faces a report is set in.
struct Faces {
    regular: Face,
    bold: Face,
    mono: Face,
}

fn faces(doc: &mut PdfDocument) -> Option<Faces> {
    Some(Faces {
        // ponytail: Verdana and Lucida Console are the smallest faces every
        // Windows has that carry Polish letters, about 550 KB together, and
        // they go in whole: printpdf subsets only with its `text_layout`
        // feature, which pulls in a layout engine. Subset with allsorts
        // directly if the file size ever matters.
        regular: load(doc, &["verdana.ttf", "segoeui.ttf", "arial.ttf"])?,
        bold: load(doc, &["verdanab.ttf", "segoeuib.ttf", "arialbd.ttf"])?,
        mono: load(doc, &["lucon.ttf", "consola.ttf", "cour.ttf"])?,
    })
}

/// Sets `text` as an A4 document. `page_label` turns a page number and the
/// page count into the footer. `Err` when no usable face could be loaded; the
/// caller then keeps the plain text.
pub fn render(text: &str, page_label: impl Fn(usize, usize) -> String) -> Result<Vec<u8>, String> {
    let lines = classify(text);
    let title = lines.iter().find(|(k, _)| *k == Kind::Title).map_or("NetDoctor", |(_, t)| t);
    let mut doc = PdfDocument::new(title);
    let f = faces(&mut doc).ok_or("no usable font in the Windows fonts folder")?;
    let pages = layout(&lines, &f);

    let dim = rgb(0.42, 0.45, 0.50);
    let total = pages.len();
    let pages: Vec<PdfPage> = pages
        .into_iter()
        .enumerate()
        .map(|(i, mut ops)| {
            let label = page_label(i + 1, total);
            let x = PAGE_W - MARGIN_X - f.regular.width(&label, FOOTER);
            text_op(&mut ops, &f.regular, FOOTER, dim.clone(), x, MARGIN_BOTTOM * 0.5, &label);
            PdfPage::new(printpdf::Mm(210.0), printpdf::Mm(297.0), ops)
        })
        .collect();

    let options = PdfSaveOptions { subset_fonts: true, ..Default::default() };
    Ok(doc.with_pages(pages).save(&options, &mut Vec::new()))
}

/// The pages' content, without the footers, which need the page count.
fn layout(lines: &[(Kind, String)], f: &Faces) -> Vec<Vec<Op>> {
    let Faces { regular, bold, mono } = f;
    let ink = rgb(0.12, 0.13, 0.16);
    let dim = rgb(0.42, 0.45, 0.50);
    let accent = rgb(0.13, 0.36, 0.66);
    let hair = rgb(0.82, 0.84, 0.87);
    let width = PAGE_W - MARGIN_X * 2.0;

    let mut pages: Vec<Vec<Op>> = vec![Vec::new()];
    let mut y = PAGE_H - MARGIN_TOP;
    for (kind, s) in lines {
        let (face, size, colour, before, after) = match kind {
            Kind::Title => (&bold, TITLE, &ink, 0.0, 4.0),
            Kind::Subtitle => (&regular, BODY, &dim, 0.0, 10.0),
            Kind::Heading => (&bold, HEADING, &accent, 10.0, 6.0),
            Kind::Mono => (&mono, MONO, &ink, 0.0, 0.0),
            Kind::Entry => (&bold, BODY, &ink, 6.0, 1.0),
            Kind::Body => (&regular, BODY, &ink, 0.0, 0.0),
            Kind::Gap => {
                y -= BODY * 0.6;
                continue;
            }
        };
        let lead = size * 1.4;
        let wrapped = wrap(face, s, size, width);
        // A heading is not left alone at the foot of a page.
        let needed = before
            + lead * wrapped.len() as f32
            + if *kind == Kind::Heading { lead * 3.0 } else { 0.0 };
        if y - needed < MARGIN_BOTTOM {
            pages.push(Vec::new());
            y = PAGE_H - MARGIN_TOP;
        } else {
            y -= before;
        }
        let Some(ops) = pages.last_mut() else { break };
        for l in &wrapped {
            y -= lead;
            text_op(ops, face, size, colour.clone(), MARGIN_X, y, l);
        }
        match kind {
            Kind::Heading => {
                y -= 3.0;
                rule(ops, y, hair.clone(), 0.6);
            }
            Kind::Subtitle => {
                y -= 6.0;
                rule(ops, y, accent.clone(), 1.2);
            }
            _ => {}
        }
        y -= after;
    }
    pages
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_report_text_is_read_into_what_each_line_is() {
        let text = "Raport NetDoctor, 25.09 2026 14:32:05\n\
                    ========\n\
                    \n\
                    POŁĄCZENIE\n  Karta         : Wi-Fi (Wi-Fi)\n\
                    POMIARY (ostatnia godzina)\n  1.1.1.1  śr. 12 ms\n\
                    2026-09-25 11:18 awaria\n  Router odpowiadał przez cały czas.\n\
                    \x20 #3  24.09 2026 16:46:08  6 s  awaria\n";
        let kinds: Vec<Kind> = classify(text).into_iter().map(|(k, _)| k).collect();
        assert_eq!(
            kinds,
            vec![
                Kind::Title,
                Kind::Subtitle,
                Kind::Gap,
                Kind::Heading,
                Kind::Mono,
                Kind::Heading,
                Kind::Mono,
                Kind::Body,
                Kind::Body,
                Kind::Entry,
            ]
        );
    }

    /// Needs the Windows fonts folder, which every machine this builds on has.
    #[test]
    fn a_long_report_comes_out_as_a_pdf_of_several_pages() {
        let mut text = String::from("Raport NetDoctor, 25.09 2026 14:32:05\n\nPOŁĄCZENIE\n");
        for i in 0..200 {
            text.push_str(&format!(
                "  Wiersz {i:>3}   : zażółć gęślą jaźń, długi opis wiersza raportu\n"
            ));
        }
        let mut doc = PdfDocument::new("test");
        let f = faces(&mut doc).expect("the Windows fonts");
        let pages = layout(&classify(&text), &f).len();
        assert!(pages >= 3, "{pages} pages");
        // Every wrapped line fits between the margins.
        let widest = wrap(&f.mono, &format!("  {}", "słowo ".repeat(60)), MONO, 100.0);
        assert!(widest.len() > 1);
        assert!(widest
            .iter()
            .all(|l| f.mono.width(l, MONO) <= 100.0 + f.mono.width("słowo", MONO)));
        let bytes = render(&text, |n, t| format!("{n}/{t}")).unwrap_or_default();
        assert!(bytes.starts_with(b"%PDF"), "not a PDF");
    }
}
