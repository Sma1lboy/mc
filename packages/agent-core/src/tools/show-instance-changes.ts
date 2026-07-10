import { tool } from "ai";
import { z } from "zod";

export const SHOW_INSTANCE_CHANGES_TOOL = "show_instance_changes";

const operationSchema = z.discriminatedUnion("type", [
  z
    .object({
      type: z.literal("set_memory"),
      memory_mb: z.number().int().min(512).max(32768),
    })
    .strict(),
  z
    .object({
      type: z.literal("set_mod_enabled"),
      file_name: z.string(),
      enabled: z.boolean(),
    })
    .strict(),
  z
    .object({
      type: z.literal("delete_mod"),
      file_name: z.string(),
    })
    .strict(),
  z
    .object({
      type: z.literal("install_mod"),
      provider: z.enum(["modrinth", "curseforge"]),
      project_id: z.string(),
      title: z.string().optional(),
    })
    .strict(),
]);

export const showInstanceChanges = () =>
  tool({
    description:
      "Show proposed changes for the currently bound instance as a confirmation card. Nothing changes until the user confirms in the launcher. Use only concrete file names from diagnose_instance or project ids from provider tools. The launcher injects root, instance id, Minecraft version, and loader.",
    inputSchema: z
      .object({
        summary: z.string().describe("One concise user-facing summary of why these changes help."),
        operations: z.array(operationSchema).min(1).max(20),
      })
      .strict(),
    // No execute: the launcher renders a confirmation card and performs approved operations.
  });
