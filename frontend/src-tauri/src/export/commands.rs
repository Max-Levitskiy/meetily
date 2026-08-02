//! Tauri commands for exporting a meeting to a file.

use log::{error, info, warn};
use serde::Serialize;
use sqlx::SqlitePool;
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

    std::fs::write(&path, markdown).map_err(|e| {
        error!("Failed to write Markdown export to {:?}: {}", path, e);
        format!("Failed to write file: {}", e)
    })?;

    let saved = path.to_string_lossy().to_string();
    info!("Exported meeting {} to {}", meeting_id, saved);
    Ok(Some(saved))
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
