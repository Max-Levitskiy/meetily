import { describe, expect, test } from "bun:test";
import { fileNameOf, optionsForScope } from "../../src/lib/meeting-export";

describe("optionsForScope", () => {
  test("full export includes both sections", () => {
    expect(optionsForScope("full")).toEqual({
      includeSummary: true,
      includeTranscript: true,
    });
  });

  test("summary scope drops the transcript", () => {
    expect(optionsForScope("summary")).toEqual({
      includeSummary: true,
      includeTranscript: false,
    });
  });

  test("transcript scope drops the summary", () => {
    expect(optionsForScope("transcript")).toEqual({
      includeSummary: false,
      includeTranscript: true,
    });
  });

  test("never sends the other option fields, so Rust defaults apply", () => {
    for (const scope of ["full", "summary", "transcript"] as const) {
      expect(Object.keys(optionsForScope(scope)).sort()).toEqual([
        "includeSummary",
        "includeTranscript",
      ]);
    }
  });
});

describe("fileNameOf", () => {
  test("reads the name from a POSIX path", () => {
    expect(fileNameOf("/Users/max/Documents/team-standup-2026-08-01.md")).toBe(
      "team-standup-2026-08-01.md",
    );
  });

  test("reads the name from a Windows path", () => {
    expect(fileNameOf("C:\\Users\\max\\Documents\\team-standup-2026-08-01.md")).toBe(
      "team-standup-2026-08-01.md",
    );
  });

  test("passes through a bare file name", () => {
    expect(fileNameOf("notes.md")).toBe("notes.md");
  });

  test("falls back to the input when there is no name to extract", () => {
    expect(fileNameOf("/")).toBe("/");
    expect(fileNameOf("")).toBe("");
  });
});
