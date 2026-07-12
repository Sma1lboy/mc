import { tool } from "ai";

import { modpackPlanSchema } from "./build-plan-schema";

export const validateModpackPlan = () =>
  tool({
    description:
      "Validate the exact base-pack and mod versions against the target Minecraft version and loader. Reports incompatible selections, missing required dependencies, declared conflicts, and duplicates. Read-only: call this on the final plan before asking for build confirmation.",
    inputSchema: modpackPlanSchema,
    // No execute: launcher client runs this through Rust IPC and appends output.
  });
