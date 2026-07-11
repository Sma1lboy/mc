import { tool } from "ai";
import { z } from "zod";

const sessionIdSchema = z
  .string()
  .min(1)
  .max(128)
  .describe("Opaque session id returned by start_deep_diagnosis.");

const diagnosticOperationSchema = z.discriminatedUnion("type", [
  z
    .object({
      type: z.literal("set_memory"),
      memory_mb: z.number().int().min(512).max(32768),
    })
    .strict(),
  z
    .object({
      type: z.literal("set_mod_enabled"),
      file_name: z.string().min(1),
      enabled: z.boolean(),
    })
    .strict(),
  z
    .object({
      type: z.literal("delete_mod"),
      file_name: z.string().min(1),
    })
    .strict(),
]);

export const startDeepDiagnosis = () =>
  tool({
    description:
      "Create a temporary filesystem copy of the bound instance and run one unchanged, offline, time-bounded baseline launch. Use only after diagnose_instance is insufficient and the user explicitly asks for or approves a visible test launch. This executes installed Mods with normal OS permissions; it is not a hostile-code, OS, or network security sandbox. The launcher injects root and instance id.",
    inputSchema: z.object({}).strict(),
  });

export const runDiagnosticTrial = () =>
  tool({
    description:
      "Run one independent hypothesis against a fresh copy of a deep-diagnosis baseline. Allowed changes are only memory, enable/disable a concrete Mod file, or delete a concrete Mod file inside the temporary copy. Never use this for source, script, arbitrary config text, JVM argument, command, or JAR-content changes. Returns bounded launch evidence and never changes the installed instance.",
    inputSchema: z
      .object({
        session_id: sessionIdSchema,
        operations: z.array(diagnosticOperationSchema).min(1).max(10),
      })
      .strict(),
  });

export const finishDeepDiagnosis = () =>
  tool({
    description:
      "Finish a deep-diagnosis session, return all recorded baseline/trial outcomes, and delete its temporary files. Call after the last useful trial or when abandoning diagnosis. A successful trial is evidence only; propose matching installed-instance remediation through show_instance_changes for explicit user confirmation.",
    inputSchema: z
      .object({
        session_id: sessionIdSchema,
      })
      .strict(),
  });
