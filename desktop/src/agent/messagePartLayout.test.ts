import { describe, expect, it } from "vitest";
import type { UIMessage } from "ai";

import { layoutMessageParts } from "./messagePartLayout";

type Part = UIMessage["parts"][number];

const text = (value: string): Part => ({ type: "text", text: value });
const reasoning = (value: string): Part => ({ type: "reasoning", text: value });
const tool = (name: string): Part =>
  ({
    type: `tool-${name}`,
    toolCallId: `call-${name}`,
    state: "input-available",
    input: {},
  }) as Part;

describe("layoutMessageParts", () => {
  it("keeps an interactive confirmation card outside later activity", () => {
    const parts = [
      text("proposal"),
      tool("confirm_modpack_build"),
      reasoning("continued reasoning"),
      tool("search_mods"),
      text("final"),
    ];

    expect(layoutMessageParts(parts)).toEqual([
      { kind: "part", index: 0 },
      { kind: "part", index: 1 },
      { kind: "activity", indices: [2, 3] },
      { kind: "part", index: 4 },
    ]);
  });

  it("preserves the existing grouped activity followed by final text", () => {
    const parts = [reasoning("thinking"), tool("wiki_search"), text("answer")];

    expect(layoutMessageParts(parts)).toEqual([
      { kind: "activity", indices: [0, 1] },
      { kind: "part", index: 2 },
    ]);
  });
});
