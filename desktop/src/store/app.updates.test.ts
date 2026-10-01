import { beforeEach, describe, expect, it, vi } from "vitest";
vi.mock("../ipc/api", () => ({ api: { checkAllUpdates: vi.fn(), listInstances: vi.fn().mockResolvedValue([]) } }));
vi.mock("../components/Toast", () => ({ toast: vi.fn() }));
import { api } from "../ipc/api";
import { toast } from "../components/Toast";
import { checkAllUpdates, setCurrentRoot } from "./app";
import { useAppStore } from "./state";

describe("update check status", () => {
  beforeEach(() => { vi.clearAllMocks(); useAppStore.setState({ currentRoot: "/fixture/a", checkingUpdates: false, updatesByInstance: {} }); });
  it("reports an empty result as no updates found, never an all-current guarantee", async () => {
    vi.mocked(api.checkAllUpdates).mockResolvedValue([]);
    await checkAllUpdates();
    expect(toast).toHaveBeenCalledWith(expect.objectContaining({ type: "info", message: "未发现可用更新。部分内容可能未被检查。" }));
    expect(useAppStore.getState().checkingUpdates).toBe(false);
  });
  it("does not apply a result after switching game directories", async () => {
    let resolve!: (value: Awaited<ReturnType<typeof api.checkAllUpdates>>) => void;
    vi.mocked(api.checkAllUpdates).mockImplementation(() => new Promise(r => { resolve = r; }));
    const pending = checkAllUpdates();
    setCurrentRoot("/fixture/b");
    resolve([{ instance_id: "old-directory", mod_updates: 1, modpack_update: false }]);
    await pending;
    expect(useAppStore.getState().updatesByInstance).toEqual({});
    expect(toast).not.toHaveBeenCalled();
    expect(useAppStore.getState().checkingUpdates).toBe(false);
  });
  it("clears old badges on directory changes", () => {
    useAppStore.setState({ updatesByInstance: { old: { mods: 2, modpack: false } } });
    setCurrentRoot("/fixture/b");
    expect(useAppStore.getState().updatesByInstance).toEqual({});
  });
  it("prevents repeat checks and allows retry after failure", async () => {
    let reject!: (error: unknown) => void;
    vi.mocked(api.checkAllUpdates).mockImplementation(() => new Promise((_, r) => { reject = r; }));
    const first = checkAllUpdates(); await checkAllUpdates();
    expect(api.checkAllUpdates).toHaveBeenCalledTimes(1);
    reject("Fixture failure"); await first;
    expect(useAppStore.getState().checkingUpdates).toBe(false);
    expect(toast).toHaveBeenLastCalledWith(expect.objectContaining({ type: "error" }));
    vi.mocked(api.checkAllUpdates).mockResolvedValue([]); await checkAllUpdates();
    expect(api.checkAllUpdates).toHaveBeenCalledTimes(2);
  });
});
