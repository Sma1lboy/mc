import { describe, expect, it } from "vitest";

import { parseRecipeCardBlocks, recipeCardIconIdsKey } from "./recipeCards";

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
        source_chunk_ids: ["chunk:quest:0"],
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
        source_chunk_ids: ["chunk:quest:0"],
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
      "create:andesite_casing\u0001minecraft:oak_planks\u0001create:andesite_alloy",
    );
  });

  it("uses representative item icons for common recipe tags", () => {
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
      "create:andesite_alloy\u0001minecraft:iron_nugget\u0001minecraft:andesite",
    );
  });

  it("uses representative item icons for quartz and redstone recipe tags", () => {
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
      "create:rose_quartz\u0001minecraft:quartz\u0001minecraft:redstone",
    );
  });
});
