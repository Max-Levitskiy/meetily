import { useCallback, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import Analytics from '@/lib/analytics';

/** Which parts of the meeting an export should contain. */
export type ExportScope = 'full' | 'summary' | 'transcript';

/**
 * Mirrors the Rust `ExportOptions`. Omitted fields fall back to their defaults
 * on the Rust side, so only the toggles that vary per scope are sent.
 */
const SCOPE_OPTIONS: Record<ExportScope, { includeSummary: boolean; includeTranscript: boolean }> = {
  full: { includeSummary: true, includeTranscript: true },
  summary: { includeSummary: true, includeTranscript: false },
  transcript: { includeSummary: false, includeTranscript: true },
};

interface UseExportOperationsProps {
  meeting: { id: string };
  /** Flushes pending editor edits so the export matches what is on screen. */
  ensureSaved?: () => Promise<void>;
}

function fileNameOf(path: string): string {
  return path.split(/[\\/]/).pop() || path;
}

export function useExportOperations({ meeting, ensureSaved }: UseExportOperationsProps) {
  const [isExporting, setIsExporting] = useState(false);

  const exportMarkdown = useCallback(async (scope: ExportScope) => {
    if (isExporting) return;

    setIsExporting(true);
    try {
      // The Rust command reads from the database, so unsaved edits have to be
      // persisted first or the exported file would be stale.
      await ensureSaved?.();

      const savedPath = await invoke<string | null>('save_meeting_markdown', {
        meetingId: meeting.id,
        options: SCOPE_OPTIONS[scope],
      });

      // A null path means the user dismissed the save dialog.
      if (!savedPath) return;

      toast.success(`Exported to ${fileNameOf(savedPath)}`);

      await Analytics.trackFeatureUsedEnhanced('export_markdown', {
        meeting_id: meeting.id,
        scope,
      });
    } catch (error) {
      console.error('Failed to export meeting as Markdown:', error);
      toast.error('Failed to export meeting', { description: String(error) });
    } finally {
      setIsExporting(false);
    }
  }, [isExporting, meeting.id, ensureSaved]);

  return {
    isExporting,
    exportMarkdown,
  };
}
