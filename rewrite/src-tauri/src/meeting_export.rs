use crate::domain::MeetingDetail;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TranscriptSide {
    You,
    Other,
    Unknown,
}

#[derive(Debug, PartialEq, Eq)]
struct TranscriptBlock {
    speaker: String,
    side: TranscriptSide,
    timecode: Option<String>,
    text: String,
}

fn speaker_side(label: &str) -> TranscriptSide {
    match label.to_ascii_lowercase().as_str() {
        "you" | "me" | "microphone" | "local speaker" => TranscriptSide::You,
        "other speaker" | "other speakers" | "system audio" | "remote speaker" => TranscriptSide::Other,
        _ => TranscriptSide::Unknown,
    }
}

fn known_speaker(label: &str) -> bool {
    speaker_side(label) != TranscriptSide::Unknown
        || label.to_ascii_lowercase().strip_prefix("speaker ").is_some_and(|number| {
            (1..=2).contains(&number.len()) && number.bytes().all(|b| b.is_ascii_digit())
        })
}

fn clock_shaped(value: &str) -> bool {
    let parts: Vec<_> = value.split(':').collect();
    (2..=3).contains(&parts.len())
        && (1..=3).contains(&parts[0].len())
        && parts.iter().all(|part| part.bytes().all(|b| b.is_ascii_digit()))
        && parts[1..].iter().all(|part| part.len() == 2)
}

