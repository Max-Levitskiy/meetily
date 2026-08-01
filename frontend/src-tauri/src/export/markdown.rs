//! Markdown rendering for meeting exports.
//!
//! Nothing here touches Tauri or the database so the formatting rules can be
//! unit tested on their own.

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Keys inside `summary_processes.result` that are not user-visible sections.
const NON_SECTION_KEYS: &[&str] = &[
    "markdown",
    "summary_json",
    "_section_order",
    "MeetingName",
    "english_cache",
];

/// Controls which parts of a meeting end up in the exported document.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ExportOptions {
    pub include_metadata: bool,
    pub include_summary: bool,
    pub include_transcript: bool,
    pub include_timestamps: bool,
    pub include_speakers: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            include_metadata: true,
            include_summary: true,
            include_transcript: true,
            include_timestamps: true,
            include_speakers: true,
        }
    }
}

/// A transcript segment flattened to just what the exporter needs.
#[derive(Debug, Clone)]
pub struct TranscriptLine {
    pub text: String,
    pub timestamp: String,
    pub audio_start_time: Option<f64>,
    pub speaker: Option<String>,
}

/// Everything the exporter needs about one meeting.
#[derive(Debug, Clone)]
pub struct MeetingExportData {
    pub id: String,
    pub title: String,
    /// RFC 3339 creation timestamp.
    pub created_at: String,
    /// Raw `summary_processes.result` JSON, when a summary exists.
    pub summary: Option<Value>,
    pub transcripts: Vec<TranscriptLine>,
}

/// Builds the full Markdown document for a meeting.
pub fn build_meeting_markdown(data: &MeetingExportData, options: &ExportOptions) -> String {
    let mut out = String::new();

    let title = match data.title.trim() {
        "" => "Untitled Meeting",
        title => title,
    };
    out.push_str(&format!("# {}\n\n", title));

    if options.include_metadata {
        out.push_str(&format!("- **Date:** {}\n", format_datetime(&data.created_at)));
        out.push_str(&format!("- **Meeting ID:** `{}`\n", data.id));
        if !data.transcripts.is_empty() {
            out.push_str(&format!("- **Segments:** {}\n", data.transcripts.len()));
        }
        out.push_str(&format!(
            "- **Exported:** {}\n\n",
            Local::now().format("%Y-%m-%d %H:%M")
        ));
    }

    let summary = if options.include_summary {
        data.summary.as_ref().and_then(summary_to_markdown)
    } else {
        None
    };

    if let Some(summary) = summary {
        out.push_str("---\n\n## Summary\n\n");
        out.push_str(&summary);
        out.push_str("\n\n");
    }

    if options.include_transcript && !data.transcripts.is_empty() {
        out.push_str("---\n\n## Transcript\n\n");
        for line in &data.transcripts {
            let text = line.text.trim();
            if text.is_empty() {
                continue;
            }

            let mut prefix = String::new();

            if options.include_timestamps {
                let stamp = match line.audio_start_time {
                    Some(offset) => format_offset(offset),
                    None => line.timestamp.trim().to_string(),
                };
                if !stamp.is_empty() {
                    prefix.push_str(&format!("**{}** ", stamp));
                }
            }

            if options.include_speakers {
                if let Some(speaker) = line
                    .speaker
                    .as_deref()
                    .map(str::trim)
                    .filter(|speaker| !speaker.is_empty())
                {
                    prefix.push_str(&format!("**{}:** ", speaker_label(speaker)));
                }
            }

            out.push_str(&format!("{}{}\n\n", prefix, text));
        }
    }

    format!("{}\n", out.trim_end())
}

/// Builds a filesystem-safe default filename such as `team-standup-2026-08-01.md`.
pub fn suggest_filename(title: &str, created_at: &str) -> String {
    let mut slug = String::new();
    let mut trailing_dash = false;

    for ch in title.trim().chars() {
        if ch.is_alphanumeric() {
            slug.extend(ch.to_lowercase());
            trailing_dash = false;
        } else if !trailing_dash && !slug.is_empty() {
            slug.push('-');
            trailing_dash = true;
        }
    }

    let slug: String = slug.chars().take(60).collect();
    let slug = match slug.trim_matches('-') {
        "" => "meeting",
        slug => slug,
    };

    let date = DateTime::parse_from_rfc3339(created_at)
        .map(|dt| dt.with_timezone(&Local).format("%Y-%m-%d").to_string())
        .unwrap_or_else(|_| Local::now().format("%Y-%m-%d").to_string());

    format!("{}-{}.md", slug, date)
}

