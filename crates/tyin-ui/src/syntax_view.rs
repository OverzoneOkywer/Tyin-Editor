use eframe::egui;
use egui::text::{LayoutJob, TextFormat};
use std::path::Path;
use tyin_syntax::{HighlightKind, HighlightSpan, SyntaxEngine};

const MAX_SYNTAX_BYTES: usize = 256 * 1024;

pub(super) fn spans(path: Option<&Path>, text: &str) -> Vec<HighlightSpan> {
    if text.len() > MAX_SYNTAX_BYTES {
        return Vec::new();
    }
    let extension = path
        .and_then(Path::extension)
        .and_then(|extension| extension.to_str())
        .unwrap_or("");
    SyntaxEngine::new()
        .highlight(text, extension)
        .unwrap_or_default()
}

pub(super) fn info(path: Option<&Path>, text: &str) -> String {
    if text.len() > MAX_SYNTAX_BYTES {
        return "syntax disabled for large file".into();
    }
    let extension = path
        .and_then(Path::extension)
        .and_then(|extension| extension.to_str())
        .unwrap_or("");
    SyntaxEngine::new()
        .parse_preview(text, extension)
        .unwrap_or_else(|error| format!("syntax error: {error}"))
}

pub(super) fn layout(
    text: &str,
    spans: &[HighlightSpan],
    wrap_width: f32,
    light_theme: bool,
) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap_width;
    let plain_format = TextFormat {
        font_id: egui::FontId::monospace(12.5),
        color: if light_theme {
            egui::Color32::from_rgb(39, 39, 39)
        } else {
            egui::Color32::from_rgb(218, 218, 218)
        },
        ..Default::default()
    };
    let mut cursor = 0;

    for span in spans {
        let start = span.range.start.min(text.len());
        let end = span.range.end.min(text.len());
        if start < cursor
            || end <= start
            || !text.is_char_boundary(start)
            || !text.is_char_boundary(end)
        {
            continue;
        }
        if cursor < start {
            job.append(&text[cursor..start], 0.0, plain_format.clone());
        }

        let mut format = plain_format.clone();
        format.color = match (span.kind, light_theme) {
            (HighlightKind::Keyword, false) => egui::Color32::from_rgb(244, 244, 244),
            (HighlightKind::Function, false) => egui::Color32::from_rgb(216, 216, 216),
            (HighlightKind::Type, false) => egui::Color32::from_rgb(190, 190, 190),
            (HighlightKind::String, false) => egui::Color32::from_rgb(161, 161, 161),
            (HighlightKind::Number, false) => egui::Color32::from_rgb(201, 201, 201),
            (HighlightKind::Comment, false) => egui::Color32::from_rgb(86, 86, 86),
            (HighlightKind::Keyword, true) => egui::Color32::from_rgb(23, 23, 23),
            (HighlightKind::Function, true) => egui::Color32::from_rgb(64, 64, 64),
            (HighlightKind::Type, true) => egui::Color32::from_rgb(88, 88, 88),
            (HighlightKind::String, true) => egui::Color32::from_rgb(79, 79, 79),
            (HighlightKind::Number, true) => egui::Color32::from_rgb(100, 100, 100),
            (HighlightKind::Comment, true) => egui::Color32::from_rgb(111, 111, 111),
        };
        job.append(&text[start..end], 0.0, format);
        cursor = end;
    }

    if cursor < text.len() {
        job.append(&text[cursor..], 0.0, plain_format);
    }
    job
}

#[cfg(test)]
mod tests {
    use super::{info, spans, MAX_SYNTAX_BYTES};
    use std::path::Path;

    #[test]
    fn skips_synchronous_parsing_for_large_files() {
        let text = " ".repeat(MAX_SYNTAX_BYTES + 1);
        assert!(spans(Some(Path::new("large.rs")), &text).is_empty());
        assert_eq!(
            info(Some(Path::new("large.rs")), &text),
            "syntax disabled for large file"
        );
    }
}
