import { useCallback, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import Analytics from '@/lib/analytics';
import { fileNameOf, optionsForScope, type ExportScope } from '@/lib/meeting-export';

export type { ExportScope };

interface UseExportOperationsProps {
  meeting: { id: string };
  /** Flushes pending editor edits so the export matches what is on screen. */
  ensureSaved?: () => Promise<void>;
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
        options: optionsForScope(scope),
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
