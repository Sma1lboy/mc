import { tool } from "ai";
import { z } from "zod";

import type { ToolExecutor } from "../types";

export const listInstances = (exec: ToolExecutor) =>
  tool({
    description:
      "List the launcher's existing game instances (id, name, Minecraft version, loader). Read-only. Use it when the user refers to what they already have installed, or to confirm an install landed.",
    inputSchema: z.object({}),
    execute: (args) => exec.list_instances(args),
  });
