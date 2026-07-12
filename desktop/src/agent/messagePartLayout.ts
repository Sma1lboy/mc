import type { UIMessage } from "ai";

type Part = UIMessage["parts"][number];

const INTERACTIVE_TOOL_TYPES = new Set([
  "tool-ask_user_question",
  "tool-confirm_modpack_build",
  "tool-confirm_deep_diagnosis",
  "tool-show_modpack",
  "tool-show_instance_changes",
]);

export type MessagePartLayoutEntry =
  | { kind: "activity"; indices: number[] }
  | { kind: "part"; index: number };

function isToolPart(part: Part): boolean {
  return typeof (part as { toolCallId?: unknown }).toolCallId === "string";
}

export function isInteractiveToolPart(part: Part): boolean {
  return isToolPart(part) && INTERACTIVE_TOOL_TYPES.has(part.type);
}

export function isActivityPart(part: Part): boolean {
  return part.type === "reasoning" || (isToolPart(part) && !isInteractiveToolPart(part));
}

/**
 * Preserve the existing "intermediate text + activity" grouping while making
 * every interactive card its own render boundary, even if later activity exists.
 */
export function layoutMessageParts(parts: Part[]): MessagePartLayoutEntry[] {
  const layout: MessagePartLayoutEntry[] = [];
  let cursor = 0;
  while (cursor < parts.length) {
    const nextInteractive = parts.findIndex(
      (part, index) => index >= cursor && isInteractiveToolPart(part),
    );
    const rangeEnd = nextInteractive < 0 ? parts.length : nextInteractive;
    appendNonInteractiveRange(layout, parts, cursor, rangeEnd);
    if (nextInteractive < 0) break;
    layout.push({ kind: "part", index: nextInteractive });
    cursor = nextInteractive + 1;
  }
  return layout;
}

function appendNonInteractiveRange(
  layout: MessagePartLayoutEntry[],
  parts: Part[],
  start: number,
  end: number,
): void {
  let lastActivity = -1;
  for (let index = start; index < end; index += 1) {
    if (isActivityPart(parts[index])) lastActivity = index;
  }
  if (lastActivity >= start) {
    layout.push({
      kind: "activity",
      indices: Array.from({ length: lastActivity - start + 1 }, (_, offset) => start + offset),
    });
  }
  for (let index = Math.max(start, lastActivity + 1); index < end; index += 1) {
    layout.push({ kind: "part", index });
  }
}
