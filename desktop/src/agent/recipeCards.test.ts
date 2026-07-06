import { describe, expect, it } from "vitest";

import { parseRecipeCardBlocks } from "./recipeCards";

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
});
