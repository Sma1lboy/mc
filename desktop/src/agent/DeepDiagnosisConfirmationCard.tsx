import { useState } from "react";
import type { UIMessage } from "ai";
import { Check, FlaskConical, LoaderCircle, X } from "lucide-react";
import { t } from "../i18n";
import type { StartDeepDiagnosisOutput } from "../ipc/bindings";
import { decideApprovedAction, executeApprovedDeepDiagnosis } from "./approvalActions";
import { captureClientToolRunId, resolveClientTool, useChatStore } from "./chatStore";

export const CONFIRM_DEEP_DIAGNOSIS_TOOL_TYPE = "tool-confirm_deep_diagnosis";

type ToolPart = Extract<UIMessage["parts"][number], { toolCallId: string }>;
type ApprovalOutput = StartDeepDiagnosisOutput | { approved: false };

function approvalOutput(value: unknown): ApprovalOutput | null {
  return value !== null && typeof value === "object" ? (value as ApprovalOutput) : null;
}

export function DeepDiagnosisConfirmationCard(props: {
  msgId: string;
  part: ToolPart;
  globalStreaming: boolean;
}): React.ReactElement {
  const { part, globalStreaming } = props;
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const context = useChatStore((state) => state.toolContext?.instance ?? null);
  const pendingLocalToolCallIds = useChatStore((state) => state.pendingLocalToolCallIds);
  const conversationId = useChatStore((state) => state.conversationId);
  const reason =
    typeof (part.input as { reason?: unknown } | undefined)?.reason === "string"
      ? (part.input as { reason: string }).reason.trim()
      : "";
  const done = part.state === "output-available";
  const output = approvalOutput(done ? part.output : null);
  const declined = output !== null && "approved" in output && output.approved === false;
  const live =
    part.state === "input-available" &&
    (!globalStreaming || pendingLocalToolCallIds.includes(part.toolCallId)) &&
    Boolean(context?.root) &&
    reason.length > 0 &&
    !busy;

  const decide = async (approved: boolean): Promise<void> => {
    if (!live || !context?.root) return;
    const boundContext = { ...context, root: context.root };
    const expectedRunId = captureClientToolRunId(
      conversationId,
      props.msgId,
      part.toolCallId,
    );
    if (approved) setBusy(true);
    setError(null);
    try {
      const result = await decideApprovedAction(approved, () =>
        executeApprovedDeepDiagnosis(boundContext),
      );
      resolveClientTool(
        conversationId,
        props.msgId,
        part.toolCallId,
        result,
        undefined,
        expectedRunId,
      );
    } catch (cause) {
      setBusy(false);
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  };

  if (part.state === "input-streaming" && !reason) {
    return <div className="my-[6px] h-[130px] border border-titlebar bg-panel-2 shadow-input animate-pulse" aria-hidden="true" />;
  }

  return (
    <div className="my-[6px] border border-titlebar bg-panel-2 shadow-input">
      <div className="flex items-start gap-[9px] px-[13px] py-[10px] border-b border-titlebar">
        <span className="mt-[1px] grid h-[24px] w-[24px] shrink-0 place-items-center bg-panel-3 text-accent">
          <FlaskConical size={14} aria-hidden="true" />
        </span>
        <div className="min-w-0">
          <div className="text-[13px] font-medium text-strong">{t("agent.deepConfirmTitle")}</div>
          {reason && <div className="mt-[2px] text-[12px] leading-[1.5] text-sub">{reason}</div>}
        </div>
      </div>
      <ul className="list-disc space-y-[3px] px-[29px] py-[9px] text-[12px] leading-[1.5] text-sub">
        <li>{t("agent.deepConfirmVisibleLaunch")}</li>
        <li>{t("agent.deepConfirmInstalledMods")}</li>
        <li>{t("agent.deepConfirmOffline")}</li>
        <li>{t("agent.deepConfirmLimit")}</li>
        <li>{t("agent.deepConfirmNotSandbox")}</li>
      </ul>
      <div className="flex items-center gap-[8px] px-[13px] py-[9px] border-t border-titlebar">
        {done ? (
          <div className="text-[12px] text-sub">
            {declined ? t("agent.deepDeclined") : t("agent.deepStarted")}
          </div>
        ) : (
          <>
            <button type="button" disabled={!live} onClick={() => void decide(true)} className="inline-flex h-[30px] items-center gap-[6px] px-[12px] bg-accent text-accent-text shadow-raised text-[12.5px] font-medium cursor-pointer disabled:opacity-50 disabled:cursor-not-allowed">
              {busy ? <LoaderCircle size={14} className="animate-spin" aria-hidden="true" /> : <Check size={14} aria-hidden="true" />}
              {busy ? t("agent.deepStarting") : t("agent.deepApprove")}
            </button>
            <button type="button" disabled={!live} onClick={() => void decide(false)} className="inline-flex h-[30px] items-center gap-[6px] px-[11px] bg-panel-3 text-sub shadow-input text-[12.5px] cursor-pointer disabled:opacity-50 disabled:cursor-not-allowed">
              <X size={14} aria-hidden="true" />
              {t("agent.deepCancel")}
            </button>
          </>
        )}
      </div>
      {!context && <div className="px-[13px] py-[8px] border-t border-titlebar text-[12px] text-danger-text">{t("agent.deepMissingContext")}</div>}
      {error && <div className="px-[13px] py-[8px] border-t border-titlebar text-[12px] text-danger-text">{t("agent.deepFailed", { err: error })}</div>}
    </div>
  );
}

export default DeepDiagnosisConfirmationCard;