/// Converts a `summary_processes.result` blob into Markdown.
///
/// Handles the three shapes the app has written over time: a plain `markdown`
/// string (what generation produces today), BlockNote `summary_json` blocks
/// (saved edits whose Markdown conversion failed), and the original
/// per-section block format.
pub fn summary_to_markdown(result: &Value) -> Option<String> {
    if let Some(markdown) = result.get("markdown").and_then(Value::as_str) {
        let markdown = markdown.trim();
        if !markdown.is_empty() {
            return Some(markdown.to_string());
        }
    }

    if let Some(blocks) = result.get("summary_json").and_then(Value::as_array) {
        let rendered = blocknote_to_markdown(blocks);
        if !rendered.trim().is_empty() {
            return Some(rendered.trim().to_string());
        }
    }

    let legacy = legacy_sections_to_markdown(result);
    match legacy.trim() {
        "" => None,
        legacy => Some(legacy.to_string()),
    }
}

/// Formats a recording-relative offset as `[MM:SS]`, widening past an hour.
fn format_offset(seconds: f64) -> String {
    let total = seconds.max(0.0).floor() as u64;
    let (hours, minutes, secs) = (total / 3600, (total % 3600) / 60, total % 60);

    if hours > 0 {
        format!("[{:02}:{:02}:{:02}]", hours, minutes, secs)
    } else {
        format!("[{:02}:{:02}]", minutes, secs)
    }
}

/// Maps the stored audio source onto a human-readable label.
fn speaker_label(speaker: &str) -> String {
    match speaker {
        "mic" => "Microphone".to_string(),
        "system" => "System Audio".to_string(),
        other => other.to_string(),
    }
}

/// Renders an RFC 3339 timestamp in local time, falling back to the raw string.
fn format_datetime(raw: &str) -> String {
    DateTime::parse_from_rfc3339(raw)
        .map(|dt| {
            dt.with_timezone(&Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|_| raw.to_string())
}

fn blocknote_to_markdown(blocks: &[Value]) -> String {
    let mut out = String::new();
    render_blocks(blocks, 0, &mut out);
    out
}

fn render_blocks(blocks: &[Value], depth: usize, out: &mut String) {
    let indent = "  ".repeat(depth);
    let mut ordinal = 0usize;
    let mut previous_was_list = false;

    for block in blocks {
        let block_type = block
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("paragraph");

        let is_list = matches!(
            block_type,
            "bulletListItem" | "numberedListItem" | "checkListItem"
        );

        // A list needs a blank line before whatever follows it, otherwise the
        // next paragraph is absorbed into the final list item.
        if previous_was_list && !is_list {
            out.push('\n');
        }

        ordinal = if block_type == "numberedListItem" {
            ordinal + 1
        } else {
            0
        };

        let text = inline_content_to_markdown(block.get("content"));

        match block_type {
            "heading" => {
                let level = block
                    .pointer("/props/level")
                    .and_then(Value::as_u64)
                    .unwrap_or(1)
                    .clamp(1, 6) as usize;
                if !text.is_empty() {
                    out.push_str(&format!("{}{} {}\n\n", indent, "#".repeat(level), text));
                }
            }
            "bulletListItem" => out.push_str(&format!("{}- {}\n", indent, text)),
            "numberedListItem" => out.push_str(&format!("{}{}. {}\n", indent, ordinal, text)),
            "checkListItem" => {
                let checked = block
                    .pointer("/props/checked")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                out.push_str(&format!(
                    "{}- [{}] {}\n",
                    indent,
                    if checked { "x" } else { " " },
                    text
                ));
            }
            "quote" => {
                if !text.is_empty() {
                    out.push_str(&format!("{}> {}\n\n", indent, text));
                }
            }
            "codeBlock" => {
                let language = block
                    .pointer("/props/language")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let code = inline_content_to_plain_text(block.get("content"));
                out.push_str(&format!(
                    "{}```{}\n{}\n{}```\n\n",
                    indent, language, code, indent
                ));
            }
            "table" => render_table(block, &indent, out),
            _ => {
                if !text.is_empty() {
                    out.push_str(&format!("{}{}\n\n", indent, text));
                }
            }
        }

        previous_was_list = is_list;

        if let Some(children) = block.get("children").and_then(Value::as_array) {
            if !children.is_empty() {
                render_blocks(children, depth + 1, out);
            }
        }
    }

    if previous_was_list {
        out.push('\n');
    }
}

fn render_table(block: &Value, indent: &str, out: &mut String) {
    let Some(rows) = block.pointer("/content/rows").and_then(Value::as_array) else {
        return;
    };

    let rendered: Vec<Vec<String>> = rows
        .iter()
        .filter_map(|row| row.get("cells").and_then(Value::as_array))
        .map(|cells| {
            cells
                .iter()
                .map(|cell| {
                    // A cell is either raw inline content or `{ content: [...] }`.
                    let inline = cell.get("content").unwrap_or(cell);
                    inline_content_to_markdown(Some(inline)).replace('|', "\\|")
                })
                .collect()
        })
        .collect();

    let Some(header) = rendered.first() else {
        return;
    };

    out.push_str(&format!("{}| {} |\n", indent, header.join(" | ")));
    out.push_str(&format!("{}|{}\n", indent, " --- |".repeat(header.len())));
    for row in rendered.iter().skip(1) {
        out.push_str(&format!("{}| {} |\n", indent, row.join(" | ")));
    }
    out.push('\n');
}

fn inline_content_to_markdown(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(items)) => items.iter().map(inline_item_to_markdown).collect(),
        _ => String::new(),
    }
}

