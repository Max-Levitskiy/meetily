//! Tauri commands for exporting a meeting to a file.

use log::{error, info, warn};
use serde::Serialize;
use sqlx::SqlitePool;
use std::path::Path;
use tauri::{AppHandle, Runtime};

use crate::database::repositories::meeting::MeetingsRepository;
use crate::database::repositories::summary::SummaryProcessesRepository;
use crate::state::AppState;

use super::markdown::{
    build_meeting_markdown, suggest_filename, ExportOptions, MeetingExportData, TranscriptLine,
};

/// A rendered export that has not been written to disk.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingMarkdownExport {
    pub markdown: String,
    pub suggested_filename: String,
}

/// Renders a meeting as Markdown and returns it without touching the filesystem.
#[tauri::command]
pub async fn export_meeting_markdown(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    options: Option<ExportOptions>,
) -> Result<MeetingMarkdownExport, String> {
    let options = options.unwrap_or_default();
    let data = load_export_data(state.db_manager.pool(), &meeting_id).await?;

    Ok(MeetingMarkdownExport {
        suggested_filename: suggest_filename(&data.title, &data.created_at),
        markdown: build_meeting_markdown(&data, &options),
    })
}

/// Renders a meeting as Markdown and writes it to a user-chosen file.
///
/// Returns the saved path, or `None` when the user cancels the save dialog.
#[tauri::command]
pub async fn save_meeting_markdown<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    options: Option<ExportOptions>,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let options = options.unwrap_or_default();
    let data = load_export_data(state.db_manager.pool(), &meeting_id).await?;
    let filename = suggest_filename(&data.title, &data.created_at);
    let markdown = build_meeting_markdown(&data, &options);

    let Some(target) = app
        .dialog()
        .file()
        .set_file_name(filename.as_str())
        .add_filter("Markdown", &["md"])
        .blocking_save_file()
    else {
        info!("Markdown export cancelled for meeting {}", meeting_id);
        return Ok(None);
    };

    let path = target
        .into_path()
        .map_err(|e| format!("Invalid save location: {}", e))?;

    write_markdown_file(&path, &markdown)?;

    let saved = path.to_string_lossy().to_string();
    info!("Exported meeting {} to {}", meeting_id, saved);
    Ok(Some(saved))
}

/// Writes rendered Markdown to disk. Shared with the tests so both go through
/// the same path.
fn write_markdown_file(path: &Path, markdown: &str) -> Result<(), String> {
    std::fs::write(path, markdown).map_err(|e| {
        error!("Failed to write Markdown export to {:?}: {}", path, e);
        format!("Failed to write file: {}", e)
    })
}

