import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { RecipeCard } from "./RecipeCard";

describe("RecipeCard", () => {
  it("does not render internal recipe type labels", () => {
    const html = renderToStaticMarkup(createElement(RecipeCard, {
      card: {
        version: 1,
        type: "create:sandpaper_polishing",
        title: "抛光玫瑰石英",
        result: { id: "create:polished_rose_quartz", label: "抛光玫瑰石英" },
        ingredients: [{ id: "create:rose_quartz", label: "玫瑰石英" }],
      },
    }));

    expect(html).not.toContain("create:sandpaper_polishing");
    expect(html).not.toContain("砂纸打磨");
    expect(html).not.toContain("mc-recipe-kind");
  });

  it("renders the crafting arrow as fixed pixel art instead of rotated segments", () => {
    const html = renderToStaticMarkup(createElement(RecipeCard, {
      card: {
        version: 1,
        type: "minecraft:crafting_shaped",
        title: "真空管",
        result: { id: "create:electron_tube", label: "真空管" },
        grid: [
          [{ id: "create:polished_rose_quartz", label: "抛光玫瑰石英" }, null, null],
          [{ id: "minecraft:iron_sheet", label: "铁板" }, null, null],
        ],
      },
    }));
    const cssPath = resolve(dirname(fileURLToPath(import.meta.url)), "chat.css");
    const css = readFileSync(cssPath, "utf8");
    const arrowCss = css.match(/\.mc-recipe-arrow[\s\S]*?\.mc-recipe-source/)?.[0] ?? "";

    expect(html).toContain('class="mc-recipe-arrow"');
    expect(html).not.toContain("mc-recipe-arrow\"><span");
    expect(arrowCss).toContain("linear-gradient");
    expect(arrowCss).not.toContain("rotate(");
    expect(arrowCss).not.toContain("transform:");
  });
});
