import { beforeEach, describe, expect, it } from "vitest";
import { clearFinished, dismissDownload, useDownloadStore, type DownloadTask } from "./downloads";
import dictionary from "../locales/downloads";
const row = (id: string, status: DownloadTask["status"]): DownloadTask => ({ id, status, title: id, kind: "modpack", stage: "", current: 0, total: 0, speedBps: 0 });
describe("download history actions", () => {
  beforeEach(() => useDownloadStore.setState({ tasks: [row("queued", "queued"), row("active", "active"), row("done", "done"), row("error", "error")] }));
  it("clears ended records, including errors, without clearing active work", () => {
    clearFinished(); expect(useDownloadStore.getState().tasks.map(t => t.id)).toEqual(["queued", "active"]);
  });
  it("dismisses only ended records even when called for an active task", () => {
    dismissDownload("active"); dismissDownload("queued"); dismissDownload("missing");
    expect(useDownloadStore.getState().tasks).toHaveLength(4);
    dismissDownload("done"); dismissDownload("error");
    expect(useDownloadStore.getState().tasks.map(t => t.id)).toEqual(["queued", "active"]);
  });
  it("provides translated failure recovery and distinguishes records from installed files", () => {
    for (const lang of ["zh", "en"] as const) {
      for (const key of ["retryHint", "errorDetails", "recordsOnly"]) expect(dictionary[lang][key]).toBeTruthy();
      expect(dictionary[lang].dismiss).toContain("{{ title }}");
    }
  });
});
