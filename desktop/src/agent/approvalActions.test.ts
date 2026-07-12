import { describe, expect, it, vi } from "vitest";

const ipc = vi.hoisted(() => ({
  build: vi.fn(),
  diagnose: vi.fn(),
}));

vi.mock("../ipc/bindings", () => ({
  commands: {
    agentToolBuildModpack: ipc.build,
    agentToolStartDeepDiagnosis: ipc.diagnose,
  },
}));

import {
  decideApprovedAction,
  executeApprovedDeepDiagnosis,
  executeApprovedModpackBuild,
} from "./approvalActions";

describe("decideApprovedAction", () => {
  it("returns a declined result without executing the privileged action", async () => {
    const execute = vi.fn(async () => ({ output_path: "/tmp/pack.mrpack" }));

    await expect(decideApprovedAction(false, execute)).resolves.toEqual({ approved: false });
    expect(execute).not.toHaveBeenCalled();
  });

  it("executes an approved privileged action exactly once and returns its output", async () => {
    const output = { output_path: "/tmp/pack.mrpack" };
    const execute = vi.fn(async () => output);

    await expect(decideApprovedAction(true, execute)).resolves.toBe(output);
    expect(execute).toHaveBeenCalledTimes(1);
  });

  it("returns the native modpack build output", async () => {
    const output = { status: "completed", output_path: "/tmp/pack.mrpack" };
    ipc.build.mockResolvedValueOnce({ status: "ok", data: output });

    await expect(
      executeApprovedModpackBuild({
        target: { mc_version: "1.20.1", loader: "fabric" },
        base_pack: null,
        extra_mods: [],
        output_filename: "pack.mrpack",
      }),
    ).resolves.toBe(output);
    expect(ipc.build).toHaveBeenCalledTimes(1);
  });

  it("binds deep diagnosis to the launcher-owned instance context", async () => {
    const output = { session_id: "diag-1" };
    ipc.diagnose.mockResolvedValueOnce({ status: "ok", data: output });

    await expect(
      executeApprovedDeepDiagnosis({ root: "/game", instanceId: "pack" }),
    ).resolves.toBe(output);
    expect(ipc.diagnose).toHaveBeenCalledWith("/game", "pack");
  });

  it("surfaces native command errors without returning an approved result", async () => {
    ipc.build.mockResolvedValueOnce({ status: "error", error: "blocked" });

    await expect(
      executeApprovedModpackBuild({
        target: { mc_version: "1.20.1", loader: "fabric" },
        base_pack: null,
        extra_mods: [],
        output_filename: "pack.mrpack",
      }),
    ).rejects.toThrow("blocked");
  });
});
