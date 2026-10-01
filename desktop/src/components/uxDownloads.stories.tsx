import { useEffect, useState } from "react";
import type { Story, StoryDefault } from "@ladle/react";
import { DownloadQueue } from "./DownloadQueue";
import { useDownloadStore, type DownloadTask } from "../util/downloads";
export default { title: "UX Downloads" } satisfies StoryDefault;
export const Acceptance: Story = () => {
  const [ready, setReady] = useState(false);
  useEffect(() => {
    const row = (id: string, status: DownloadTask["status"]): DownloadTask => ({ id, title: `Fixture: ${id}`, kind: "modpack", status, stage: "", current: 3, total: 10, speedBps: 0, ...(status === "error" ? { error: "Fixture: HTTP 503 while downloading modpack" } : {}) });
    useDownloadStore.setState({ tasks: [row("Installing modpack", "active"), row("Queued mod", "queued"), row("Installed modpack", "done"), row("Failed modpack", "error")] });
    setReady(true);
    return () => useDownloadStore.setState({ tasks: [] });
  }, []);
  return <>
    <p style={{ position: "fixed", top: 8, left: 16, zIndex: 10000 }}>Fixture: real DownloadQueue · simulated active, queued, done and failed tasks</p>
    <div style={{ position: "fixed", top: 80, right: 64 }}>{ready && <DownloadQueue />}</div>
  </>;
};
