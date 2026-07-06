import type { UIMessage } from "ai";

type KeyInput = string | number | null | undefined;

export function uniqueSiblingKeys<T>(
  items: readonly T[],
  keyOf: (item: T, index: number) => KeyInput,
): string[] {
  const seen = new Map<string, number>();
  return items.map((item, index) => {
    const raw = keyOf(item, index);
    const base = normalizeKey(raw, `item:missing:${index}`);
    const count = seen.get(base) ?? 0;
    seen.set(base, count + 1);
    return count === 0 ? base : `${base}~${count}`;
  });
}

export function chatMessageKeys(messages: readonly Pick<UIMessage, "id">[]): string[] {
  return uniqueSiblingKeys(messages, (message, index) =>
    message.id ? `message:${message.id}` : `message:missing:${index}`,
  );
}

export function chatPartKeys(parts: readonly UIMessage["parts"][number][]): string[] {
  return uniqueSiblingKeys(parts, chatPartKeyBase);
}

export function askOptionKeys(options: readonly { id?: string; label?: string }[]): string[] {
  return uniqueSiblingKeys(options, (option, index) => {
    const id = option.id?.trim();
    if (id) return `ask-option:id:${id}`;
    const label = option.label?.trim();
    return label ? `ask-option:label:${label}` : `ask-option:missing:${index}`;
  });
}

function chatPartKeyBase(part: UIMessage["parts"][number], index: number): string {
  const toolCallId = (part as { toolCallId?: unknown }).toolCallId;
  if (typeof toolCallId === "string" && toolCallId.trim()) return `part:tool:${toolCallId.trim()}`;

  const typed = part as { type?: unknown; id?: unknown };
  const type = typeof typed.type === "string" && typed.type.trim() ? typed.type.trim() : "unknown";
  const id = typeof typed.id === "string" && typed.id.trim() ? typed.id.trim() : "";
  return id ? `part:${type}:${id}` : `part:${type}:${index}`;
}

function normalizeKey(value: KeyInput, fallback: string): string {
  const key = value == null ? "" : String(value).trim();
  return key || fallback;
}
