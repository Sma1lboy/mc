export type InstanceChangeOperation =
  | { type: "set_memory"; memory_mb: number }
  | { type: "set_mod_enabled"; file_name: string; enabled: boolean }
  | { type: "delete_mod"; file_name: string }
  | {
      type: "install_mod";
      provider: "modrinth" | "curseforge";
      project_id: string;
      title?: string;
    };

export interface NormalizedInstanceChangePlan {
  operations: InstanceChangeOperation[];
  error?: "empty_plan" | "too_many_operations" | "invalid_operation";
}

export function normalizeInstanceChangeOperations(value: unknown): NormalizedInstanceChangePlan {
  if (!Array.isArray(value) || value.length === 0) {
    return { operations: [], error: "empty_plan" };
  }
  if (value.length > 20) {
    return { operations: [], error: "too_many_operations" };
  }

  const operations: InstanceChangeOperation[] = [];
  for (const raw of value) {
    const operation = normalizeOperation(raw);
    if (!operation) return { operations: [], error: "invalid_operation" };
    operations.push(operation);
  }
  return { operations };
}

function normalizeOperation(value: unknown): InstanceChangeOperation | null {
  if (!value || typeof value !== "object") return null;
  const operation = value as Record<string, unknown>;
  switch (operation.type) {
    case "set_memory":
      return typeof operation.memory_mb === "number" &&
        Number.isInteger(operation.memory_mb) &&
        operation.memory_mb >= 512 &&
        operation.memory_mb <= 32768
        ? { type: "set_memory", memory_mb: operation.memory_mb }
        : null;
    case "set_mod_enabled":
      return safeSegment(operation.file_name) && typeof operation.enabled === "boolean"
        ? {
            type: "set_mod_enabled",
            file_name: operation.file_name,
            enabled: operation.enabled,
          }
        : null;
    case "delete_mod":
      return safeSegment(operation.file_name)
        ? { type: "delete_mod", file_name: operation.file_name }
        : null;
    case "install_mod": {
      if (
        (operation.provider !== "modrinth" && operation.provider !== "curseforge") ||
        !safeIdentifier(operation.project_id) ||
        (operation.title !== undefined && typeof operation.title !== "string")
      ) {
        return null;
      }
      return {
        type: "install_mod",
        provider: operation.provider,
        project_id: operation.project_id,
        ...(typeof operation.title === "string" ? { title: operation.title } : {}),
      };
    }
    default:
      return null;
  }
}

function safeSegment(value: unknown): value is string {
  return (
    typeof value === "string" &&
    value.length > 0 &&
    value !== "." &&
    value !== ".." &&
    !value.includes("/") &&
    !value.includes("\\") &&
    !value.includes("\0")
  );
}

function safeIdentifier(value: unknown): value is string {
  return typeof value === "string" && value.trim().length > 0 && !/[\\/\0]/.test(value);
}
