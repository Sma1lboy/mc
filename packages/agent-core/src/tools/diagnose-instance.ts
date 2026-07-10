import { tool } from "ai";
import { z } from "zod";

export const diagnoseInstance = () =>
  tool({
    description:
      "Diagnose the currently bound installed instance. Checks enabled mod metadata, duplicate ids, loader mismatches, memory recommendation, and recent crash evidence. Read-only. The launcher injects the root and instance id; request the bounded log tail only when needed for deeper debugging.",
    inputSchema: z
      .object({
        include_log_tail: z
          .boolean()
          .optional()
          .describe("Return the bounded recent log tail in addition to structured issues."),
      })
      .strict(),
    // No execute: launcher client injects instance context and runs this through Rust IPC.
  });