fn inline_item_to_markdown(item: &Value) -> String {
    if item.get("type").and_then(Value::as_str) == Some("link") {
        let label = inline_content_to_markdown(item.get("content"));
        let href = item.get("href").and_then(Value::as_str).unwrap_or_default();
        return if href.is_empty() {
            label
        } else {
            format!("[{}]({})", label, href)
        };
    }

    let text = item.get("text").and_then(Value::as_str).unwrap_or_default();
    apply_styles(text, item.get("styles"))
}

fn apply_styles(text: &str, styles: Option<&Value>) -> String {
    if text.is_empty() {
        return String::new();
    }

    let Some(styles) = styles else {
        return text.to_string();
    };
    let enabled = |key: &str| styles.get(key).and_then(Value::as_bool).unwrap_or(false);

    // `code` wins outright: emphasis markers inside a code span render literally.
    if enabled("code") {
        return format!("`{}`", text);
    }

    let mut out = text.to_string();
    if enabled("bold") {
        out = format!("**{}**", out);
    }
    if enabled("italic") {
        out = format!("*{}*", out);
    }
    if enabled("strike") {
        out = format!("~~{}~~", out);
    }
    out
}

fn inline_content_to_plain_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| match item.get("text").and_then(Value::as_str) {
                Some(text) => text.to_string(),
                None => inline_content_to_plain_text(item.get("content")),
            })
            .collect(),
        _ => String::new(),
    }
}

