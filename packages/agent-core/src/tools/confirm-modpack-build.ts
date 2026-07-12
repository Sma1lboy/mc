import { tool } from "ai";
import { z } from "zod";

import { modpackPlanSchema } from "./build-plan-schema";

/** Client-side confirmation boundary for the disk-writing modpack build. */
export const CONFIRM_MODPACK_BUILD_TOOL = "confirm_modpack_build";

export const confirmModpackBuild = () =>
  tool({
    description:
      "Show the exact validated custom modpack plan as a confirmation card. The launcher builds only if the user clicks Build. Call this after validate_modpack_plan returns a non-blocked report; never ask for a separate plain-text confirmation first. The tool result is either the build output or { approved: false }.",
    inputSchema: modpackPlanSchema
      .extend({
        output_filename: z
          .string()
          .describe(
            'Output file name (no path). ".mrpack" is appended if missing. The launcher decides the directory.',
          ),
      })
      .strict(),
    // No execute: the launcher renders the card and owns the privileged IPC call.
  });
