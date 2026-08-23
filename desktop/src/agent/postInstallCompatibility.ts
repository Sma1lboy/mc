import { commands } from "../ipc/bindings";

/** Host-owned closure: every successful install gets the same bounded static remediation pass. */
export function closePostInstallCompatibility(root: string, instanceId: string) {
  return commands.agentToolDiagnoseInstance(root, instanceId, {
    include_log_tail: false,
    mode: "remediate",
  });
}
