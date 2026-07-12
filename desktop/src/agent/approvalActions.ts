import {
  commands,
  type BuildModpackArgs,
  type BuildModpackOutput,
  type StartDeepDiagnosisOutput,
} from "../ipc/bindings";

/** Execute a privileged action only after an explicit launcher-card decision. */
export async function decideApprovedAction<T>(
  approved: boolean,
  execute: () => Promise<T>,
): Promise<T | { approved: false }> {
  return approved ? execute() : { approved: false };
}

export async function executeApprovedModpackBuild(
  input: BuildModpackArgs,
): Promise<BuildModpackOutput> {
  const response = await commands.agentToolBuildModpack(input);
  if (response.status === "error") throw new Error(response.error);
  return response.data;
}

export async function executeApprovedDeepDiagnosis(context: {
  root: string;
  instanceId: string;
}): Promise<StartDeepDiagnosisOutput> {
  const response = await commands.agentToolStartDeepDiagnosis(context.root, context.instanceId);
  if (response.status === "error") throw new Error(response.error);
  return response.data;
}
