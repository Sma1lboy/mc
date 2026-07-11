/** Minimal runtime shape shared by local and cloud conversation payloads. */
export interface PersistedConversationRecord {
  id: string;
  messages: unknown[];
}

function parseJson(value: string): unknown | null {
  try {
    return JSON.parse(value) as unknown;
  } catch {
    return null;
  }
}

function isConversationRecord(value: unknown): value is PersistedConversationRecord {
  return (
    typeof value === "object" &&
    value !== null &&
    typeof (value as PersistedConversationRecord).id === "string" &&
    Array.isArray((value as PersistedConversationRecord).messages)
  );
}

/**
 * Native IPC returns a JSON array of JSON record strings because UIMessage
 * payloads cannot cross Specta recursively. Keep this transport boundary strict:
 * cloud records use a separate path and must not be accepted here.
 */
export function parseSerializedConversationRecords<T extends PersistedConversationRecord>(
  raw: string,
): T[] {
  const list = parseJson(raw);
  if (!Array.isArray(list)) return [];
  return list.flatMap((value) => {
    if (typeof value !== "string") return [];
    const record = parseJson(value);
    return isConversationRecord(record) ? [record as T] : [];
  });
}
