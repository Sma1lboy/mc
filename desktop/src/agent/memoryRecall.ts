import type { MemoryCandidate, MemoryRecallRequest } from "@kobemc/agent-core";
import type { UIMessage } from "ai";
import type { AgentToolContext, AgentWikiContext } from "./agentContext";
import type { PersistedConversationRecord } from "./conversationHistory";

export const DESKTOP_MEMORY_LIMITS = Object.freeze({
  maxCandidates: 8,
  maxContentChars: 1_024,
  maxScopeIdChars: 256,
  maxSourceIdChars: 256,
  maxReferenceChars: 1_024,
});

const PROVENANCE_SOURCE = "launcher.conversation.automatic_tool_output";
const DIAGNOSTIC_TOOLS = new Set([
  "diagnose_instance",
  "run_diagnostic_trial",
  "finish_deep_diagnosis",
]);
const INSTANCE_FACT_TOOLS = new Set(["wiki_search", "wiki_open", "list_instances"]);

interface BuildMemoryRecallInput {
  conversationId: string;
  toolContext: AgentToolContext | null;
  records: readonly PersistedConversationRecord[];
  isAutomaticTool: (name: string) => boolean;
}

interface PreparedCandidate {
  candidate: MemoryCandidate;
  fingerprint: string;
  priority: number;
  updatedAt: number;
  messageIndex: number;
  partIndex: number;
}

/** Build a bounded, side-effect-free recall request from launcher-owned history. */
export function buildMemoryRecallRequest({
  conversationId,
  toolContext,
  records,
  isAutomaticTool,
}: BuildMemoryRecallInput): MemoryRecallRequest | undefined {
  const scopeId = instanceScopeId(toolContext);
  if (!scopeId || !boundedId(conversationId)) return undefined;

  const prepared = records
    .filter((record) => record.id !== conversationId && instanceScopeId(record.toolContext) === scopeId)
    .slice()
    .sort(compareRecords)
    .flatMap((record, recordIndex) =>
      candidatesFromRecord(record, recordIndex, scopeId, isAutomaticTool),
    )
    .sort(compareRecency);
  const seen = new Set<string>();
  const unique = prepared.filter((item) => {
    if (seen.has(item.fingerprint)) return false;
    seen.add(item.fingerprint);
    return true;
  });
  const candidates = unique
    .sort(comparePriority)
    .slice(0, DESKTOP_MEMORY_LIMITS.maxCandidates)
    .map((item) => item.candidate);
  if (candidates.length === 0) return undefined;
  return { identity: { scopeId, conversationId }, candidates };
}

function candidatesFromRecord(
  record: PersistedConversationRecord,
  recordIndex: number,
  scopeId: string,
  isAutomaticTool: (name: string) => boolean,
): PreparedCandidate[] {
  if (!boundedId(record.id)) return [];
  const updatedAt = isoTimestamp(record.updatedAt);
  if (!updatedAt) return [];
  const prepared: PreparedCandidate[] = [];
  record.messages.forEach((message, messageIndex) => {
    if (message.role !== "assistant" || !boundedId(message.id)) return;
    message.parts.forEach((part, partIndex) => {
      const tool = completedToolOutput(part);
      if (!tool || !isAutomaticTool(tool.name)) return;
      const serialized = stableJson(tool.output);
      if (serialized === undefined) return;
      const reference = provenanceReference(record.id, message.id, tool.toolCallId, tool.name);
      if (!reference) return;
      const content = boundedContent(`${tool.name} output: `, serialized);
      const sourceKey = `${recordIndex}:${messageIndex}:${partIndex}`;
      prepared.push({
        candidate: {
          id: `tool-output:${sourceKey}`,
          scopeId,
          conversationId: record.id,
          visibility: "scope",
          tier: "recall",
          memoryKey: `automatic-tool:${tool.name}:${sourceKey}`,
          content,
          updatedAt,
          provenance: { source: PROVENANCE_SOURCE, reference },
        },
        fingerprint: `${tool.name}\u0000${serialized}`,
        priority: toolPriority(tool.name),
        updatedAt: record.updatedAt,
        messageIndex,
        partIndex,
      });
    });
  });
  return prepared;
}

function instanceScopeId(context: AgentToolContext | null | undefined): string | undefined {
  if (!context) return undefined;
  const sources = [context.instance, context.wiki].filter(
    (value): value is AgentWikiContext => value !== undefined,
  );
  if (sources.length === 0) return undefined;
  const scopes = sources.map((value) => scopeParts(value, context.root));
  if (scopes.some((value) => value === undefined)) return undefined;
  const [first, ...rest] = scopes as [readonly [string, string], ...Array<readonly [string, string]>];
  if (rest.some(([root, instanceId]) => root !== first[0] || instanceId !== first[1])) {
    return undefined;
  }
  const scopeId = `instance:${JSON.stringify(first)}`;
  return scopeId.length <= DESKTOP_MEMORY_LIMITS.maxScopeIdChars ? scopeId : undefined;
}

