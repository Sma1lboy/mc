import { tool } from "ai";
import { z } from "zod";

import { modpackPlanSchema } from "./build-plan-schema";

export const buildModpack = () =>
  tool({
    description:
      "Deterministically build and verify a .mrpack from a base pack (or from scratch) plus extra mods. THIS WRITES TO DISK — only call it after the user has explicitly confirmed the final plan.",
    inputSchema: modpackPlanSchema.extend({
      output_filename: z
        .string()
        .describe(
          'Output file name (no path). ".mrpack" is appended if missing. The launcher decides the directory.',
        ),
    }),
    // No execute: launcher client runs this through Rust IPC and appends output.
  });
