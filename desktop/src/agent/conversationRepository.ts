import { commands } from "../ipc/bindings";
import {
  normalizeConversationRecords,
  parseSerializedConversationRecords,
  serializeConversationRecord,
} from "./conversationHistory";
import type { ConversationRecord } from "./chatStore";

export function mergeConversationRecords(
  current: ConversationRecord[],
  incoming: ConversationRecord[],
): ConversationRecord[] {
  return normalizeConversationRecords([...current, ...incoming]);
}

export const conversationRepository = {
  async hydrate(): Promise<ConversationRecord[]> {
    try {
      const result = await commands.agentHistoryHydrate();
      if (result.status !== "ok") return [];
      return await parseSerializedConversationRecords<ConversationRecord>(result.data);
    } catch {
      return [];
    }
  },

  async sync(): Promise<ConversationRecord[]> {
    try {
      const result = await commands.agentHistorySync();
      if (result.status !== "ok") return [];
      return await parseSerializedConversationRecords<ConversationRecord>(result.data);
    } catch {
      return [];
    }
  },

  save(record: ConversationRecord): void {
    void (async () => {
      const serialized = await serializeConversationRecord(record);
      if (serialized) await commands.agentHistorySave(record.id, serialized);
    })().catch(() => undefined);
  },
};
