import { tool } from "ai";
import { z } from "zod";

import type { ToolExecutor } from "../types";

export const installModpack = (exec: ToolExecutor) =>
  tool({
    description:
      "Install a .mrpack produced by build_modpack in THIS conversation into the launcher as a playable instance. THIS WRITES TO DISK — only call it after build_modpack succeeded and the user has said they want it installed. Returns the new instance id.",
    inputSchema: z.object({
      path: z
        .string()
        .describe("The output_path from the successful build_modpack result, passed through exactly as returned."),
    }),
    execute: (args) => exec.install_modpack(args),
  });