fn format_timecode(seconds: f64) -> String {
    let seconds = seconds.max(0.0).floor() as u64;
    if seconds >= 3600 {
        format!("{}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60)
    } else {
        format!("{}:{:02}", seconds / 60, seconds % 60)
    }
}

/// Match the editor's known labels and clock-shaped prefixes. None preserves
/// the original unstructured export, including whitespace and bracketed prose.
fn parse_transcript(value: &str) -> Option<Vec<TranscriptBlock>> {
    let structured = |blocks: Vec<TranscriptBlock>| {
        blocks.iter().any(|block| block.speaker != "Transcript" || block.timecode.is_some())
            .then_some(blocks)
    };
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(value) {
        if let Some(segments) = json.as_array().or_else(|| json.get("segments")?.as_array()) {
            let blocks: Vec<_> = segments.iter().filter_map(|segment| {
                let text = segment.get("text")?;
                let speaker = segment.get("speaker").and_then(|v| v.as_str()).unwrap_or("Transcript");
                let timecode = segment.get("timecode").and_then(|v| v.as_str())
                    .filter(|time| clock_shaped(time)).map(str::to_owned)
                    .or_else(|| segment.get("startSeconds").and_then(|v| v.as_f64())
                        .filter(|seconds| seconds.is_finite()).map(format_timecode));
                let side = match segment.get("side").and_then(|v| v.as_str()) {
                    Some("you") => TranscriptSide::You,
                    Some("other") => TranscriptSide::Other,
                    _ => speaker_side(speaker),
                };
                Some(TranscriptBlock {
                    speaker: speaker.to_owned(), side, timecode,
                    text: text.as_str().map(str::to_owned).unwrap_or_else(|| text.to_string()),
                })
            }).collect();
            if !blocks.is_empty() {
                return structured(blocks);
            }
        }
    }
    if let Some(blocks) = parse_legacy_table(value) {
        return structured(blocks);
    }
    let mut paragraphs = Vec::new();
    let mut paragraph = String::new();
    for line in value.lines() {
        if line.trim().is_empty() {
            if !paragraph.is_empty() {
                paragraphs.push(std::mem::take(&mut paragraph));
            }
        } else {
            if !paragraph.is_empty() { paragraph.push('\n'); }
            paragraph.push_str(line);
        }
    }
    if !paragraph.is_empty() { paragraphs.push(paragraph); }
    structured(paragraphs.iter().map(|paragraph| {
        let mut text = paragraph.trim();
        let mut timecode = None;
        if let Some((clock, rest)) = text.strip_prefix('[').and_then(|v| v.split_once(']')) {
            if clock_shaped(clock) {
                timecode = Some(clock.to_owned());
                text = rest.trim_start();
            }
        }
        let (speaker, text) = match text.split_once(':') {
            Some((label, rest)) if known_speaker(label) => (label, rest.trim()),
            _ => ("Transcript", text),
        };
        TranscriptBlock {
            speaker: speaker.to_owned(), side: speaker_side(speaker), timecode,
            text: text.to_owned(),
        }
    }).collect())
}

/// The legacy app saved two-source meetings as a tab-separated table of
/// ten-second rows: `Time\tOther speakers\tMicrophone`. Consecutive rows from
/// one side join into a passage that keeps the clock of its first row.
fn parse_legacy_table(value: &str) -> Option<Vec<TranscriptBlock>> {
    let mut lines = value.lines();
    let header: Vec<_> = lines.next()?.split('\t').map(|cell| cell.trim().to_ascii_lowercase()).collect();
    if header != ["time", "other speakers", "microphone"] {
        return None;
    }
    let mut blocks: Vec<TranscriptBlock> = Vec::new();
    for line in lines {
        let mut cells = line.split('\t');
        let clock = cells.next().unwrap_or("").trim();
        for speaker in ["Other speakers", "You"] {
            let text = cells.next().unwrap_or("").trim();
            if text.is_empty() { continue; }
            match blocks.last_mut() {
                Some(last) if last.speaker == speaker => {
                    last.text.push(' ');
                    last.text.push_str(text);
                }
                _ => blocks.push(TranscriptBlock {
                    speaker: speaker.to_owned(), side: speaker_side(speaker),
                    timecode: clock_shaped(clock).then(|| clock.to_owned()),
                    text: text.to_owned(),
                }),
            }
        }
    }
    (!blocks.is_empty()).then_some(blocks)
}

fn block_source(block: &TranscriptBlock) -> String {
    let time = block.timecode.as_ref().map(|time| format!("[{time}] ")).unwrap_or_default();
    let speaker = if block.speaker == "Transcript" { String::new() } else { format!("{}: ", block.speaker) };
    format!("{time}{speaker}{}", block.text)
}

/// Clean only spoken passages. Speaker labels, clocks and saved table/JSON
/// structure are kept outside the model's input.
pub(super) fn map_passages(value: &str, mut clean: impl FnMut(&str) -> Result<String, String>) -> Result<String, String> {
    if let Some(mut blocks) = parse_transcript(value) {
        for block in &mut blocks {
            if !block.text.trim().is_empty() { block.text = clean(&block.text)?; }
        }
        return Ok(blocks.iter().map(block_source).collect::<Vec<_>>().join("\n\n"));
    }
    value.split("\n\n").map(|passage| {
        if passage.trim().is_empty() { Ok(passage.to_owned()) } else { clean(passage) }
    }).collect::<Result<Vec<_>, _>>().map(|passages| passages.join("\n\n"))
}

fn with_outage_markers(detail: &MeetingDetail) -> MeetingDetail {
    let mut result = detail.clone();
    if detail.audio_outages.is_empty() { return result; }
    let mut gaps: Vec<_> = detail.audio_outages.iter().collect();
    gaps.sort_by(|a, b| a.start_seconds.total_cmp(&b.start_seconds));
    let marker = |gap: &crate::domain::AudioOutage| {
        let start = super::outages::clock(gap.start_seconds);
        let end = super::outages::clock(gap.end_seconds);
        let label = if gap.source == "recognition" { "Transcription missing" } else { "Audio missing" };
        if gap.start_seconds == gap.end_seconds {
            format!("[{label} around {start}] {}", gap.reason)
        } else {
            format!("[{label} {start}–{end}] {}", gap.reason)
        }
    };
    let mut passages = Vec::new();
    let mut next = 0;
    if let Some(blocks) = parse_transcript(&detail.transcript) {
        for block in blocks {
            if let Some(at) = block.timecode.as_deref().and_then(super::outages::seconds) {
                while next < gaps.len() && gaps[next].start_seconds <= at {
                    passages.push(marker(gaps[next]));
                    next += 1;
                }
            }
            passages.push(block_source(&block));
        }
    } else if !detail.transcript.is_empty() {
        passages.push(detail.transcript.clone());
    }
    // Without clocks in edited prose there is no safe text-to-time mapping.
    // Keep the original prose and append the explicitly timed marker lines.
    passages.extend(gaps[next..].iter().map(|gap| marker(gap)));
    result.transcript = passages.join("\n\n");
    result
}

fn transcript_text(value: &str, markdown: bool) -> String {
    let Some(blocks) = parse_transcript(value) else {
        return if markdown { escape_markdown(value) } else { value.to_owned() };
    };
    blocks.iter().map(|block| {
        if markdown {
            let heading = match (&*block.speaker, &block.timecode) {
                ("Transcript", None) => String::new(),
                ("Transcript", Some(time)) => format!("{time}\n"),
                (speaker, time) => format!("**{}**{}\n", escape_markdown(speaker),
                    time.as_ref().map(|t| format!(" · {t}")).unwrap_or_default()),
            };
            format!("{heading}{}", escape_markdown(&block.text))
        } else {
            let time = block.timecode.as_ref().map(|t| format!("[{t}] ")).unwrap_or_default();
            let speaker = if block.speaker == "Transcript" { String::new() } else { format!("{}: ", block.speaker) };
            format!("{time}{speaker}{}", block.text)
        }
    }).collect::<Vec<_>>().join("\n\n")
}

fn transcript_html(value: &str) -> String {
    let Some(blocks) = parse_transcript(value) else {
        return format!("<pre>{}</pre>", escape_html(value));
    };
    let mut html = String::from("<div class=\"transcript\">");
    for block in blocks {
        let side = match block.side { TranscriptSide::You => "you", TranscriptSide::Other => "other", TranscriptSide::Unknown => "unknown" };
        html.push_str(&format!("<section class=\"transcript-block {side}\">"));
        if block.speaker != "Transcript" || block.timecode.is_some() {
            html.push_str(&format!("<header><strong>{}</strong>", escape_html(&block.speaker)));
            if let Some(time) = block.timecode {
                html.push_str(&format!("<span class=\"timecode\">{}</span>", escape_html(&time)));
            }
            html.push_str("</header>");
        }
        html.push_str(&format!("<p>{}</p></section>", escape_html(&block.text)));
    }
    html.push_str("</div>");
    html
}

pub fn render_export(detail: &MeetingDetail, format: &str) -> Result<String, String> {
    let marked = with_outage_markers(detail);
    let detail = &marked;
    match format {
        "text" => Ok(format!(
            "{}\n{}\n\nNOTES\n{}\n\n{}\n\nTRANSCRIPT\n{}\n",
            detail.title,
            detail.created_at,
            NOTES_NOTICE,
            plain_notes(&detail.notes),
            transcript_text(&detail.transcript, false)
        )),
        "markdown" => Ok(format!(
            "# {}\n\n{}\n\n## Notes\n\n{}\n\n{}\n\n## Transcript\n\n{}\n",
            escape_markdown(&detail.title),
            escape_markdown(&detail.created_at),
            NOTES_NOTICE,
            safe_markdown(&detail.notes),
            transcript_text(&detail.transcript, true)
        )),
        "html" => render_html(detail),
        _ => Err("Choose text, markdown, html, or pdf export.".into()),
    }
}

const NOTES_NOTICE: &str = "Notes may include AI-generated text and user edits. Check details against the full transcript.";
const PAPER: &[u8] = include_bytes!("../../src/brand/material/paper-stock-seamless.png");

fn markdown_parser(text: &str) -> pulldown_cmark::Parser<'_> {
    pulldown_cmark::Parser::new_ext(
        text,
        pulldown_cmark::Options::ENABLE_TABLES
            | pulldown_cmark::Options::ENABLE_STRIKETHROUGH
            | pulldown_cmark::Options::ENABLE_TASKLISTS,
    )
}

