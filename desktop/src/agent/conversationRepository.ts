import { commands } from "../ipc/bindings";
import { parseSerializedConversationRecords } from "./conversationHistory";
import type { ConversationRecord } from "./chatStore";
import {
  CONVERSATION_PRIVACY_VERSION,
  projectConversationForPersistence,
} from "./conversationPrivacy";

const CONVERSATION_LIMIT = 50;

async function normalizeStoredRecords(
  records: ConversationRecord[],
  currentOwnerId: string | null,
): Promise<ConversationRecord[]> {
  const normalized: ConversationRecord[] = [];
  for (const record of records) {
    if (record.privacyVersion === CONVERSATION_PRIVACY_VERSION) {
      normalized.push(record);
      continue;
    }

    const projected = projectConversationForPersistence({
      ...record,
      updatedAt: Math.max(record.updatedAt + 1, Date.now()),
    });
    try {
      await commands.agentHistorySave(
        projected.id,
        JSON.stringify(projected),
        currentOwnerId,
      );
    } catch {
      // The sanitized in-memory record remains usable; its newer timestamp lets a
      // later sync retry replacing an older cloud copy.
    }
    normalized.push(projected);
  }
  return normalized;
}

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
  async hydrate(currentOwnerId: string | null): Promise<ConversationRecord[]> {
    try {
      const result = await commands.agentHistoryHydrate();
      if (result.status !== "ok") return [];
      return normalizeStoredRecords(
        parseSerializedConversationRecords<ConversationRecord>(result.data),
        currentOwnerId,
      );
    } catch {
      return [];
    }
  },

  async sync(currentOwnerId: string): Promise<ConversationRecord[] | null> {
    try {
      const result = await commands.agentHistorySync(currentOwnerId);
      if (result.status !== "ok") return null;
      return normalizeStoredRecords(
        parseSerializedConversationRecords<ConversationRecord>(result.data),
        currentOwnerId,
      );
    } catch {
      return null;
    }
  },

  save(record: ConversationRecord, currentOwnerId: string | null): void {
    const projected = projectConversationForPersistence(record);
    void commands.agentHistorySave(
      projected.id,
      JSON.stringify(projected),
      currentOwnerId,
    );
  },
};
