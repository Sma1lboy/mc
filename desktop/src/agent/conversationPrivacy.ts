import type { UIMessage } from "ai";
import type { AgentToolContext } from "./agentContext";
import type { ConversationRecord } from "./chatStore";

export const CONVERSATION_PRIVACY_VERSION = 1;

const REDACTED = "[REDACTED]";
const LOCAL_PATH = "[LOCAL_PATH]";

const SENSITIVE_KEYS = [
  "password",
  "passwd",
  "passphrase",
  "secret",
  "token",
  "accesstoken",
  "refreshtoken",
  "apikey",
  "authorization",
  "auth",
  "credential",
  "privatekey",
  "webhook",
  "cookie",
  "databaseurl",
  "connectionstring",
];

const LARGE_PRIVATE_KEYS = new Set([
  "raw",
  "body",
  "logtail",
  "manifest",
  "downloadurl",
  "sha1",
  "sha256",
  "sha512",
  "sessionid",
  "outputpath",
]);

const WIKI_TOOL_TYPES = new Set(["tool-wiki_search", "tool-wiki_open"]);

export interface PublicShareTextPart {
  type: "text";
  text: string;
}

export interface PublicShareQuestionPart {
  type: "tool-ask_user_question";
  input: {
    question: string;
    options: Array<{ label: string; description?: string }>;
  };
  output: { selected: string[] };
}

export interface PublicShareMessage {
  role: "user" | "assistant";
  parts: Array<PublicShareTextPart | PublicShareQuestionPart>;
}

function normalizedKey(key: string): string {
  return key.toLowerCase().replace(/[^a-z0-9]/g, "");
}

function isSensitiveKey(key: string): boolean {
  const normalized = normalizedKey(key);
  return SENSITIVE_KEYS.some((part) => normalized === part || normalized.endsWith(part));
}

function privateContextValues(context: AgentToolContext | null | undefined): string[] {
  const values = [
    context?.root,
    context?.instance?.root,
    ...(context?.instance?.sourcePaths ?? []),
    context?.wiki?.root,
    ...(context?.wiki?.sourcePaths ?? []),
  ].filter((value): value is string => typeof value === "string" && value.length > 0);
  const variants = new Set<string>();
  for (const value of values) {
    variants.add(value);
    variants.add(value.replaceAll("\\", "/"));
    variants.add(value.replaceAll("/", "\\"));
  }
  return [...variants].sort((left, right) => right.length - left.length);
}

function replacePrivateValues(value: string, privateValues: string[]): string {
  let sanitized = value;
  for (const privateValue of privateValues) {
    sanitized = sanitized.split(privateValue).join(LOCAL_PATH);
  }
  return sanitized;
}