/// Keep Markdown structure but escape raw HTML and neutralize active link targets.
fn safe_markdown(text: &str) -> String {
    use pulldown_cmark::{Event, Tag};
    let mut edits = Vec::new();
    for (event, range) in markdown_parser(text).into_offset_iter() {
        match event {
            Event::Html(_) | Event::InlineHtml(_) => {
                edits.push((range.clone(), escape_html(&text[range])))
            }
            Event::Start(Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. })
                if !safe_link(&dest_url) =>
            {
                edits.push((range.clone(), escape_markdown(&text[range])));
            }
            _ => {}
        }
    }
    edits.sort_by_key(|(range, _)| range.start);
    let mut result = String::new();
    let mut end = 0;
    for (range, replacement) in edits {
        if range.start < end {
            continue;
        }
        result.push_str(&text[end..range.start]);
        result.push_str(&replacement);
        end = range.end;
    }
    result.push_str(&text[end..]);
    result
}

fn safe_link(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    !url.chars().any(char::is_control)
        && (lower.starts_with("https://")
            || lower.starts_with("http://")
            || lower.starts_with("mailto:"))
}

fn notes_html(text: &str) -> String {
    use pulldown_cmark::{Event, Tag, TagEnd};
    let events = markdown_parser(text).map(|event| match event {
        Event::Html(text) | Event::InlineHtml(text) => Event::Text(text),
        // Exports never load transcript-supplied images or expose active links.
        Event::Start(Tag::Image { .. } | Tag::Link { .. }) => Event::Start(Tag::Emphasis),
        Event::End(TagEnd::Image | TagEnd::Link) => Event::End(TagEnd::Emphasis),
        other => other,
    });
    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, events);
    html
}

