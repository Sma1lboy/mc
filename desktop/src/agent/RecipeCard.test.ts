import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

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
});
