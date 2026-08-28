export const MAX_AGENT_USER_INPUT_BYTES = 64 * 1024;

export const AGENT_USER_INPUT_INVALID_ERROR = "agent.inputPlainText";
export const AGENT_USER_INPUT_EMPTY_ERROR = "agent.inputNonEmpty";
export const AGENT_USER_INPUT_TOO_LARGE_ERROR = "agent.inputTooLarge";

export type AgentUserInputAdmission =
  | { status: "accepted"; text: string }
  | { status: "empty" }
  | { status: "rejected"; error: string };

/** Validate the runtime shape and UTF-8 budget before trimming or copying user input. */
export function admitAgentUserInput(raw: unknown): AgentUserInputAdmission {
  if (typeof raw !== "string") {
    return { status: "rejected", error: AGENT_USER_INPUT_INVALID_ERROR };
  }
  if (exceedsUtf8ByteLimit(raw, MAX_AGENT_USER_INPUT_BYTES)) {
    return { status: "rejected", error: AGENT_USER_INPUT_TOO_LARGE_ERROR };
  }
  const text = raw.trim();
  return text ? { status: "accepted", text } : { status: "empty" };
}

function exceedsUtf8ByteLimit(value: string, limit: number): boolean {
  let bytes = 0;
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    if (code <= 0x7f) bytes += 1;
    else if (code <= 0x7ff) bytes += 2;
    else if (code >= 0xd800 && code <= 0xdbff) {
      const next = value.charCodeAt(index + 1);
      if (next >= 0xdc00 && next <= 0xdfff) {
        bytes += 4;
        index += 1;
      } else {
        bytes += 3;
      }
    } else {
      bytes += 3;
    }
    if (bytes > limit) return true;
  }
  return false;
}