function sanitizeCredentialText(value: string): string {
  return value
    .replace(/\bBearer\s+[A-Za-z0-9._~+\/-]+=*/gi, `Bearer ${REDACTED}`)
    .replace(/\b(?:sk|pk|ghp|glpat)-[A-Za-z0-9_-]{12,}\b/g, REDACTED)
    .replace(/\bCookie\s*:\s*[^\r\n]+/gi, `Cookie: ${REDACTED}`)
    .replace(
      /(^|[\s,{;])(password|passwd|passphrase|secret|token|access[_-]?token|refresh[_-]?token|api[_-]?key|authorization|auth|credential|private[_-]?key|webhook|cookie|database[_-]?url|connection[_-]?string)(\s*[:=]\s*)("[^"]*"|'[^']*'|[^\r\n,;}]+)/gim,
      (_match, prefix: string, key: string, separator: string) =>
        `${prefix}${key}${separator}${REDACTED}`,
    );
}

function sanitizeLocalPathText(value: string): string {
  return value
    .replace(/file:\/\/[^\r\n"'`<>()]+/gi, LOCAL_PATH)
    .replace(/\\\\[^\r\n"'`<>()]+/g, LOCAL_PATH)
    .replace(/~[\\/][^\r\n"'`<>()]+/g, LOCAL_PATH)
    .replace(/[A-Za-z]:[\\/][^\r\n"'`<>()]+/g, LOCAL_PATH)
    .replace(/\/(?:Users|home|var|tmp|private|Volumes|opt|etc|Library|Applications)\/[^\r\n"'`<>()]+/g, LOCAL_PATH)
    .replace(/(^|[\s("'`])\/(?!\/)(?:[^/\s"'`<>()]+\/)+[^\s"'`<>()]+/g, (_match, prefix: string) => `${prefix}${LOCAL_PATH}`);
}

function sanitizeString(value: string, privateValues: string[]): string {
  return sanitizeCredentialText(
    sanitizeLocalPathText(replacePrivateValues(value, privateValues)),
  );
}

function sanitizePublicString(value: string): string {
  return sanitizeCredentialText(sanitizeLocalPathText(value))
    .replace(/https?:\/\/[^\s"'`<>()]+/gi, REDACTED)
    .replace(/\b[a-f0-9]{40,128}\b/gi, REDACTED);
}

function sanitizeValue(
  value: unknown,
  key: string,
  depth: number,
  sanitizeText: (text: string) => string,
): unknown {
  if (depth > 24) return REDACTED;
  if (isSensitiveKey(key) || LARGE_PRIVATE_KEYS.has(normalizedKey(key))) return REDACTED;
  if (typeof value === "string") return sanitizeText(value);
  if (Array.isArray(value)) {
    return value.map((item) => sanitizeValue(item, key, depth + 1, sanitizeText));
  }
  if (!value || typeof value !== "object") return value;

  const result: Record<string, unknown> = {};
  for (const [entryKey, entryValue] of Object.entries(value as Record<string, unknown>)) {
    if (entryKey === "providerMetadata") continue;
    result[entryKey] = sanitizeValue(entryValue, entryKey, depth + 1, sanitizeText);
  }
  return result;
}

function projectMessageForPersistence(message: UIMessage, privateValues: string[]): UIMessage {
  const sanitizeText = (value: string) => sanitizeString(value, privateValues);
  const raw = message as unknown as Record<string, unknown>;
  const rawParts = Array.isArray(raw.parts) ? raw.parts : [];
  const parts = rawParts.flatMap((part) => {
    if (!part || typeof part !== "object") return [part];
    const toolPart = part as Record<string, unknown>;
    if (toolPart.type === "reasoning") return [];
    if (typeof toolPart.type === "string" && WIKI_TOOL_TYPES.has(toolPart.type)) {
      const withoutPrivateOutput = "output" in toolPart
        ? {
            ...toolPart,
            output: {
              privacyRedacted: true,
              reason: "instance_content_not_persisted",
            },
          }
        : toolPart;
      return [sanitizeValue(withoutPrivateOutput, "part", 0, sanitizeText)];
    }
    return [sanitizeValue(toolPart, "part", 0, sanitizeText)];
  });
  const messageFields = { ...raw };
  delete messageFields.parts;
  const projected = sanitizeValue(
    messageFields,
    "message",
    0,
    sanitizeText,
  ) as Record<string, unknown>;
  return { ...projected, parts } as unknown as UIMessage;
}

function projectToolContext(context: AgentToolContext | null | undefined): AgentToolContext | null {
  if (!context) return null;
  const projected: AgentToolContext = {};
  if (context.mode) projected.mode = context.mode;
  if (context.instance) {
    projected.instance = {
      modpackId: context.instance.modpackId,
      instanceId: context.instance.instanceId,
      mcVersion: context.instance.mcVersion,
      loader: context.instance.loader,
    };
  } else if (context.wiki) {
    projected.wiki = {
      modpackId: context.wiki.modpackId,
      instanceId: context.wiki.instanceId,
    };
  }
  return projected;
}

export function projectConversationForPersistence(record: ConversationRecord): ConversationRecord {
  const privateValues = privateContextValues(record.toolContext);
  return {
    id: record.id,
    createdAt: record.createdAt,
    updatedAt: record.updatedAt,
    title: sanitizeString(record.title, privateValues),
    messages: record.messages.map((message) => projectMessageForPersistence(message, privateValues)),
    toolContext: projectToolContext(record.toolContext),
    ownerId: record.ownerId ?? null,
    privacyVersion: CONVERSATION_PRIVACY_VERSION,
  };
}

function publicQuestionPart(raw: Record<string, unknown>): PublicShareQuestionPart | null {
  if (!raw.input || typeof raw.input !== "object") return null;
  const input = raw.input as Record<string, unknown>;
  if (typeof input.question !== "string") return null;
  const options = Array.isArray(input.options)
    ? input.options.flatMap((option) => {
        if (!option || typeof option !== "object") return [];
        const value = option as Record<string, unknown>;
        if (typeof value.label !== "string") return [];
        return [{
          label: sanitizePublicString(value.label),
          ...(typeof value.description === "string"
            ? { description: sanitizePublicString(value.description) }
            : {}),
        }];
      })
    : [];
  const output = raw.output && typeof raw.output === "object"
    ? raw.output as Record<string, unknown>
    : {};
  const selected = Array.isArray(output.selected)
    ? output.selected.filter((value): value is string => typeof value === "string")
        .map(sanitizePublicString)
    : [];
  return {
    type: "tool-ask_user_question",
    input: {
      question: sanitizePublicString(input.question),
      options,
    },
    output: { selected },
  };
}

function visibleParts(message: UIMessage): Array<PublicShareTextPart | PublicShareQuestionPart> {
  const parts = (message as unknown as { parts?: unknown[] }).parts ?? [];
  const visible: Array<PublicShareTextPart | PublicShareQuestionPart> = [];
  for (const part of parts) {
    if (!part || typeof part !== "object") continue;
    const raw = part as Record<string, unknown>;
    if (raw.type === "text" && typeof raw.text === "string") {
      visible.push({ type: "text", text: sanitizePublicString(raw.text) });
      continue;
    }
    if (raw.type !== "tool-ask_user_question") continue;
    const question = publicQuestionPart(raw);
    if (question) visible.push(question);
  }
  return visible;
}

export function projectMessagesForPublicShare(messages: UIMessage[]): PublicShareMessage[] {
  const projected: PublicShareMessage[] = [];
  for (const message of messages) {
    if (message.role !== "user" && message.role !== "assistant") continue;
    const parts = visibleParts(message);
    if (parts.length === 0) continue;
    projected.push({ role: message.role, parts });
  }
  return projected;
}