function scopeParts(
  source: AgentWikiContext,
  hostRoot: string | undefined,
): readonly [string, string] | undefined {
  const root = source.root.trim();
  const instanceId = source.instanceId.trim();
  if (
    !root ||
    !instanceId ||
    root !== source.root ||
    instanceId !== source.instanceId ||
    (hostRoot !== undefined && hostRoot !== source.root)
  ) {
    return undefined;
  }
  return [root, instanceId];
}

function completedToolOutput(
  part: UIMessage["parts"][number],
): { name: string; toolCallId: string; output: unknown } | undefined {
  const candidate = part as {
    type?: unknown;
    toolCallId?: unknown;
    state?: unknown;
    output?: unknown;
    preliminary?: unknown;
  };
  if (
    typeof candidate.type !== "string" ||
    !candidate.type.startsWith("tool-") ||
    typeof candidate.toolCallId !== "string" ||
    !boundedId(candidate.toolCallId) ||
    candidate.state !== "output-available" ||
    candidate.preliminary === true
  ) {
    return undefined;
  }
  const name = candidate.type.slice("tool-".length);
  return name ? { name, toolCallId: candidate.toolCallId, output: candidate.output } : undefined;
}

function stableJson(value: unknown): string | undefined {
  try {
    const normalized = normalizeJson(value, new Set());
    if (normalized === OMIT) return undefined;
    return JSON.stringify(normalized);
  } catch {
    return undefined;
  }
}

const OMIT = Symbol("omit");

function normalizeJson(value: unknown, ancestors: Set<object>): unknown | typeof OMIT {
  if (value === null || typeof value === "string" || typeof value === "boolean") return value;
  if (typeof value === "number") return Number.isFinite(value) ? value : OMIT;
  if (typeof value !== "object") return OMIT;
  if (ancestors.has(value)) throw new Error("cyclic tool output");
  ancestors.add(value);
  let normalized: unknown;
  if (Array.isArray(value)) {
    normalized = value.map((item) => {
      const child = normalizeJson(item, ancestors);
      return child === OMIT ? null : child;
    });
  } else {
    const record = value as Record<string, unknown>;
    normalized = Object.fromEntries(
      Object.keys(record)
        .sort()
        .flatMap((key) => {
          const child = normalizeJson(record[key], ancestors);
          return child === OMIT ? [] : [[key, child]];
        }),
    );
  }
  ancestors.delete(value);
  return normalized;
}

function boundedContent(prefix: string, serialized: string): string {
  const limit = DESKTOP_MEMORY_LIMITS.maxContentChars;
  if (prefix.length + serialized.length <= limit) return prefix + serialized;
  const suffix = "...[truncated]";
  const end = Math.max(0, limit - prefix.length - suffix.length);
  return prefix + safeSlice(serialized, end) + suffix;
}

function safeSlice(value: string, end: number): string {
  if (end <= 0) return "";
  const sliced = value.slice(0, end);
  const last = sliced.charCodeAt(sliced.length - 1);
  return last >= 0xd800 && last <= 0xdbff ? sliced.slice(0, -1) : sliced;
}

function provenanceReference(
  conversationId: string,
  messageId: string,
  toolCallId: string,
  toolName: string,
): string | undefined {
  const reference = [
    `conversation=${JSON.stringify(conversationId)}`,
    `message=${JSON.stringify(messageId)}`,
    `toolCall=${JSON.stringify(toolCallId)}`,
    `tool=${toolName}`,
  ].join(";");
  return reference.length <= DESKTOP_MEMORY_LIMITS.maxReferenceChars ? reference : undefined;
}

function boundedId(value: string): boolean {
  return value.trim().length > 0 && value.length <= DESKTOP_MEMORY_LIMITS.maxSourceIdChars;
}

function isoTimestamp(value: number): string | undefined {
  try {
    return new Date(value).toISOString();
  } catch {
    return undefined;
  }
}

function toolPriority(name: string): number {
  if (DIAGNOSTIC_TOOLS.has(name)) return 0;
  if (INSTANCE_FACT_TOOLS.has(name)) return 1;
  return 2;
}

function compareRecency(left: PreparedCandidate, right: PreparedCandidate): number {
  return (
    right.updatedAt - left.updatedAt ||
    right.messageIndex - left.messageIndex ||
    right.partIndex - left.partIndex ||
    left.candidate.provenance.reference.localeCompare(right.candidate.provenance.reference)
  );
}

function comparePriority(left: PreparedCandidate, right: PreparedCandidate): number {
  return left.priority - right.priority || compareRecency(left, right);
}

function compareRecords(
  left: PersistedConversationRecord,
  right: PersistedConversationRecord,
): number {
  return right.updatedAt - left.updatedAt || left.id.localeCompare(right.id);
}
