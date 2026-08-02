/** Which parts of the meeting an export should contain. */
export type ExportScope = 'full' | 'summary' | 'transcript';

/** The subset of Rust's `ExportOptions` that varies between scopes. */
export interface ScopeOptions {
  includeSummary: boolean;
  includeTranscript: boolean;
}

/**
 * Maps an export scope onto the toggles sent to the Rust command.
 *
 * Only the fields that vary are sent — the Rust side fills the rest from its
 * defaults, so metadata, timestamps and speaker labels stay on.
 */
export function optionsForScope(scope: ExportScope): ScopeOptions {
  return {
    includeSummary: scope !== 'transcript',
    includeTranscript: scope !== 'summary',
  };
}

/**
 * Extracts the file name from a saved path for display.
 *
 * Handles both POSIX and Windows separators because the path comes back from
 * the native save dialog.
 */
export function fileNameOf(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() || path;
}
