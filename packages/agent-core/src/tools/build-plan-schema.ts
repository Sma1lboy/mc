import { z } from "zod";

export const buildTargetSchema = z
  .object({
    mc_version: z.string(),
    loader: z.string(),
  })
  .strict();

export const buildBasePackSchema = z
  .object({
    project_id: z.string().describe("Modrinth project id of the base pack."),
    version_id: z
      .string()
      .describe("The exact base pack version id to build on (from inspect/search)."),
    title: z.string().optional(),
    slug: z.string().optional(),
  })
  .strict();

export const buildModRefSchema = z
  .object({
    provider: z.string().optional().describe('"modrinth" (default) or "curseforge".'),
    project_id: z.string(),
    version_id: z.string().describe("The resolved version id (from resolve_mods)."),
    title: z.string().optional(),
  })
  .strict();

export const modpackPlanSchema = z
  .object({
    target: buildTargetSchema,
    base_pack: buildBasePackSchema
      .nullish()
      .describe("The chosen base pack, or null to start from scratch (empty base)."),
    extra_mods: z
      .array(buildModRefSchema)
      .default([])
      .describe("Extra mods to add, as resolved refs from resolve_mods."),
  })
  .strict();
