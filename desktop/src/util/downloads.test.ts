import { beforeEach, describe, expect, it } from "vitest";
import { selectDownloadForRef, useDownloadStore, type DownloadTask } from "./downloads";

function task(id: string, refId: string, status: DownloadTask["status"], current = 0): DownloadTask {
  return {
    id,
    refId,
    title: id,
    icon: null,
    kind: "modpack",
    status,
    stage: "",
    current,
    total: 10,
    speedBps: 0,
  };
}

describe("download queue selectors", () => {
  beforeEach(() => {
    useDownloadStore.setState({ tasks: [] });
  });

  it("selects the active or queued task for a ref before older finished tasks", () => {
    const finished = task("done", "pack-a", "done", 10);
    const active = task("active", "pack-a", "active", 1);

    expect(selectDownloadForRef([finished, active], "pack-a")).toBe(active);
  });

  it("does not notify a ref subscriber when only another task progress changes", () => {
    const alpha = task("alpha", "pack-a", "active", 1);
    const beta = task("beta", "pack-b", "queued", 0);
    useDownloadStore.setState({ tasks: [alpha, beta] });

    const seen: Array<DownloadTask | undefined> = [];
    const unsubscribe = useDownloadStore.subscribe(
      (state) => selectDownloadForRef(state.tasks, "pack-b"),
      (next) => seen.push(next),
    );

    useDownloadStore.setState({ tasks: [{ ...alpha, current: 2 }, beta] });
    expect(seen).toEqual([]);

    const betaActive = { ...beta, status: "active" as const };
    useDownloadStore.setState({ tasks: [{ ...alpha, current: 3 }, betaActive] });
    expect(seen).toEqual([betaActive]);

    unsubscribe();
  });
});
