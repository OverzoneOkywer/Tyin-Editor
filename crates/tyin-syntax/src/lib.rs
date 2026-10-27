use anyhow::Result;
use std::ops::Range;
use tree_sitter::{Language, Node, Parser};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HighlightKind {
    Keyword,
    String,
    Comment,
    Number,
    Type,
    Function,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighlightSpan {
    pub range: Range<usize>,
    pub kind: HighlightKind,
}

#[derive(Default)]
pub struct SyntaxEngine {
    enabled: bool,
}

impl SyntaxEngine {
    pub fn new() -> Self {
        Self { enabled: true }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    fn get_lang(ext: &str) -> Option<Language> {
        match ext {
            "rs" => Some(tree_sitter_rust::LANGUAGE.into()),
            "json" => Some(tree_sitter_json::LANGUAGE.into()),
            "toml" => Some(tree_sitter_toml_ng::LANGUAGE.into()),
            _ => None,
        }
    }

    pub fn parse_preview(&self, code: &str, ext: &str) -> Result<String> {
        let Some(lang) = Self::get_lang(ext) else {
            return Ok("plain text".to_string());
        };
        let mut parser = Parser::new();
        parser.set_language(&lang)?;
        let tree = parser.parse(code, None);
        Ok(tree.map_or_else(
            || format!("{} (parse pending)", ext),
            |tree| {
                format!(
                    "Tree-sitter: {} nodes, lang={}",
                    tree.root_node().child_count() + 1,
                    ext
                )
            },
        ))
    }

    pub fn highlight(&self, code: &str, ext: &str) -> Result<Vec<HighlightSpan>> {
        let Some(lang) = Self::get_lang(ext) else {
            return Ok(Vec::new());
        };
        let mut parser = Parser::new();
        parser.set_language(&lang)?;
        let Some(tree) = parser.parse(code, None) else {
            return Ok(Vec::new());
        };

        let mut spans = Vec::new();
        collect_spans(tree.root_node(), &mut spans);
        spans.sort_by_key(|span| span.range.start);
        Ok(spans)
    }
}

fn collect_spans(node: Node<'_>, spans: &mut Vec<HighlightSpan>) {
    if node.kind() == "function_item" {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "identifier" {
                spans.push(HighlightSpan {
                    range: child.byte_range(),
                    kind: HighlightKind::Function,
                });
            } else {
                collect_spans(child, spans);
            }
        }
        return;
    }

    if let Some(kind) = classify_node(node.kind()) {
        if node.child_count() == 0 || matches!(kind, HighlightKind::Comment | HighlightKind::String)
        {
            spans.push(HighlightSpan {
                range: node.byte_range(),
                kind,
            });
            return;
        }
    }

    if node.child_count() == 0 {
        return;
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_spans(child, spans);
    }
}

fn classify_node(kind: &str) -> Option<HighlightKind> {
    if kind.contains("comment") {
        Some(HighlightKind::Comment)
    } else if kind.contains("string") || kind == "escape_sequence" {
        Some(HighlightKind::String)
    } else if kind.contains("integer") || kind.contains("float") || kind.contains("number") {
        Some(HighlightKind::Number)
    } else if matches!(
        kind,
        "fn" | "let"
            | "mut"
            | "pub"
            | "use"
            | "mod"
            | "struct"
            | "enum"
            | "impl"
            | "trait"
            | "type"
            | "const"
            | "static"
            | "if"
            | "else"
            | "match"
            | "for"
            | "while"
            | "loop"
            | "return"
            | "break"
            | "continue"
            | "async"
            | "await"
            | "move"
            | "where"
            | "self"
            | "Self"
            | "true"
            | "false"
            | "null"
            | "in"
    ) {
        Some(HighlightKind::Keyword)
    } else if kind.contains("type") || kind == "primitive_type" || kind == "type_identifier" {
        Some(HighlightKind::Type)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{HighlightKind, SyntaxEngine};

    #[test]
    fn highlights_rust_tokens_from_tree_sitter() {
        let source = "fn main() { let count: u32 = 42; // note\n}";
        let spans = SyntaxEngine::new().highlight(source, "rs").unwrap();

        let tokens: Vec<_> = spans
            .iter()
            .map(|span| (&source[span.range.clone()], span.kind))
            .collect();
        assert!(tokens.contains(&("fn", HighlightKind::Keyword)));
        assert!(tokens.contains(&("42", HighlightKind::Number)));
        assert!(
            tokens.contains(&("// note", HighlightKind::Comment)),
            "highlighted tokens: {tokens:?}"
        );
        assert!(tokens.contains(&("u32", HighlightKind::Type)));
        assert!(tokens.contains(&("main", HighlightKind::Function)));
    }

    #[test]
    fn unknown_languages_use_plain_text_without_failing() {
        assert!(SyntaxEngine::new()
            .highlight("hello", "txt")
            .unwrap()
            .is_empty());
        assert_eq!(
            SyntaxEngine::new().parse_preview("hello", "txt").unwrap(),
            "plain text"
        );
    }
}
