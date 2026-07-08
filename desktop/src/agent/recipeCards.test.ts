import { describe, expect, it } from "vitest";

import {
  parseRecipeCardBlocks,
  recipeCardIconIdsKey,
  recipeItemDisplayName,
} from "./recipeCards";

describe("parseRecipeCardBlocks", () => {
  it("extracts recipe_card fenced JSON while preserving surrounding markdown", () => {
    const text = [
      "安山机壳这样做：",
      "",
      "```recipe_card",
      JSON.stringify({
        version: 1,
        type: "crafting_shaped",
        title: "安山机壳",
        result: { id: "create:andesite_casing", label: "安山机壳", count: 1 },
        grid: [
          [
            { id: "#minecraft:planks", label: "任意木板" },
            { id: "#minecraft:planks", label: "任意木板" },
            { id: "#minecraft:planks", label: "任意木板" },
          ],
          [
            { id: "#minecraft:planks", label: "任意木板" },
            { id: "create:andesite_alloy", label: "安山合金" },
            { id: "#minecraft:planks", label: "任意木板" },
          ],
          [
            { id: "#minecraft:planks", label: "任意木板" },
            { id: "#minecraft:planks", label: "任意木板" },
            { id: "#minecraft:planks", label: "任意木板" },
          ],
        ],
        source_document_ids: ["doc:quest"],
      }),
      "```",
      "",
      "需要 8 个木板和 1 个安山合金。",
    ].join("\n");

    const parts = parseRecipeCardBlocks(text);

    expect(parts).toHaveLength(3);
    expect(parts[0]).toEqual({ type: "markdown", text: "安山机壳这样做：" });
    expect(parts[1]).toMatchObject({
      type: "recipe_card",
      card: {
        title: "安山机壳",
        result: { id: "create:andesite_casing", label: "安山机壳", count: 1 },
        source_document_ids: ["doc:quest"],
      },
    });
    if (parts[1].type !== "recipe_card") throw new Error("expected recipe card");
    expect(parts[1].card.grid?.[1]?.[1]).toEqual({
      id: "create:andesite_alloy",
      label: "安山合金",
    });
    expect(parts[2]).toEqual({
      type: "markdown",
      text: "需要 8 个木板和 1 个安山合金。",
    });
  });

  it("keeps malformed recipe_card fences as markdown", () => {
    const text = "```recipe_card\n{ nope\n```";

    expect(parseRecipeCardBlocks(text)).toEqual([{ type: "markdown", text }]);
  });

  it("normalizes legacy chunk citations to parent document citations", () => {
    const parts = parseRecipeCardBlocks([
      "```recipe_card",
      JSON.stringify({
        version: 1,
        type: "crafting_shaped",
        result: { id: "create:andesite_casing", label: "安山机壳" },
        source_chunk_ids: ["chunk:abcd:2:efgh"],
      }),
      "```",
    ].join("\n"));

    if (parts[0].type !== "recipe_card") throw new Error("expected recipe card");
    expect(parts[0].card.source_document_ids).toEqual(["doc:abcd"]);
    expect(parts[0].card.source_chunk_ids).toBeUndefined();
  });

  it("builds a stable icon lookup key for reparsed recipe cards", () => {
    const text = [
      "```recipe_card",
      JSON.stringify({
        version: 1,
        type: "crafting_shaped",
        result: { id: "create:andesite_casing", label: "安山机壳" },
        grid: [
          [
            { id: "#minecraft:planks", label: "任意木板" },
            { id: "#minecraft:planks", label: "任意木板" },
            { id: "#minecraft:planks", label: "任意木板" },
          ],
          [
            { id: "#minecraft:planks", label: "任意木板" },
            { id: "create:andesite_alloy", label: "安山合金" },
            { id: "#minecraft:planks", label: "任意木板" },
          ],
          [
            { id: "#minecraft:planks", label: "任意木板" },
            { id: "#minecraft:planks", label: "任意木板" },
            { id: "#minecraft:planks", label: "任意木板" },
          ],
        ],
      }),
      "```",
    ].join("\n");

    const first = parseRecipeCardBlocks(text);
    const second = parseRecipeCardBlocks(text);
    if (first[0].type !== "recipe_card" || second[0].type !== "recipe_card") {
      throw new Error("expected recipe cards");
    }

    expect(first[0].card).not.toBe(second[0].card);
    expect(recipeCardIconIdsKey(first[0].card)).toBe(recipeCardIconIdsKey(second[0].card));
    expect(recipeCardIconIdsKey(first[0].card)).toBe(
      "create:andesite_casing\u0001#minecraft:planks\u0001create:andesite_alloy",
    );
  });

  it("keeps recipe tags as backend icon lookup ids", () => {
    const parts = parseRecipeCardBlocks([
      "```recipe_card",
      JSON.stringify({
        version: 1,
        type: "crafting_shaped",
        result: { id: "create:andesite_alloy", label: "安山合金" },
        grid: [
          [
            { id: "#forge:nuggets/iron", label: "铁粒" },
            { id: "minecraft:andesite", label: "安山岩" },
          ],
        ],
      }),
      "```",
    ].join("\n"));
    if (parts[0].type !== "recipe_card") throw new Error("expected recipe card");

    expect(recipeCardIconIdsKey(parts[0].card)).toBe(
      "create:andesite_alloy\u0001#forge:nuggets/iron\u0001minecraft:andesite",
    );
  });

  it("keeps quartz and redstone tags as backend icon lookup ids", () => {
    const parts = parseRecipeCardBlocks([
      "```recipe_card",
      JSON.stringify({
        version: 1,
        type: "crafting_shapeless",
        result: { id: "create:rose_quartz", label: "玫瑰石英" },
        ingredients: [
          { id: "#forge:gems/quartz", label: "下界石英" },
          { id: "#forge:dusts/redstone", label: "红石粉", count: 8 },
        ],
      }),
      "```",
    ].join("\n"));
    if (parts[0].type !== "recipe_card") throw new Error("expected recipe card");

    expect(recipeCardIconIdsKey(parts[0].card)).toBe(
      "create:rose_quartz\u0001#forge:gems/quartz\u0001#forge:dusts/redstone",
    );
  });

  it("uses user-facing item labels instead of namespaced ids", () => {
    expect(recipeItemDisplayName({
      id: "create:polished_rose_quartz",
      label: "create:polished_rose_quartz",
    })).toBe("polished rose quartz");
  });
});
