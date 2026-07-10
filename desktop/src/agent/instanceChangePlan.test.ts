import { describe, expect, it } from "vitest";

import { normalizeInstanceChangeOperations } from "./instanceChangePlan";

describe("normalizeInstanceChangeOperations", () => {
  it("accepts the four supported operation shapes", () => {
    const result = normalizeInstanceChangeOperations([
      { type: "set_memory", memory_mb: 4096 },
      { type: "set_mod_enabled", file_name: "example.jar", enabled: false },
      { type: "delete_mod", file_name: "duplicate.jar" },
      { type: "install_mod", provider: "modrinth", project_id: "sodium" },
    ]);

    expect(result.error).toBeUndefined();
    expect(result.operations).toHaveLength(4);
  });

  it("rejects empty plans and out-of-range memory", () => {
    expect(normalizeInstanceChangeOperations([]).error).toBe("empty_plan");
    expect(
      normalizeInstanceChangeOperations([{ type: "set_memory", memory_mb: 511 }]).error,
    ).toBe("invalid_operation");
  });

  it("rejects unsafe file names and unsupported providers", () => {
    expect(
      normalizeInstanceChangeOperations([
        { type: "set_mod_enabled", file_name: "../escape.jar", enabled: false },
      ]).error,
    ).toBe("invalid_operation");
    expect(
      normalizeInstanceChangeOperations([
        { type: "install_mod", provider: "unknown", project_id: "example" },
      ]).error,
    ).toBe("invalid_operation");
  });

  it("rejects the whole plan instead of silently dropping an invalid item", () => {
    const result = normalizeInstanceChangeOperations([
      { type: "set_memory", memory_mb: 4096 },
      { type: "delete_mod", file_name: "../../bad.jar" },
    ]);

    expect(result.error).toBe("invalid_operation");
    expect(result.operations).toEqual([]);
  });
});
