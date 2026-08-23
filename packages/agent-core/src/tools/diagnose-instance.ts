import { tool } from "ai";
import { z } from "zod";

export const diagnoseInstance = () =>
  tool({
    description:
      "Diagnose the currently bound installed instance with launcher checks plus bundled InterMed static analysis. Use mode inspect for read-only diagnosis. Use mode remediate only when the user explicitly asks to repair the instance; it may reversibly disable uniquely implicated enabled Mods, re-scan up to three times, and returns the final static result. It never edits Mod code/configs or launches Minecraft. The launcher injects root and instance id.",
    inputSchema: z
      .object({
        include_log_tail: z
          .boolean()
          .optional()
          .describe("Return the bounded recent log tail in addition to structured issues."),
        mode: z
          .enum(["inspect", "remediate"])
          .optional()
          .default("inspect")
          .describe(
            "inspect is read-only; remediate is only for explicit repair intent and may reversibly disable uniquely implicated Mods before re-scanning.",
          ),
      })
      .strict(),
    // No execute: launcher client injects instance context and runs this through Rust IPC.
  });
