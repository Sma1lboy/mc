import { describe, expect, it } from "vitest";

import { askOptionKeys, chatMessageKeys, chatPartKeys, uniqueSiblingKeys } from "./renderKeys";

describe("chat render keys", () => {
  it("deduplicates repeated sibling keys without falling back to raw indexes", () => {
    const keys = uniqueSiblingKeys(
      [{ id: "same" }, { id: "same" }, { id: "other" }, { id: "same" }],
      (item) => `message:${item.id}`,
    );

    expect(keys).toEqual(["message:same", "message:same~1", "message:other", "message:same~2"]);
  });

  it("keeps duplicate chat message ids unique among siblings", () => {
    const keys = chatMessageKeys([
      { id: "u1" },
      { id: "u1" },
      { id: "u2" },
    ]);

    expect(new Set(keys).size).toBe(keys.length);
    expect(keys[0]).toBe("message:u1");
    expect(keys[1]).toBe("message:u1~1");
  });

  it("keeps duplicate tool call ids unique among activity siblings", () => {
    const keys = chatPartKeys([
      { type: "tool-wiki_search", toolCallId: "call_1", state: "input-available", input: {} },
      { type: "tool-wiki_search", toolCallId: "call_1", state: "output-available", input: {}, output: {} },
      { type: "text", text: "done" },
    ]);

    expect(new Set(keys).size).toBe(keys.length);
    expect(keys[0]).toBe("part:tool:call_1");
    expect(keys[1]).toBe("part:tool:call_1~1");
  });

  it("keeps duplicate ask option ids unique among siblings", () => {
    const keys = askOptionKeys([
      { id: "yes", label: "Yes" },
      { id: "yes", label: "Yes again" },
      { label: "Custom" },
      { label: "Custom" },
    ]);

    expect(new Set(keys).size).toBe(keys.length);
    expect(keys).toEqual([
      "ask-option:id:yes",
      "ask-option:id:yes~1",
      "ask-option:label:Custom",
      "ask-option:label:Custom~1",
    ]);
  });
});