/// Gathers the meeting, its transcript segments and its stored summary.
async fn load_export_data(
    pool: &SqlitePool,
    meeting_id: &str,
) -> Result<MeetingExportData, String> {
    let meeting = MeetingsRepository::get_meeting_metadata(pool, meeting_id)
        .await
        .map_err(|e| format!("Failed to load meeting: {}", e))?
        .ok_or_else(|| format!("Meeting not found: {}", meeting_id))?;

    // Ordered so segments with a recording offset lead, matching playback order,
    // with insertion order breaking ties for pre-offset recordings.
    let rows = sqlx::query_as::<_, (String, String, Option<f64>, Option<String>)>(
        "SELECT transcript, timestamp, audio_start_time, speaker
         FROM transcripts
         WHERE meeting_id = ?
         ORDER BY audio_start_time IS NULL, audio_start_time ASC, rowid ASC",
    )
    .bind(meeting_id)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("Failed to load transcripts: {}", e))?;

    let transcripts = rows
        .into_iter()
        .map(|(text, timestamp, audio_start_time, speaker)| TranscriptLine {
            text,
            timestamp,
            audio_start_time,
            speaker,
        })
        .collect();

    // A missing or unparseable summary is not fatal — the transcript alone is
    // still worth exporting.
    let summary = match SummaryProcessesRepository::get_summary_data(pool, meeting_id).await {
        Ok(Some(process)) => process.result.and_then(|raw| {
            serde_json::from_str(&raw)
                .map_err(|e| {
                    warn!(
                        "Stored summary for meeting {} is not valid JSON ({}); exporting without it.",
                        meeting_id, e
                    );
                })
                .ok()
        }),
        Ok(None) => None,
        Err(e) => {
            warn!(
                "Failed to load summary for meeting {} ({}); exporting without it.",
                meeting_id, e
            );
            None
        }
    };

    Ok(MeetingExportData {
        id: meeting.id,
        title: meeting.title,
        created_at: meeting.created_at.0.to_rfc3339(),
        summary,
        transcripts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use sqlx::sqlite::SqlitePoolOptions;

    const MEETING_ID: &str = "meeting-1";

    /// Opens a file-backed database and applies the project's real migrations.
    ///
    /// A file rather than `sqlite::memory:` because each pooled connection to an
    /// in-memory database gets its own private schema.
    async fn migrated_pool(dir: &Path) -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .connect(&format!("sqlite://{}/test.db?mode=rwc", dir.display()))
            .await
            .expect("open test database");

        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("run migrations");

        pool
    }

    async fn insert_meeting(pool: &SqlitePool) {
        let created = Utc.with_ymd_and_hms(2026, 8, 1, 9, 30, 0).unwrap();

        sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at) VALUES (?, ?, ?, ?)")
            .bind(MEETING_ID)
            .bind("Team Standup")
            .bind(created)
            .bind(created)
            .execute(pool)
            .await
            .expect("insert meeting");
    }

    /// Inserts segments out of order so the query's ordering is actually proven.
    async fn insert_transcripts(pool: &SqlitePool) {
        let rows: [(&str, &str, &str, Option<f64>, Option<&str>); 3] = [
            ("t-2", "Second line.", "09:30:12", Some(12.0), Some("system")),
            ("t-3", "Legacy line.", "09:31:00", None, None),
            ("t-1", "First line.", "09:30:00", Some(0.0), Some("mic")),
        ];

        for (id, text, timestamp, audio_start_time, speaker) in rows {
            sqlx::query(
                "INSERT INTO transcripts (id, meeting_id, transcript, timestamp, audio_start_time, speaker)
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(id)
            .bind(MEETING_ID)
            .bind(text)
            .bind(timestamp)
            .bind(audio_start_time)
            .bind(speaker)
            .execute(pool)
            .await
            .expect("insert transcript");
        }
    }

    async fn insert_summary(pool: &SqlitePool, result: &str) {
        let now = Utc.with_ymd_and_hms(2026, 8, 1, 10, 0, 0).unwrap();

        sqlx::query(
            "INSERT INTO summary_processes (meeting_id, status, created_at, updated_at, result)
             VALUES (?, 'completed', ?, ?, ?)",
        )
        .bind(MEETING_ID)
        .bind(now)
        .bind(now)
        .bind(result)
        .execute(pool)
        .await
        .expect("insert summary");
    }

    #[tokio::test]
    async fn loads_meeting_transcripts_and_summary_from_the_database() {
        let dir = tempfile::tempdir().unwrap();
        let pool = migrated_pool(dir.path()).await;
        insert_meeting(&pool).await;
        insert_transcripts(&pool).await;
        insert_summary(&pool, r###"{"markdown":"## Decisions\n\nShip on Friday."}"###).await;

        let data = load_export_data(&pool, MEETING_ID).await.unwrap();

        assert_eq!(data.id, MEETING_ID);
        assert_eq!(data.title, "Team Standup");
        assert!(data.created_at.starts_with("2026-08-01T09:30:00"));

        // Segments carrying an offset lead, in offset order; the legacy segment
        // without one sorts last.
        let texts: Vec<&str> = data
            .transcripts
            .iter()
            .map(|line| line.text.as_str())
            .collect();
        assert_eq!(texts, ["First line.", "Second line.", "Legacy line."]);

        assert_eq!(data.transcripts[0].speaker.as_deref(), Some("mic"));
        assert_eq!(data.transcripts[1].speaker.as_deref(), Some("system"));
        assert_eq!(data.transcripts[2].speaker, None);
        assert_eq!(data.transcripts[2].audio_start_time, None);
        assert_eq!(data.transcripts[2].timestamp, "09:31:00");

        assert_eq!(
            data.summary.as_ref().unwrap()["markdown"],
            "## Decisions\n\nShip on Friday."
        );
    }

    #[tokio::test]
    async fn writes_a_markdown_file_for_a_meeting() {
        let dir = tempfile::tempdir().unwrap();
        let pool = migrated_pool(dir.path()).await;
        insert_meeting(&pool).await;
        insert_transcripts(&pool).await;
        insert_summary(&pool, r###"{"markdown":"## Decisions\n\nShip on Friday."}"###).await;

        let data = load_export_data(&pool, MEETING_ID).await.unwrap();
        let options = ExportOptions::default();
        let markdown = build_meeting_markdown(&data, &options);

        let path = dir.path().join(suggest_filename(&data.title, &data.created_at));
        write_markdown_file(&path, &markdown).unwrap();

        assert_eq!(path.file_name().unwrap(), "team-standup-2026-08-01.md");

        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.starts_with("# Team Standup\n"));
        assert!(written.contains("- **Meeting ID:** `meeting-1`\n"));
        assert!(written.contains("## Summary\n\n## Decisions\n\nShip on Friday."));
        assert!(written.contains("**[00:00]** **Microphone:** First line."));
        assert!(written.contains("**[00:12]** **System Audio:** Second line."));
        // The pre-offset segment falls back to its wall-clock timestamp.
        assert!(written.contains("**09:31:00** Legacy line."));
    }

    #[tokio::test]
    async fn exports_the_transcript_when_no_summary_exists() {
        let dir = tempfile::tempdir().unwrap();
        let pool = migrated_pool(dir.path()).await;
        insert_meeting(&pool).await;
        insert_transcripts(&pool).await;

        let data = load_export_data(&pool, MEETING_ID).await.unwrap();
        assert!(data.summary.is_none());

        let markdown = build_meeting_markdown(&data, &ExportOptions::default());
        assert!(!markdown.contains("## Summary"));
        assert!(markdown.contains("## Transcript"));
        assert!(markdown.contains("First line."));
    }

    #[tokio::test]
    async fn a_corrupt_summary_does_not_block_the_export() {
        let dir = tempfile::tempdir().unwrap();
        let pool = migrated_pool(dir.path()).await;
        insert_meeting(&pool).await;
        insert_transcripts(&pool).await;
        insert_summary(&pool, "{not valid json").await;

        let data = load_export_data(&pool, MEETING_ID).await.unwrap();

        assert!(data.summary.is_none());
        let markdown = build_meeting_markdown(&data, &ExportOptions::default());
        assert!(markdown.contains("First line."));
    }

    #[tokio::test]
    async fn a_meeting_with_no_transcripts_still_exports_its_summary() {
        let dir = tempfile::tempdir().unwrap();
        let pool = migrated_pool(dir.path()).await;
        insert_meeting(&pool).await;
        insert_summary(&pool, r###"{"markdown":"## Decisions\n\nShip on Friday."}"###).await;

        let data = load_export_data(&pool, MEETING_ID).await.unwrap();
        assert!(data.transcripts.is_empty());

        let markdown = build_meeting_markdown(&data, &ExportOptions::default());
        assert!(markdown.contains("## Summary"));
        assert!(!markdown.contains("## Transcript"));
        // The segment count is omitted rather than reported as zero.
        assert!(!markdown.contains("**Segments:**"));
    }

    #[tokio::test]
    async fn reports_an_unknown_meeting() {
        let dir = tempfile::tempdir().unwrap();
        let pool = migrated_pool(dir.path()).await;

        let error = load_export_data(&pool, "does-not-exist").await.unwrap_err();
        assert!(error.contains("Meeting not found"), "unexpected error: {}", error);
    }
}
