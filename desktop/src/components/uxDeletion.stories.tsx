import { useEffect, useState } from "react";
import type { Story, StoryDefault } from "@ladle/react";
import { InstanceRow } from "./InstanceRow";
import { WorldsPanel } from "./instance-manage/WorldsPanel";
import type { InstanceSummary } from "../ipc/types";
export default { title: "UX Deletion" } satisfies StoryDefault;
const instance: InstanceSummary = { id: "fixture", name: "Fixture world collection", mc_version: "1.20.1", loader: "fabric", installed: true };
export const Acceptance: Story = () => {
  const [ready, setReady] = useState(false);
  const view = new URLSearchParams(location.search).get("uxView") ?? "instance";
  useEffect(() => {
    const w = window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string, args?: unknown) => Promise<unknown> }; uxDeleteCalls: number };
    const original = w.__TAURI_INTERNALS__.invoke;
    w.uxDeleteCalls = 0;
    w.__TAURI_INTERNALS__.invoke = async (cmd, args) => {
      if (cmd.startsWith("delete_")) { w.uxDeleteCalls++; throw "Fixture: deletion is blocked"; }
      if (cmd === "instance_worlds") return [{ folder: "fixture-world", name: "Fixture survival world", game_mode: "survival", last_played: 0, seed: 12345, size_bytes: 1024 }];
      return original(cmd, args);
    };
    setReady(true);
    return () => { w.__TAURI_INTERNALS__.invoke = original; };
  }, []);
  return <>
    <p style={{ position: "fixed", top: 8, left: 16, zIndex: 10000 }}>Fixture: real deletion dialog · synthetic data · deletion blocked</p>
    <div style={{ marginTop: 64 }}>{ready && (view === "world" ? <WorldsPanel instance={instance} /> : <InstanceRow instance={{ ...instance, loader_version: undefined, icon: undefined, last_played: 0, running: false }} onDelete={() => { throw new Error("Fixture: deletion is blocked"); }} />)}</div>
  </>;
};
