import { tool } from "ai";
import { z } from "zod";

import { askUserOptionSchema } from "../types";

/** Tool name whose call the loop renders as interactive chips and ends the turn. */
export const ASK_USER_TOOL = "ask_user_question";

/**
 * A native AI SDK CLIENT-SIDE tool: intentionally NO `execute`. The SDK surfaces
 * the tool call and pauses the turn (it can't run it); the loop (agent.ts) turns
 * the call into interactive chips, and the app supplies the result — the user's
 * pick — which resumes the same turn (see `resumeTurn`). This is why the model
 * must NOT be told to "stop and wait": the framework already pauses for it.
 */
export const askUserQuestion = () =>
  tool({
    description:
      "Ask the user to choose among concrete options via clickable chips instead of free text — e.g. which base pack to use, or which feature areas they want. Prefer this over a plain markdown question whenever the choice is a short, well-defined set. The UI always shows a free-text field alongside the options, so do NOT add an 'Other' / '其他' option yourself. The user's selection (possibly including a typed custom answer) comes back to you as this tool's result.",
    inputSchema: z.object({
      question: z.string().describe("The question to ask the user, in the user's language."),
      // Reuse the single option schema (see types.ts) — don't re-declare the shape.
      options: z.array(askUserOptionSchema).min(2).describe("The selectable options (at least 2)."),
      multi_select: z
        .boolean()
        .default(false)
        .describe("true = the user may pick several options; false = pick exactly one."),
    }),
    // No execute → client-side tool.
  });
