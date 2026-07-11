import { commands } from "../ipc/bindings";
import { parseSerializedConversationRecords } from "./conversationHistory";
import type { ConversationRecord } from "./chatStore";

const CONVERSATION_LIMIT = 50;

export function mergeConversationRecords(
  current: ConversationRecord[],
  incoming: ConversationRecord[],
): ConversationRecord[] {
  const byId = new Map(current.map((record) => [record.id, record] as const));
  for (const record of incoming) {
    const existing = byId.get(record.id);
    if (!existing || record.updatedAt >= existing.updatedAt) byId.set(record.id, record);
  }
  return [...byId.values()]
    .sort((a, b) => b.updatedAt - a.updatedAt)
    .slice(0, CONVERSATION_LIMIT);
}

export const conversationRepository = {
  async hydrate(): Promise<ConversationRecord[]> {
    try {
      const result = await commands.agentHistoryHydrate();
      if (result.status !== "ok") return [];
      return parseSerializedConversationRecords<ConversationRecord>(result.data);
    } catch {
      return [];
    }
  },

  async sync(): Promise<ConversationRecord[]> {
    try {
      const result = await commands.agentHistorySync();
      if (result.status !== "ok") return [];
      return parseSerializedConversationRecords<ConversationRecord>(result.data);
    } catch {
      return [];
    }
  },

  save(record: ConversationRecord): void {
    void commands.agentHistorySave(record.id, JSON.stringify(record));
  },
};
