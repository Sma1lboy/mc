import { tool } from "ai";
import { z } from "zod";

const sessionIdSchema = z
  .string()
  .min(1)
  .max(128)
  .describe("Opaque session id returned after confirm_deep_diagnosis is approved.");

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

/** Client-side confirmation boundary for the visible diagnostic launch. */
export const CONFIRM_DEEP_DIAGNOSIS_TOOL = "confirm_deep_diagnosis";

export const confirmDeepDiagnosis = () =>
  tool({
    description:
      "Show a confirmation card for a visible, offline, time-bounded baseline launch of a temporary instance copy. Call when diagnose_instance is insufficient; the card itself requests approval, so do not ask for separate permission first. This executes installed Mods with normal OS permissions and is not a hostile-code, OS, or network security sandbox. The result is either the started session or { approved: false }.",
    inputSchema: z
      .object({
        reason: z.string().min(1).max(400).describe("Concise reason a test launch is needed."),
      })
      .strict(),
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
