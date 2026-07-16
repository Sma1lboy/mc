import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

import {
  evaluateWikiEvalTranscript,
  extractRecipeCards,
  type WikiEvalCase,
} from "../src/eval/wiki-eval";

describe("wiki eval helpers", () => {
  it("extracts fenced recipe_card JSON blocks", () => {
    const cards = extractRecipeCards(`本地配方如下。

\`\`\`recipe_card
{
  "version": 1,
  "result": { "id": "create:andesite_casing", "label": "Andesite Casing", "count": 1 },
  "source_document_ids": ["doc:recipe-andesite-casing"]
}
\`\`\`
`);

    expect(cards).toHaveLength(1);
    expect(cards[0].result.id).toBe("create:andesite_casing");
  });

  it("evaluates transcript checks deterministically", () => {
    const testCase: WikiEvalCase = {
      id: "recipe-card",
      prompt: "安山机壳怎么做？",
      checks: {
        requiredToolCalls: ["wiki_search"],
        requiredRecipeResultIds: ["create:andesite_casing"],
        forbiddenVisiblePatterns: ["doc:recipe-andesite-casing", "chunk:recipe-andesite-casing"],
        requiredText: ["本地"],
      },
    };

    const verdict = evaluateWikiEvalTranscript(testCase, {
      finalText: `本地索引里有这个配方。

\`\`\`recipe_card
{
  "version": 1,
  "result": { "id": "create:andesite_casing", "label": "Andesite Casing", "count": 1 },
  "source_document_ids": ["doc:recipe-andesite-casing"]
}
\`\`\`
`,
      toolCalls: [
        {
          name: "wiki_search",
          input: { query: "andesite casing" },
          output: { hits: [] },
        },
      ],
    });

    expect(verdict.passed).toBe(true);
    expect(verdict.checks.every((check) => check.passed)).toBe(true);
  });

  it("fails when none of a required text alternative group is present", () => {
    const verdict = evaluateWikiEvalTranscript(
      {
        id: "removed-recipe",
        prompt: "这个包里安山合金怎么做？",
        checks: {
          requiredToolCalls: ["wiki_search"],
          requiredAnyText: [["移除", "删除", "removed"]],
        },
      },
      {
        finalText: "本地索引没有暴露这个配方。",
        toolCalls: [{ name: "wiki_search", input: {}, output: { hits: [] } }],
      },
    );

    expect(verdict.passed).toBe(false);
    expect(verdict.checks.find((check) => check.name.startsWith("contains_any_"))?.passed).toBe(
      false,
    );
  });

  it("lists built-in wiki eval cases from the CLI without an API key", () => {
    const script = fileURLToPath(new URL("../../../scripts/wiki-agent-eval.mjs", import.meta.url));
    const out = spawnSync(process.execPath, [script, "--list-cases", "--json"], {
      encoding: "utf8",
    });

    expect(out.status).toBe(0);
    const cases = JSON.parse(out.stdout) as Array<{ id: string }>;
    expect(cases.map((item) => item.id)).toContain("recipe-card");
    expect(cases.map((item) => item.id)).toContain("removed-recipe");
    expect(cases.map((item) => item.id)).toContain("prompt-injection");
  });
});