/// Renders the original summary format: a map of section key to
/// `{ title, blocks: [{ type, content }] }`.
fn legacy_sections_to_markdown(result: &Value) -> String {
    let Some(map) = result.as_object() else {
        return String::new();
    };

    // `_section_order` preserves the author's ordering when the app wrote one.
    let keys: Vec<String> = match result.get("_section_order").and_then(Value::as_array) {
        Some(order) => order
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        None => map.keys().cloned().collect(),
    };

    let mut out = String::new();

    for key in keys {
        if NON_SECTION_KEYS.contains(&key.as_str()) {
            continue;
        }

        let Some(section) = map.get(&key) else {
            continue;
        };
        let Some(blocks) = section.get("blocks").and_then(Value::as_array) else {
            continue;
        };

        let title = section
            .get("title")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .unwrap_or(&key);

        let mut body = String::new();
        let mut previous_was_bullet = false;

        for block in blocks {
            let content = block
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            if content.is_empty() {
                continue;
            }

            let block_type = block.get("type").and_then(Value::as_str).unwrap_or("text");
            let is_bullet = block_type == "bullet";

            if previous_was_bullet && !is_bullet {
                body.push('\n');
            }

            match block_type {
                "heading1" => body.push_str(&format!("### {}\n\n", content)),
                "heading2" => body.push_str(&format!("#### {}\n\n", content)),
                "bullet" => body.push_str(&format!("- {}\n", content)),
                _ => body.push_str(&format!("{}\n\n", content)),
            }

            previous_was_bullet = is_bullet;
        }

        if body.trim().is_empty() {
            continue;
        }

        out.push_str(&format!("## {}\n\n{}\n\n", title, body.trim_end()));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn meeting(summary: Option<Value>, transcripts: Vec<TranscriptLine>) -> MeetingExportData {
        MeetingExportData {
            id: "meeting-123".to_string(),
            title: "Team Standup".to_string(),
            created_at: "2026-08-01T09:30:00Z".to_string(),
            summary,
            transcripts,
        }
    }

    fn line(text: &str, offset: Option<f64>, speaker: Option<&str>) -> TranscriptLine {
        TranscriptLine {
            text: text.to_string(),
            timestamp: "09:30:00".to_string(),
            audio_start_time: offset,
            speaker: speaker.map(str::to_string),
        }
    }

    #[test]
    fn formats_offsets_as_minutes_and_seconds() {
        assert_eq!(format_offset(0.0), "[00:00]");
        assert_eq!(format_offset(65.9), "[01:05]");
        assert_eq!(format_offset(3725.0), "[01:02:05]");
        // Negative offsets should clamp rather than underflow the cast.
        assert_eq!(format_offset(-5.0), "[00:00]");
    }

    #[test]
    fn maps_audio_sources_to_readable_labels() {
        assert_eq!(speaker_label("mic"), "Microphone");
        assert_eq!(speaker_label("system"), "System Audio");
        assert_eq!(speaker_label("Alice"), "Alice");
    }

    #[test]
    fn prefers_the_markdown_field() {
        let result = json!({
            "markdown": "## Decisions\n\nShip on Friday.",
            "summary_json": [{ "type": "paragraph", "content": [{ "type": "text", "text": "ignored" }] }],
        });

        assert_eq!(
            summary_to_markdown(&result).unwrap(),
            "## Decisions\n\nShip on Friday."
        );
    }

    #[test]
    fn falls_back_to_blocknote_blocks() {
        let result = json!({
            "markdown": "   ",
            "summary_json": [
                { "type": "heading", "props": { "level": 2 },
                  "content": [{ "type": "text", "text": "Action Items" }] },
                { "type": "bulletListItem",
                  "content": [{ "type": "text", "text": "Ship it", "styles": { "bold": true } }] },
                { "type": "bulletListItem",
                  "content": [{ "type": "text", "text": "Then rest" }] },
                { "type": "paragraph", "content": [{ "type": "text", "text": "Done." }] },
            ],
        });

        assert_eq!(
            summary_to_markdown(&result).unwrap(),
            "## Action Items\n\n- **Ship it**\n- Then rest\n\nDone."
        );
    }

    #[test]
    fn renders_blocknote_links_checklists_and_code() {
        let blocks = json!([
            { "type": "checkListItem", "props": { "checked": true },
              "content": [{ "type": "text", "text": "Booked" }] },
            { "type": "checkListItem", "props": { "checked": false },
              "content": [{ "type": "text", "text": "Pending" }] },
            { "type": "paragraph", "content": [
                { "type": "link", "href": "https://example.com",
                  "content": [{ "type": "text", "text": "notes" }] }
            ]},
            { "type": "codeBlock", "props": { "language": "rust" },
              "content": [{ "type": "text", "text": "let x = 1;", "styles": { "bold": true } }] },
        ]);

        let rendered = blocknote_to_markdown(blocks.as_array().unwrap());

        assert!(rendered.contains("- [x] Booked\n"));
        assert!(rendered.contains("- [ ] Pending\n"));
        assert!(rendered.contains("[notes](https://example.com)"));
        // Code block content must stay literal, not pick up the bold style.
        assert!(rendered.contains("```rust\nlet x = 1;\n```"));
    }

    #[test]
    fn renders_nested_blocknote_children() {
        let blocks = json!([
            { "type": "bulletListItem", "content": [{ "type": "text", "text": "Parent" }],
              "children": [
                  { "type": "bulletListItem", "content": [{ "type": "text", "text": "Child" }] }
              ]},
        ]);

        let rendered = blocknote_to_markdown(blocks.as_array().unwrap());

        assert!(rendered.contains("- Parent\n"));
        assert!(rendered.contains("  - Child\n"));
    }

    #[test]
    fn renders_blocknote_tables() {
        let blocks = json!([
            { "type": "table", "content": { "type": "tableContent", "rows": [
                { "cells": [[{ "type": "text", "text": "Owner" }], [{ "type": "text", "text": "Task" }]] },
                { "cells": [[{ "type": "text", "text": "Max" }], [{ "type": "text", "text": "Ship" }]] },
            ]}},
        ]);

        let rendered = blocknote_to_markdown(blocks.as_array().unwrap());

        assert!(rendered.contains("| Owner | Task |\n"));
        assert!(rendered.contains("| --- | --- |\n"));
        assert!(rendered.contains("| Max | Ship |\n"));
    }

    #[test]
    fn falls_back_to_the_legacy_section_format() {
        let result = json!({
            "MeetingName": "Team Standup",
            "Agenda": {
                "title": "Agenda",
                "blocks": [
                    { "type": "bullet", "content": "Roadmap" },
                    { "type": "bullet", "content": "Hiring" },
                ],
            },
            "Decisions": {
                "title": "Decisions",
                "blocks": [{ "type": "text", "content": "Ship on Friday." }],
            },
            "_section_order": ["Agenda", "Decisions"],
        });

        assert_eq!(
            summary_to_markdown(&result).unwrap(),
            "## Agenda\n\n- Roadmap\n- Hiring\n\n## Decisions\n\nShip on Friday."
        );
    }

    #[test]
    fn returns_none_when_there_is_no_summary_content() {
        assert!(summary_to_markdown(&json!({ "markdown": "  " })).is_none());
        assert!(summary_to_markdown(&json!({})).is_none());
        // A cache-only blob carries no user-visible sections.
        assert!(summary_to_markdown(&json!({ "english_cache": { "markdown": "x" } })).is_none());
    }

    #[test]
    fn builds_a_document_with_summary_and_transcript() {
        let data = meeting(
            Some(json!({ "markdown": "## Decisions\n\nShip on Friday." })),
            vec![
                line("Morning everyone.", Some(0.0), Some("mic")),
                line("Hello.", Some(12.0), Some("system")),
            ],
        );

        let rendered = build_meeting_markdown(&data, &ExportOptions::default());

        assert!(rendered.starts_with("# Team Standup\n\n"));
        assert!(rendered.contains("- **Meeting ID:** `meeting-123`\n"));
        assert!(rendered.contains("- **Segments:** 2\n"));
        assert!(rendered.contains("## Summary\n\n## Decisions\n\nShip on Friday."));
        assert!(rendered.contains("**[00:00]** **Microphone:** Morning everyone.\n"));
        assert!(rendered.contains("**[00:12]** **System Audio:** Hello.\n"));
        assert!(rendered.ends_with("Hello.\n"));
    }

    #[test]
    fn honours_section_toggles() {
        let data = meeting(
            Some(json!({ "markdown": "Summary body." })),
            vec![line("Spoken words.", Some(0.0), Some("mic"))],
        );

        let summary_only = build_meeting_markdown(
            &data,
            &ExportOptions {
                include_transcript: false,
                ..Default::default()
            },
        );
        assert!(summary_only.contains("## Summary"));
        assert!(!summary_only.contains("## Transcript"));

        let transcript_only = build_meeting_markdown(
            &data,
            &ExportOptions {
                include_summary: false,
                include_metadata: false,
                ..Default::default()
            },
        );
        assert!(!transcript_only.contains("## Summary"));
        assert!(!transcript_only.contains("**Meeting ID:**"));
        assert!(transcript_only.contains("## Transcript"));

        let bare = build_meeting_markdown(
            &data,
            &ExportOptions {
                include_timestamps: false,
                include_speakers: false,
                ..Default::default()
            },
        );
        assert!(bare.contains("\nSpoken words.\n"));
        assert!(!bare.contains("[00:00]"));
        assert!(!bare.contains("Microphone"));
    }

    #[test]
    fn falls_back_to_the_wall_clock_timestamp_without_an_offset() {
        let data = meeting(None, vec![line("Legacy segment.", None, None)]);
        let rendered = build_meeting_markdown(&data, &ExportOptions::default());

        assert!(rendered.contains("**09:30:00** Legacy segment."));
    }

    #[test]
    fn skips_empty_segments_and_untitled_meetings_still_render() {
        let mut data = meeting(None, vec![line("   ", Some(0.0), None)]);
        data.title = "   ".to_string();

        let rendered = build_meeting_markdown(&data, &ExportOptions::default());

        assert!(rendered.starts_with("# Untitled Meeting\n"));
        assert!(!rendered.contains("[00:00]"));
    }

    #[test]
    fn partial_option_payloads_fall_back_to_defaults() {
        // The UI only sends the toggles that vary between export scopes.
        let options: ExportOptions =
            serde_json::from_str(r#"{"includeSummary":false,"includeTranscript":true}"#).unwrap();

        assert!(!options.include_summary);
        assert!(options.include_transcript);
        assert!(options.include_metadata);
        assert!(options.include_timestamps);
        assert!(options.include_speakers);
    }

    #[test]
    fn suggests_a_slugged_filename() {
        assert_eq!(
            suggest_filename("Team Standup", "2026-08-01T09:30:00Z"),
            "team-standup-2026-08-01.md"
        );
        assert_eq!(
            suggest_filename("  Q3 // Planning!!  ", "2026-08-01T09:30:00Z"),
            "q3-planning-2026-08-01.md"
        );
        assert!(suggest_filename("***", "2026-08-01T09:30:00Z").starts_with("meeting-"));
    }
}
