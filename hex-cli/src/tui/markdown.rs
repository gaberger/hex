//! Markdown → ratatui Lines converter for the hex chat TUI.
//!
//! Converts assistant response markdown into styled ratatui `Line`s.
//! Handles: **bold**, *italic*, `inline code`, fenced code blocks,
//! # headings, - bullet lists, paragraphs, and line breaks.

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use ratatui::prelude::*;

/// Convert a markdown string to styled ratatui Lines.
///
/// `width` is the available terminal width — used to size code block borders.
pub fn render_markdown(text: &str, width: u16) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut current_spans: Vec<Span<'static>> = Vec::new();

    // Style stack — innermost style wins for nested markup
    let mut bold = false;
    let mut italic = false;
    let mut in_code_block = false;
    let mut code_lang = String::new();
    let mut in_list_item = false;
    let mut list_prefix_pending = false;

    let opts = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let parser = Parser::new_ext(text, opts);

    for event in parser {
        match event {
            // ── Block starts ──────────────────────────────────────────────
            Event::Start(Tag::CodeBlock(kind)) => {
                // Flush any pending inline content
                if !current_spans.is_empty() {
                    lines.push(Line::from(std::mem::take(&mut current_spans)));
                }
                in_code_block = true;
                code_lang = match kind {
                    CodeBlockKind::Fenced(lang) => lang.to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                // Code block header
                let lang_label = if code_lang.is_empty() {
                    "code".to_string()
                } else {
                    code_lang.clone()
                };
                let fill_width = (width as usize).saturating_sub(lang_label.len() + 8);
                let header = format!(
                    "  ╭─ {} {}╮",
                    lang_label,
                    "─".repeat(fill_width)
                );
                lines.push(Line::from(Span::styled(
                    header,
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::DIM),
                )));
            }

            Event::End(TagEnd::CodeBlock) => {
                in_code_block = false;
                let fill_width = (width as usize).saturating_sub(4);
                let footer = format!("  ╰{}╯", "─".repeat(fill_width));
                lines.push(Line::from(Span::styled(
                    footer,
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::DIM),
                )));
                lines.push(Line::from(""));
                code_lang.clear();
            }

            Event::Start(Tag::Paragraph) => {
                // Nothing to do on start; content arrives as Text events
            }

            Event::End(TagEnd::Paragraph) => {
                if !current_spans.is_empty() {
                    lines.push(Line::from(std::mem::take(&mut current_spans)));
                }
                lines.push(Line::from(""));
            }

            Event::Start(Tag::Heading { .. }) => {
                current_spans.push(Span::styled(
                    "  ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ));
                bold = true;
            }

            Event::End(TagEnd::Heading(_)) => {
                bold = false;
                if !current_spans.is_empty() {
                    lines.push(Line::from(std::mem::take(&mut current_spans)));
                }
                lines.push(Line::from(""));
            }

            Event::Start(Tag::Strong) => {
                bold = true;
            }
            Event::End(TagEnd::Strong) => {
                bold = false;
            }

            Event::Start(Tag::Emphasis) => {
                italic = true;
            }
            Event::End(TagEnd::Emphasis) => {
                italic = false;
            }

            Event::Start(Tag::List(_)) => {
                in_list_item = true;
            }
            Event::End(TagEnd::List(_)) => {
                in_list_item = false;
                lines.push(Line::from(""));
            }

            Event::Start(Tag::Item) => {
                list_prefix_pending = true;
            }
            Event::End(TagEnd::Item) => {
                if !current_spans.is_empty() {
                    lines.push(Line::from(std::mem::take(&mut current_spans)));
                }
                list_prefix_pending = false;
            }

            Event::Start(Tag::BlockQuote(_)) => {
                current_spans.push(Span::styled(
                    "  ▎ ",
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::DIM),
                ));
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                if !current_spans.is_empty() {
                    lines.push(Line::from(std::mem::take(&mut current_spans)));
                }
                lines.push(Line::from(""));
            }

            // ── Inline code ───────────────────────────────────────────────
            Event::Code(code) => {
                current_spans.push(Span::styled(
                    format!(" {} ", code.as_ref()),
                    Style::default()
                        .fg(Color::LightGreen)
                        .bg(Color::Rgb(40, 40, 40)),
                ));
            }

            // ── Text ──────────────────────────────────────────────────────
            Event::Text(text) => {
                if in_code_block {
                    // Each line of a code block gets a left gutter
                    for line in text.lines() {
                        let spans = vec![
                            Span::styled(
                                "  │ ",
                                Style::default()
                                    .fg(Color::DarkGray)
                                    .add_modifier(Modifier::DIM),
                            ),
                            Span::styled(
                                line.to_string(),
                                Style::default().fg(Color::White),
                            ),
                        ];
                        lines.push(Line::from(spans));
                    }
                } else {
                    // Prepend list bullet if needed
                    if list_prefix_pending {
                        current_spans.push(Span::styled(
                            "  • ",
                            Style::default().fg(Color::Cyan),
                        ));
                        list_prefix_pending = false;
                    }

                    let style = match (bold, italic) {
                        (true, true) => Style::default()
                            .add_modifier(Modifier::BOLD)
                            .add_modifier(Modifier::ITALIC),
                        (true, false) => Style::default().add_modifier(Modifier::BOLD),
                        (false, true) => Style::default().add_modifier(Modifier::ITALIC),
                        (false, false) => Style::default(),
                    };
                    current_spans.push(Span::styled(text.into_string(), style));
                }
            }

            // ── Line breaks ───────────────────────────────────────────────
            Event::SoftBreak => {
                if !in_code_block && !current_spans.is_empty() {
                    lines.push(Line::from(std::mem::take(&mut current_spans)));
                }
            }

            Event::HardBreak => {
                if !current_spans.is_empty() {
                    lines.push(Line::from(std::mem::take(&mut current_spans)));
                }
            }

            Event::Rule => {
                let rule = "─".repeat(width.saturating_sub(4) as usize);
                lines.push(Line::from(Span::styled(
                    format!("  {}", rule),
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::DIM),
                )));
            }

            _ => {}
        }
    }

    // Suppress unused variable warnings for flags only used via pattern matching
    let _ = in_list_item;

    // Flush any remaining inline content
    if !current_spans.is_empty() {
        lines.push(Line::from(current_spans));
    }

    lines
}
