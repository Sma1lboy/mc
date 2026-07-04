import { tool } from "ai";
import { z } from "zod";

export const listInstances = () =>
  tool({
    description:
      "List the launcher's existing game instances (id, name, Minecraft version, loader). Read-only. Use it when the user refers to what they already have installed, or to confirm an install landed.",
    inputSchema: z.object({}),
    // No execute: launcher client runs this through Rust IPC and appends output.
  });
