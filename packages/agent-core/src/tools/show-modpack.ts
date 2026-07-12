import { tool } from "ai";
import { z } from "zod";

/** Tool name whose call the UI renders as an installable modpack card. */
export const SHOW_MODPACK_TOOL = "show_modpack";

const basePack = z.object({
  provider: z.string().optional().describe('"modrinth" (default) or "curseforge".'),
  project_id: z.string(),
  version_id: z.string().describe("The exact pack version id (from search/inspect results)."),
  title: z.string(),
  mc_version: z.string().optional(),
  loader: z.string().optional(),
});

const builtPack = z.object({
  path: z.string().describe("The output_path from the approved confirm_modpack_build result, verbatim."),
  title: z.string().describe("A short display name for the built pack."),
  mc_version: z.string().optional(),
  loader: z.string().optional(),
});

/**
 * A native CLIENT-SIDE tool (no `execute`, like `ask_user_question`): the UI
 * renders the pack as a card with an Install button and the turn pauses. The
 * install runs on the user's click — the model never installs anything itself —
 * and the outcome (`{ installed, instance_id? }`) resumes the turn.
 */
export const showModpack = () =>
  tool({
    description:
      "Show the final modpack to the user as an installable card and pause for their action. Pass EXACTLY ONE of `base` / `mrpack`: `base` when the plan is just a ready-made pack with NO extra mods; `mrpack` after an approved confirm_modpack_build, with its output_path. The user's install (or skip) comes back as this tool's result.",
    inputSchema: z.object({
      base: basePack.nullish().describe("The recommended ready-made pack, when no extra mods were added."),
      mrpack: builtPack.nullish().describe("The built .mrpack, when extra mods were added."),
    }),
    // No execute → client-side tool.
  });
