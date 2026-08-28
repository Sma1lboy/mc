import type { UIMessage } from "ai";
import type {
  AgentInstanceContext,
  AgentToolContext,
  AgentWikiContext,
} from "./agentContext";

export const MAX_CONVERSATION_RECORDS = 50;
export const MAX_CONVERSATION_RECORD_BYTES = 1_048_576;
export const MAX_CONVERSATION_MESSAGES = 500;

const MAX_SERIALIZED_HISTORY_BYTES =
  MAX_CONVERSATION_RECORDS * (MAX_CONVERSATION_RECORD_BYTES * 2 + 2) + 2;

/** Runtime shape shared by local and cloud conversation payloads. */
export interface PersistedConversationRecord {
  id: string;
  createdAt: number;
  updatedAt: number;
  title: string;
  messages: UIMessage[];
  toolContext?: AgentToolContext | null;
}

function parseJson(value: string): unknown | null {
  try {
    return JSON.parse(value) as unknown;
  } catch {
    return null;
  }
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isSafeTimestamp(value: unknown): value is number {
  return Number.isSafeInteger(value) && (value as number) >= 0;
}

function isWikiContext(value: unknown): value is AgentWikiContext {
  return (
    isObject(value) &&
    typeof value.root === "string" &&
    typeof value.modpackId === "string" &&
    typeof value.instanceId === "string" &&
    Array.isArray(value.sourcePaths) &&
    value.sourcePaths.every((path) => typeof path === "string")
  );
}

function isInstanceContext(value: unknown): value is AgentInstanceContext {
  if (!isWikiContext(value)) return false;
  const instance = value as unknown as Record<string, unknown>;
  return typeof instance.mcVersion === "string" && typeof instance.loader === "string";
}

function isToolContext(value: unknown): value is AgentToolContext | null | undefined {
  if (value === undefined || value === null) return true;
  if (!isObject(value)) return false;
  if (value.root !== undefined && typeof value.root !== "string") return false;
  if (
    value.mode !== undefined &&
    value.mode !== "build" &&
    value.mode !== "instance" &&
    value.mode !== "modpack" &&
    value.mode !== "wiki"
  ) {
    return false;
  }
  if (value.instance !== undefined && !isInstanceContext(value.instance)) return false;
  if (value.wiki !== undefined && !isWikiContext(value.wiki)) return false;
  return true;
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

async function validateConversationRecord<T extends PersistedConversationRecord>(
  value: unknown,
): Promise<T | null> {
  if (!isObject(value)) return null;
  if (typeof value.id !== "string" || value.id.trim() === "") return null;
  if (!isSafeTimestamp(value.createdAt) || !isSafeTimestamp(value.updatedAt)) return null;
  if (value.createdAt > value.updatedAt) return null;
  if (typeof value.title !== "string") return null;
  if (!Array.isArray(value.messages) || value.messages.length > MAX_CONVERSATION_MESSAGES) {
    return null;
  }
  if (!isToolContext(value.toolContext)) return null;

  const { safeValidateUIMessages } = await import("ai");
  const validated = await safeValidateUIMessages<UIMessage>({ messages: value.messages });
  if (!validated.success) return null;
  return { ...value, messages: validated.data } as T;
}

function stableRecordKey(record: PersistedConversationRecord): string {
  try {
    return JSON.stringify(record, (_key, value: unknown) => {
      if (!isObject(value)) return value;
      return Object.fromEntries(
        Object.keys(value)
          .sort()
          .map((key) => [key, value[key]]),
      );
    });
  } catch {
    return "";
  }
}

function preferredRecord<T extends PersistedConversationRecord>(left: T, right: T): T {
  if (left.updatedAt !== right.updatedAt) {
    return left.updatedAt > right.updatedAt ? left : right;
  }
  return stableRecordKey(left) >= stableRecordKey(right) ? left : right;
}

/** Deduplicate, deterministically order, and retain the newest bounded record set. */
export function normalizeConversationRecords<T extends PersistedConversationRecord>(
  records: readonly T[],
): T[] {
  const byId = new Map<string, T>();
  for (const record of records) {
    const existing = byId.get(record.id);
    byId.set(record.id, existing ? preferredRecord(existing, record) : record);
  }
  return [...byId.values()]
    .sort((left, right) => {
      if (left.updatedAt !== right.updatedAt) return right.updatedAt - left.updatedAt;
      if (left.id === right.id) return 0;
      return left.id < right.id ? 1 : -1;
    })
    .slice(0, MAX_CONVERSATION_RECORDS);
}

/** Validate and serialize one record before it crosses into native persistence. */
export async function serializeConversationRecord(
  record: PersistedConversationRecord,
): Promise<string | null> {
  const validated = await validateConversationRecord(record);
  if (!validated) return null;
  try {
    const serialized = JSON.stringify(validated);
    return exceedsUtf8ByteLimit(serialized, MAX_CONVERSATION_RECORD_BYTES) ? null : serialized;
  } catch {
    return null;
  }
}

/**
 * Native IPC returns a JSON array of JSON record strings because UIMessage
 * payloads cannot cross Specta recursively. Invalid records are isolated, but
 * an invalid or over-count transport envelope fails closed as a whole.
 */
export async function parseSerializedConversationRecords<
  T extends PersistedConversationRecord,
>(raw: string): Promise<T[]> {
  if (exceedsUtf8ByteLimit(raw, MAX_SERIALIZED_HISTORY_BYTES)) return [];
  const list = parseJson(raw);
  if (!Array.isArray(list) || list.length > MAX_CONVERSATION_RECORDS) return [];

  const records: T[] = [];
  for (const value of list) {
    if (
      typeof value !== "string" ||
      exceedsUtf8ByteLimit(value, MAX_CONVERSATION_RECORD_BYTES)
    ) {
      continue;
    }
    const record = await validateConversationRecord<T>(parseJson(value));
    if (record) records.push(record);
  }
  return normalizeConversationRecords(records);
}