fn render_html(detail: &MeetingDetail) -> Result<String, String> {
    use base64::Engine;
    let base64 = base64::engine::general_purpose::STANDARD;
    let font = base64.encode(include_bytes!(
        "../../src/brand/fonts/SourceSerif4Variable-Roman.ttf.woff2"
    ));
    let paper = base64.encode(PAPER);
    Ok(format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; font-src data:; img-src data:; base-uri 'none'; form-action 'none'">
<title>{title}</title><style>
@font-face{{font-family:Stationery;src:url(data:font/woff2;base64,{font}) format('woff2');font-weight:200 900}}
*{{box-sizing:border-box}}html{{background:#e5e5db;color:#28352c}}body{{margin:0;font:18px/1.55 Stationery,Georgia,serif}}
main{{max-width:860px;margin:36px auto;padding:44px 52px 60px;background:#f9f7ef url(data:image/png;base64,{paper}) repeat left top/620px 620px;box-shadow:0 1px 2px #28352c38,0 10px 32px #28352c12}}
.brand{{font-size:14px;font-weight:650;letter-spacing:.04em;color:#34432e}}h1{{font-size:34px;line-height:1.18;margin:22px 0 14px;font-weight:650}}h2{{font-size:24px;margin:32px 0 14px}}h3,h4,h5,h6{{font-size:20px;margin:22px 0 10px}}p,ul,ol,pre,table{{margin:12px 0}}.meta,.notice{{color:#5b6052;font-size:14px}}.section-title{{border-top:1px solid #79816955;padding-top:24px}}pre{{white-space:pre-wrap;font:inherit;overflow-wrap:anywhere}}code{{font-size:.93em}}table{{border-collapse:collapse;width:100%;font-size:16px}}th,td{{text-align:left;vertical-align:top;padding:9px 12px;border:1px solid #79816955}}th{{font-weight:650}}blockquote{{border-left:3px solid #748267;padding-left:18px;margin-left:0}}li{{margin:5px 0}}.notes{{overflow-wrap:anywhere}}
.transcript-block{{width:86%;margin:22px 0;overflow-wrap:anywhere}}.transcript-block.you{{margin-left:auto;text-align:right}}.transcript-block.unknown{{width:100%}}.transcript-block header{{break-after:avoid}}.transcript-block strong{{font-weight:650}}.timecode{{display:block;font-size:14px;color:#5b6052}}.transcript-block p{{white-space:pre-wrap;margin:8px 0}}
@media(max-width:650px){{main{{margin:0;padding:24px}}}}@media print{{@page{{size:A4;margin:15mm}}html{{background:white}}main{{max-width:none;margin:0;padding:0;box-shadow:none;print-color-adjust:exact;-webkit-print-color-adjust:exact}}h1,h2,h3,h4{{break-after:avoid}}tr{{break-inside:avoid}}}}
</style></head><body><main><div class="brand">SonoJot</div><h1>{title}</h1><p class="meta">{date} &nbsp; / &nbsp; {minutes:.0} min</p>
<h2 class="section-title">Notes</h2><p class="notice">{notice}</p><div class="notes">{notes}</div>
<h2 class="section-title">Transcript</h2>{transcript}</main></body></html>"#,
        title = escape_html(&detail.title),
        date = escape_html(&detail.created_at),
        minutes = detail.duration_seconds / 60.0,
        notice = NOTES_NOTICE,
        notes = notes_html(&detail.notes),
        transcript = transcript_html(&detail.transcript)
    ))
}

#[derive(Default)]
struct NoteRun {
    text: String,
    bold: bool,
}
enum NoteBlock {
    Paragraph {
        heading: u8,
        depth: usize,
        runs: Vec<NoteRun>,
    },
    Table(Vec<Vec<Vec<NoteRun>>>),
}

/// Share semantic Markdown blocks between text and PDF so markers never print as formatting.
fn note_blocks(text: &str) -> Vec<NoteBlock> {
    use pulldown_cmark::{Event, Tag, TagEnd};
    let mut blocks = Vec::new();
    let mut runs = Vec::new();
    let mut heading = 0;
    let mut strong: usize = 0;
    let mut lists: Vec<Option<u64>> = Vec::new();
    let mut rows = Vec::new();
    let mut cells = Vec::new();
    let mut in_table = false;
    fn flush(blocks: &mut Vec<NoteBlock>, runs: &mut Vec<NoteRun>, heading: u8, depth: usize) {
        if !runs.is_empty() {
            blocks.push(NoteBlock::Paragraph {
                heading,
                depth,
                runs: std::mem::take(runs),
            });
        }
    }
    for event in markdown_parser(text) {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                flush(&mut blocks, &mut runs, heading, lists.len());
                heading = level as u8;
            }
            Event::End(TagEnd::Heading(_)) => {
                flush(&mut blocks, &mut runs, heading, lists.len());
                heading = 0;
            }
            Event::End(TagEnd::Paragraph | TagEnd::CodeBlock | TagEnd::BlockQuote(_))
                if !in_table =>
            {
                flush(&mut blocks, &mut runs, heading, lists.len())
            }
            Event::Start(Tag::List(start)) => {
                flush(&mut blocks, &mut runs, heading, lists.len());
                lists.push(start);
            }
            Event::End(TagEnd::List(_)) => {
                flush(&mut blocks, &mut runs, heading, lists.len());
                lists.pop();
            }
            Event::Start(Tag::Item) => {
                flush(&mut blocks, &mut runs, heading, lists.len());
                let prefix = match lists.last_mut() {
                    Some(Some(next)) => {
                        let prefix = format!("{}. ", next);
                        *next += 1;
                        prefix
                    }
                    _ => "• ".into(),
                };
                runs.push(NoteRun {
                    text: prefix,
                    bold: false,
                });
            }
            Event::End(TagEnd::Item) => flush(&mut blocks, &mut runs, heading, lists.len()),
            Event::Start(Tag::Strong) => strong += 1,
            Event::End(TagEnd::Strong) => strong = strong.saturating_sub(1),
            Event::Start(Tag::Table(_)) => {
                flush(&mut blocks, &mut runs, heading, lists.len());
                in_table = true;
            }
            Event::End(TagEnd::TableCell) => cells.push(std::mem::take(&mut runs)),
            Event::End(TagEnd::TableHead | TagEnd::TableRow) => {
                rows.push(std::mem::take(&mut cells))
            }
            Event::End(TagEnd::Table) => {
                blocks.push(NoteBlock::Table(std::mem::take(&mut rows)));
                in_table = false;
            }
            Event::Text(text) | Event::Code(text) | Event::Html(text) | Event::InlineHtml(text) => {
                runs.push(NoteRun {
                    text: text.into_string(),
                    bold: strong > 0,
                })
            }
            Event::SoftBreak | Event::HardBreak => runs.push(NoteRun {
                text: "\n".into(),
                bold: false,
            }),
            Event::TaskListMarker(done) => runs.push(NoteRun {
                text: if done { "[x] " } else { "[ ] " }.into(),
                bold: false,
            }),
            Event::Rule => flush(&mut blocks, &mut runs, heading, lists.len()),
            _ => {}
        }
    }
    flush(&mut blocks, &mut runs, heading, lists.len());
    blocks
}

fn plain_notes(text: &str) -> String {
    note_blocks(text)
        .into_iter()
        .map(|block| match block {
            NoteBlock::Paragraph { runs, .. } => {
                runs.into_iter().map(|run| run.text).collect::<String>()
            }
            NoteBlock::Table(rows) => rows
                .into_iter()
                .map(|cells| {
                    cells
                        .into_iter()
                        .map(|runs| runs.into_iter().map(|run| run.text).collect::<String>())
                        .collect::<Vec<_>>()
                        .join(" | ")
                })
                .collect::<Vec<_>>()
                .join("\n"),
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Pure-Rust PDF export with embedded fonts and a Unicode character map.
/// Unsupported glyphs are reported before the destination is replaced.
pub fn render_pdf(detail: &MeetingDetail) -> Result<Vec<u8>, String> {
    let marked = with_outage_markers(detail);
    let detail = &marked;
    use genpdf::{
        elements::{Break, Paragraph},
        fonts::{FontData, FontFamily},
        style::{Color, Style},
        Element,
    };
    const REGULAR: &[u8] = include_bytes!("../../src/brand/fonts/SourceSerif4-Regular.ttf");
    const BOLD: &[u8] = include_bytes!("../../src/brand/fonts/SourceSerif4-Bold.ttf");
    if detail.transcript.len() + detail.notes.len() > 512 * 1024 {
        return Err("PDF export is limited to 512 KiB of text. Export this large meeting as HTML, Markdown, or text.".into());
    }
    let coverage = rusttype::Font::from_bytes(REGULAR)
        .map_err(|_| "The embedded PDF font could not be read.")?;
    for text in [
        &detail.title,
        &detail.created_at,
        &detail.notes,
        &detail.transcript,
    ] {
        if let Some(ch) = text
            .chars()
            .find(|ch| !ch.is_whitespace() && coverage.glyph(*ch).id().0 == 0)
        {
            return Err(format!("The PDF font does not contain U+{:04X}. Export as HTML or text to preserve every character.", u32::from(ch)));
        }
    }
    let regular =
        FontData::new(REGULAR.to_vec(), None).map_err(|_| "Cannot load the embedded PDF font.")?;
    let bold = FontData::new(BOLD.to_vec(), None)
        .map_err(|_| "Cannot load the embedded PDF heading font.")?;
    let mut document = genpdf::Document::new(FontFamily {
        regular: regular.clone(),
        bold: bold.clone(),
        italic: regular,
        bold_italic: bold,
    });
    document.set_title(&detail.title);
    document.set_paper_size(genpdf::PaperSize::A4);
    document.set_font_size(11);
    document.set_line_spacing(1.35);
    document.set_minimal_conformance();
    let mut decorator = genpdf::SimplePageDecorator::new();
    decorator.set_margins(20);
    decorator.set_header(|page| {
        let mut header = genpdf::elements::LinearLayout::vertical();
        header.push(
            Paragraph::new(format!("SonoJot  /  {page}")).styled(
                Style::new()
                    .with_font_size(9)
                    .with_color(Color::Rgb(91, 96, 82)),
            ),
        );
        header.push(Break::new(1));
        header
    });
    // Paper is decorative; decode it once at a modest resolution so the image
    // objects in a long PDF do not overwhelm memory. Text remains vector text.
    let paper_pixels = image::load_from_memory_with_format(PAPER, image::ImageFormat::Png)
        .map_err(|e| format!("Cannot decode the bundled paper for PDF: {e}"))?
        .resize_exact(310, 310, image::imageops::FilterType::Triangle);
    let paper = genpdf::elements::Image::from_dynamic_image(paper_pixels)
        .map_err(|e| format!("Cannot load the bundled paper for PDF: {e}"))?
        // Retain the app's physical 620-CSS-pixel tile scale.
        .with_dpi(48.0);
    document.set_page_decorator(PaperPage {
        paper,
        content: decorator,
    });
    document.push(Paragraph::new(&detail.title).styled(Style::new().bold().with_font_size(23)));
    document.push(Break::new(0.5));
    document.push(
        Paragraph::new(format!(
            "{}  |  {:.0} min",
            detail.created_at,
            detail.duration_seconds / 60.0
        ))
        .styled(
            Style::new()
                .with_font_size(10)
                .with_color(Color::Rgb(90, 90, 90)),
        ),
    );
    for (heading, text) in [("Notes", &detail.notes), ("Transcript", &detail.transcript)] {
        document.push(Break::new(1.2));
        document.push(Paragraph::new(heading).styled(Style::new().bold().with_font_size(15)));
        document.push(Break::new(0.4));
        if heading == "Notes" {
            document.push(
                Paragraph::new(NOTES_NOTICE).styled(
                    Style::new()
                        .with_font_size(9)
                        .with_color(Color::Rgb(91, 96, 82)),
                ),
            );
            document.push(Break::new(0.5));
        }
        if text.trim().is_empty() {
            document.push(
                Paragraph::new("No saved text.")
                    .styled(Style::new().with_color(Color::Rgb(110, 110, 110))),
            );
        } else if heading == "Notes" {
            for block in note_blocks(text) {
                match block {
                    NoteBlock::Paragraph {
                        heading,
                        depth,
                        mut runs,
                    } => {
                        if depth > 1 {
                            runs.insert(
                                0,
                                NoteRun {
                                    text: "    ".repeat(depth - 1),
                                    bold: false,
                                },
                            );
                        }
                        let paragraph = pdf_paragraph(runs);
                        let style = if heading > 0 {
                            Style::new()
                                .bold()
                                .with_font_size(if heading < 3 { 14 } else { 12 })
                        } else {
                            Style::new()
                        };
                        document.push(paragraph.styled(style.with_color(Color::Rgb(40, 53, 44))));
                        document.push(Break::new(if heading > 0 { 0.4 } else { 0.25 }));
                    }
                    NoteBlock::Table(rows) => {
                        let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
                        if columns == 0 {
                            continue;
                        }
                        let mut table = genpdf::elements::TableLayout::new(vec![1; columns]);
                        table.set_cell_decorator(genpdf::elements::FrameCellDecorator::new(
                            true, true, false,
                        ));
                        for (index, row) in rows.into_iter().enumerate() {
                            let mut cells: Vec<Box<dyn Element>> = row
                                .into_iter()
                                .map(|runs| {
                                    let style = if index == 0 {
                                        Style::new().bold()
                                    } else {
                                        Style::new()
                                    };
                                    Box::new(
                                        pdf_paragraph(runs)
                                            .styled(style.with_font_size(10))
                                            .padded(2),
                                    ) as Box<dyn Element>
                                })
                                .collect();
                            while cells.len() < columns {
                                cells.push(Box::new(Paragraph::default()));
                            }
                            table
                                .push_row(cells)
                                .map_err(|e| format!("Cannot lay out a notes table: {e}"))?;
                        }
                        document.push(table);
                        document.push(Break::new(0.5));
                    }
                }
            }
        } else if let Some(blocks) = parse_transcript(text) {
            for block in blocks {
                let (alignment, padding) = match block.side {
                    // You sit on the right, as the sender does in a conversation view.
                    TranscriptSide::You => (genpdf::Alignment::Right, (0, 0, 0, 24)),
                    TranscriptSide::Other => (genpdf::Alignment::Left, (0, 24, 0, 0)),
                    TranscriptSide::Unknown => (genpdf::Alignment::Left, (0, 0, 0, 0)),
                };
                if block.speaker != "Transcript" || block.timecode.is_some() {
                    document.push(Paragraph::new(&block.speaker).aligned(alignment)
                        .styled(Style::new().bold().with_color(Color::Rgb(40, 53, 44))).padded(padding));
                    if let Some(time) = &block.timecode {
                        document.push(Paragraph::new(time).aligned(alignment)
                            .styled(Style::new().with_font_size(9).with_color(Color::Rgb(91, 96, 82))).padded(padding));
                    }
                    document.push(Break::new(0.25));
                }
                for line in block.text.lines() {
                    if line.trim().is_empty() {
                        document.push(Break::new(0.4));
                    } else {
                        document.push(Paragraph::new(line.replace('\t', "    ")).aligned(alignment)
                            .styled(Style::new().with_color(Color::Rgb(40, 53, 44))).padded(padding));
                    }
                }
                document.push(Break::new(0.8));
            }
        } else {
            for line in text.lines() {
                if line.trim().is_empty() {
                    document.push(Break::new(0.65));
                } else {
                    document.push(
                        Paragraph::new(line.replace('\t', "    "))
                            .styled(Style::new().with_color(Color::Rgb(40, 53, 44))),
                    );
                }
            }
        }
    }
    let mut bytes = Vec::new();
    document.render(&mut bytes).map_err(|e| format!("Cannot render this meeting as PDF: {e}. HTML or text export preserves the complete text."))?;
    Ok(bytes)
}

fn pdf_paragraph(runs: Vec<NoteRun>) -> genpdf::elements::Paragraph {
    let mut paragraph = genpdf::elements::Paragraph::default();
    for run in runs {
        let style = if run.bold {
            genpdf::style::Style::new().bold()
        } else {
            genpdf::style::Style::new()
        };
        paragraph.push_styled(run.text.replace('\t', "    "), style);
    }
    paragraph
}

struct PaperPage {
    paper: genpdf::elements::Image,
    content: genpdf::SimplePageDecorator,
}

impl genpdf::PageDecorator for PaperPage {
    fn decorate_page<'a>(
        &mut self,
        context: &genpdf::Context,
        area: genpdf::render::Area<'a>,
        style: genpdf::style::Style,
    ) -> Result<genpdf::render::Area<'a>, genpdf::error::Error> {
        use genpdf::Element;
        let tile = 620.0 * 25.4 / 96.0;
        let mut y = 0.0;
        while y < f64::from(area.size().height) {
            let mut x = 0.0;
            while x < f64::from(area.size().width) {
                self.paper.set_position((x, y));
                self.paper.render(context, area.clone(), style)?;
                x += tile;
            }
            y += tile;
        }
        self.content.decorate_page(context, area, style)
    }
}
fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn escape_markdown(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if "\\`*_{}[]()#+-.!|>".contains(ch) {
            out.push('\\');
        }
        match ch {
            '<' => out.push_str("&lt;"),
            '&' => out.push_str("&amp;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(transcript: &str) -> MeetingDetail {
        MeetingDetail {
            summary: crate::domain::MeetingSummary {
                id: "structured-export".into(), title: "Release planning".into(),
                created_at: "2026-09-19T14:30:00Z".into(), duration_seconds: 3700.0,
                status: "transcribed".into(), has_audio: false, has_transcript: true,
            },
            transcript: transcript.into(), notes: "### Decision\n\nRelease on Friday after the checklist is complete.".into(),
            source: "both".into(), audio_path: None, error: None, transcription_model: None, speaker_count: 0, speaker_labels: true,
            recognition_completeness: crate::domain::RecognitionCompleteness::Complete,
            recognition_warnings: Vec::new(),
            recording_warnings: Vec::new(),
            audio_outages: Vec::new(), missing_seconds: 0.0,
            cleaned_transcript: None, transcription_stopped_at_seconds: None,
        }
    }

    #[test]
    fn immutable_gap_lines_are_timed_and_present_in_every_export_format() {
        let mut detail = sample("[0:00] You: Before.\n\n[0:30] Other speakers: After.");
        detail.audio_outages = vec![crate::domain::AudioOutage {
            source: "microphone".into(), start_seconds: 10.0, end_seconds: 24.0,
            reason: "Microphone disconnected <unsafe>".into(),
        }, crate::domain::AudioOutage {
            source: "recognition".into(), start_seconds: 25.0, end_seconds: 26.0,
            reason: "Transcription reached its output token limit".into(),
        }];
        for format in ["text", "markdown", "html"] {
            let exported = render_export(&detail, format).unwrap();
            let first = exported.find("Audio missing 0:10–0:24").unwrap();
            let second = exported.find("Transcription missing 0:25–0:26").unwrap();
            assert!(exported.find("Before").unwrap() < first);
            assert!(first < second && second < exported.find("After").unwrap());
            if format == "html" { assert!(!exported.contains("<unsafe>")); }
        }
        let marked = with_outage_markers(&detail);
        assert!(marked.transcript.contains("\n\n[Audio missing 0:10–0:24]"));
        let pdf = render_pdf(&detail).unwrap();
        assert!(pdf.starts_with(b"%PDF-"));
        assert!(String::from_utf8_lossy(&pdf).contains("/ToUnicode"));
        assert!(!detail.transcript.contains("missing"));
    }

    #[test]
    fn cleanup_preserves_time_and_speakers_for_text_json_and_legacy_tables() {
        for value in [
            "[0:10] You: um hello\n\n[0:30] Other speakers: um answer",
            r#"[{"speaker":"You","startSeconds":10,"text":"um hello"},{"speaker":"Other speakers","startSeconds":30,"text":"um answer"}]"#,
            "Time\tOther speakers\tMicrophone\n0:10\t\tum hello\n0:30\tum answer\t",
        ] {
            let cleaned = map_passages(value, |text| Ok(text.strip_prefix("um ").unwrap().into())).unwrap();
            assert_eq!(cleaned, "[0:10] You: hello\n\n[0:30] Other speakers: answer");
        }
    }

    #[test]
    fn old_detail_defaults_keep_cleaned_copy_and_outages_optional() {
        let mut saved = serde_json::to_value(sample("Original.")).unwrap();
        for key in ["audioOutages", "missingSeconds", "cleanedTranscript", "transcriptionStoppedAtSeconds"] {
            saved.as_object_mut().unwrap().remove(key);
        }
        let detail: MeetingDetail = serde_json::from_value(saved).unwrap();
        assert!(detail.audio_outages.is_empty());
        assert_eq!(detail.missing_seconds, 0.0);
        assert!(detail.cleaned_transcript.is_none());
        assert!(detail.transcription_stopped_at_seconds.is_none());
    }

    #[test]
    fn ordinary_prose_and_brackets_stay_unstructured() {
        for text in ["So the plan is simple: ship Friday.", "[laughter]", "[later] You: hello",
            "[1:2] hello", "[1000:01] hello", "Alex: Friday", "Speaker 123: Friday", ""] {
            assert_eq!(parse_transcript(text), None, "{text}");
            assert_eq!(transcript_text(text, false), text);
            assert_eq!(transcript_text(text, true), escape_markdown(text));
            assert_eq!(transcript_html(text), format!("<pre>{}</pre>", escape_html(text)));
        }
    }

    #[test]
    fn parses_editor_labels_timecodes_and_continuations() {
        for label in ["You", "Me", "Microphone", "Local speaker", "Other speaker", "Other speakers",
            "System audio", "Remote speaker", "Speaker 1", "Speaker 99", "yOu"] {
            let blocks = parse_transcript(&format!("[0:12] {label}: First line.\r\nSecond line.\r\n \r\n[1:00:03] Other speakers: Answer.")).unwrap();
            assert_eq!(blocks.len(), 2);
            assert_eq!(blocks[0].speaker, label);
            assert_eq!(blocks[0].timecode.as_deref(), Some("0:12"));
            assert_eq!(blocks[0].text, "First line.\nSecond line.");
            assert_eq!(blocks[1].side, TranscriptSide::Other);
            assert_eq!(blocks[1].timecode.as_deref(), Some("1:00:03"));
        }
        let blocks = parse_transcript("You: Hello.\n\n[12:34] Unattributed.\n\n[laughter]").unwrap();
        assert_eq!(blocks[0].timecode, None);
        assert_eq!(blocks[1].speaker, "Transcript");
        assert_eq!(blocks[1].text, "Unattributed.");
        assert_eq!(blocks[2].text, "[laughter]");
    }

    #[test]
    fn parses_segment_json_and_preserves_explicit_sides() {
        let input = r#"{"segments":[{"speaker":"You","startSeconds":12.9,"text":"Hello."},{"speaker":"Speaker 2","side":"other","startSeconds":3603,"text":"Answer."}]}"#;
        let blocks = parse_transcript(input).unwrap();
        assert_eq!(blocks[0].side, TranscriptSide::You);
        assert_eq!(blocks[0].timecode.as_deref(), Some("0:12"));
        assert_eq!(blocks[1].side, TranscriptSide::Other);
        assert_eq!(blocks[1].timecode.as_deref(), Some("1:00:03"));
        assert!(parse_transcript(r#"[{"speaker":"Me","timecode":"2:01","text":"Hi"}]"#).is_some());
        assert!(parse_transcript(r#"[{"text":"No structure"}]"#).is_none());
    }

    #[test]
    fn the_legacy_two_column_table_becomes_alternating_passages() {
        let blocks = parse_transcript("Time\tOther speakers\tMicrophone\r\n00:00\tGreat, thank you.\tcan do easily.\r\n00:10\t>> That's awesome.\t\r\n00:20\tVery helpful.\tYeah, perfect.").unwrap();
        let seen: Vec<_> = blocks.iter()
            .map(|b| (b.side, b.timecode.as_deref(), b.text.as_str())).collect();
        assert_eq!(seen, [
            (TranscriptSide::Other, Some("00:00"), "Great, thank you."),
            (TranscriptSide::You, Some("00:00"), "can do easily."),
            (TranscriptSide::Other, Some("00:10"), ">> That's awesome. Very helpful."),
            (TranscriptSide::You, Some("00:20"), "Yeah, perfect."),
        ]);
        let html = render_export(&sample("Time\tOther speakers\tMicrophone\n00:00\tHello.\tHi."), "html").unwrap();
        assert!(html.contains(".transcript-block.you{margin-left:auto;text-align:right}"));
    }

    #[test]
    fn structured_exports_keep_paragraphs_and_escape_content() {
        let text = "[0:12] You: <script> & **spoken**\nA second line.\n\n[0:31] Other speakers: Agreed.";
        let detail = sample(text);
        let plain = render_export(&detail, "text").unwrap();
        assert!(plain.ends_with(&format!("{text}\n")));
        let markdown = render_export(&detail, "markdown").unwrap();
        assert!(markdown.contains("**You** · 0:12\n&lt;script\\> &amp; \\*\\*spoken\\*\\*"));
        assert!(markdown.contains("\n\n**Other speakers** · 0:31\nAgreed\\."));
        let html = render_export(&detail, "html").unwrap();
        assert!(html.contains("class=\"transcript-block you\""));
        assert!(html.contains("class=\"transcript-block other\""));
        assert!(html.contains("<strong>You</strong><span class=\"timecode\">0:12</span>"));
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt; &amp; **spoken**"));
    }

    #[test]
    fn structured_pdf_keeps_unicode_text_and_writes_visual_fixture() {
        let detail = sample("[0:12] You: We can release on Friday once the checklist is complete. The budget stays at $12,000.\n\n[0:31] Other speakers: I will confirm the remaining checklist items by Thursday and send the release notice after approval.\n\n[1:00:03] You: Thanks. Keep the original recording with these notes.");
        let pdf = render_pdf(&detail).unwrap();
        assert!(pdf.starts_with(b"%PDF-"));
        assert!(String::from_utf8_lossy(&pdf).contains("/ToUnicode"));
        if let Some(output) = std::env::var_os("SONOJOT_MEETING_EXPORT_DIR") {
            let dir = std::path::PathBuf::from(output);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("meeting-structured.pdf"), pdf).unwrap();
            std::fs::write(dir.join("meeting-structured.html"), render_export(&detail, "html").unwrap()).unwrap();
        }
    }
}
