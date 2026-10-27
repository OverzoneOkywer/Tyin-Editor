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

pub(super) fn layout(text: &str, spans: &[HighlightSpan], wrap_width: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap_width;
    let mut plain_format = TextFormat::default();
    plain_format.font_id = egui::FontId::monospace(14.0);
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
        format.color = match span.kind {
            HighlightKind::Keyword => egui::Color32::from_rgb(198, 120, 221),
            HighlightKind::String => egui::Color32::from_rgb(152, 195, 121),
            HighlightKind::Comment => egui::Color32::from_rgb(128, 138, 150),
            HighlightKind::Number => egui::Color32::from_rgb(209, 154, 102),
            HighlightKind::Type => egui::Color32::from_rgb(86, 182, 194),
            HighlightKind::Function => egui::Color32::from_rgb(97, 175, 239),
        };
        job.append(&text[start..end], 0.0, format);
        cursor = end;
    }

    if cursor < text.len() {
        job.append(&text[cursor..], 0.0, plain_format);
    }
    job
}
