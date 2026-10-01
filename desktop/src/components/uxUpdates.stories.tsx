import { useEffect, useState } from "react";
import type { Story, StoryDefault } from "@ladle/react";
import Library from "../pages/Library";
import { useAppStore } from "../store/state";
import { ModsTab } from "./instance-manage/ModsTab";
import { useModsTab } from "./instance-manage/useModsTab";
import { ToastContainer } from "./Toast";
import type { InstanceSummary } from "../ipc/types";
export default { title: "UX Updates" } satisfies StoryDefault;
const instance: InstanceSummary = { id: "fixture", name: "Fixture: mixed mod sources", mc_version: "1.20.1", loader: "fabric", installed: true };
function Mods() {
  const m = useModsTab(instance, true);
  return <div style={{ marginTop: 64, display: "flex", flexDirection: "column", gap: 12 }}><ModsTab instance={instance} m={m} browsing={false} onExitBrowse={() => {}} startBrowse={() => {}} onLoaderAdded={() => {}} /></div>;
}
export const Acceptance: Story = () => {
  const [ready, setReady] = useState(false);
  const view = new URLSearchParams(location.search).get("uxView") ?? "library";
  useEffect(() => {
    const w = window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string, args?: unknown) => Promise<unknown> } };
    const original = w.__TAURI_INTERNALS__.invoke;
    w.__TAURI_INTERNALS__.invoke = async (cmd, args) => {
      if (["check_all_updates", "check_mod_updates", "list_versions"].includes(cmd)) return [];
      if (cmd === "list_instances") return [instance];
      if (cmd === "instance_mods") return [{ file_name: "unrecognized.jar", name: "Fixture: manually installed mod", enabled: true, version: "1.0", loader: "fabric" }, { file_name: "disabled.jar", name: "Fixture: disabled mod", enabled: false, version: "1.0", loader: "fabric" }];
      return original(cmd, args);
    };
    useAppStore.setState({ instances: [instance], currentRoot: "/fixture", socialEnabled: false, checkingUpdates: false, updatesByInstance: {} });
    setReady(true);
    return () => { w.__TAURI_INTERNALS__.invoke = original; };
  }, []);
  return <>
    <p style={{ position: "fixed", top: 8, left: 16, zIndex: 10000 }}>Fixture: real {view === "mods" ? "ModsTab" : "Library"} · simulated empty update result</p>
    {ready && (view === "mods" ? <Mods /> : <div style={{ marginTop: 64 }}><Library /></div>)}
    <ToastContainer />
  </>;
};
