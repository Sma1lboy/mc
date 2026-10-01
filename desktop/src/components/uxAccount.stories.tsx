import { useEffect, useState } from "react";
import type { Story, StoryDefault } from "@ladle/react";
import { AccountDialog } from "./AccountDialog";
import { ToastContainer } from "./Toast";

export default { title: "UX Account" } satisfies StoryDefault;

/** Real AccountDialog; only the Tauri/clipboard boundary is simulated. No sign-in occurs. */
export const Acceptance: Story = () => {
  const [ready, setReady] = useState(false);
  const [open, setOpen] = useState(true);
  useEffect(() => {
    type FixtureWindow = Window & { __TAURI_INTERNALS__: { invoke: (cmd: string, args?: unknown) => Promise<unknown> }; uxScenario: string; uxCalls: Record<string, number> };
    const w = window as unknown as FixtureWindow;
    const original = w.__TAURI_INTERNALS__.invoke;
    const clipboard = Object.getOwnPropertyDescriptor(navigator, "clipboard");
    w.uxScenario = new URLSearchParams(location.search).get("uxCase") ?? "denied";
    w.uxCalls = {};
    w.__TAURI_INTERNALS__.invoke = async (cmd, args) => {
      w.uxCalls[cmd] = (w.uxCalls[cmd] ?? 0) + 1;
      if (cmd === "list_accounts") return [];
      if (cmd === "msa_login_start") {
        if (w.uxScenario === "start-error") throw "Fixture: login service unavailable";
        if (w.uxScenario === "delayed") await new Promise(r => setTimeout(r, 600));
        return { user_code: "TEST-CODE", device_code: "fixture-device", interval: 5, expires_in: 900, verification_uri: "https://example.invalid/fixture-sign-in" };
      }
      if (cmd === "msa_login_poll") {
        if (w.uxScenario === "poll-error") throw "Fixture: code expired";
        return new Promise(() => {});
      }
      if (cmd === "plugin:shell|open") {
        if (w.uxScenario === "denied") throw "Fixture: browser unavailable";
        return null;
      }
      return original(cmd, args);
    };
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: async () => {
      w.uxCalls.clipboard = (w.uxCalls.clipboard ?? 0) + 1;
      if (w.uxScenario === "denied") throw new Error("Fixture: clipboard denied");
    } } });
    setReady(true);
    return () => {
      w.__TAURI_INTERNALS__.invoke = original;
      if (clipboard) Object.defineProperty(navigator, "clipboard", clipboard);
      else Reflect.deleteProperty(navigator, "clipboard");
    };
  }, []);
  return <>
    <p style={{ position: "fixed", top: 8, left: 16, zIndex: 10000 }}>Fixture: real AccountDialog · simulated sign-in, clipboard and browser</p>
    {ready && open && <AccountDialog onClose={() => setOpen(false)} onDone={() => setOpen(false)} />}
    {ready && !open && <button onClick={() => setOpen(true)}>Reopen fixture</button>}
    <ToastContainer />
  </>;
};
