#!/usr/bin/env node
// Lightweight eval harness for the wiki-mode agent. It runs the real agent loop
// against deterministic local wiki tool fixtures, then scores the transcript
// with deterministic checks instead of a judge model.

import process from "node:process";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const DEFAULT_MODEL = "deepseek/deepseek-v4-pro";
const DEFAULT_BASE_URL = "https://openrouter.ai/api/v1";
const MAX_ROUNDS = 8;

const CASES = [
  {
    id: "recipe-card",
    prompt: "安山机壳怎么做？",
    checks: {
      requiredToolCalls: ["wiki_search"],
      requiredRecipeResultIds: ["create:andesite_casing"],
      forbiddenVisiblePatterns: ["doc:recipe-andesite-casing", "chunk:recipe-andesite-casing"],
    },
  },
  {
    id: "removed-recipe",
    prompt: "这个包里安山合金怎么做？",
    checks: {
      requiredToolCalls: ["wiki_search"],
      forbiddenRecipeCards: true,
      requiredAnyText: [["移除", "删除", "removed", "remove"]],
      forbiddenVisiblePatterns: ["doc:override-andesite-alloy", "chunk:override-andesite-alloy"],
    },
  },
  {
    id: "quest-search",
    prompt: "Create 开局下一步任务要我做什么？",
    checks: {
      requiredToolCalls: ["wiki_search"],
      requiredText: ["Crushing Wheel"],
      forbiddenVisiblePatterns: ["doc:quest-create-start", "chunk:quest-create-start"],
    },
  },
  {
    id: "no-local-answer",
    prompt: "这个包里怎么召唤末影龙二阶段？",
    checks: {
      requiredToolCalls: ["wiki_search"],
      forbiddenRecipeCards: true,
      forbiddenVisiblePatterns: ["doc:", "chunk:"],
    },
  },
];

function parseArgs(argv) {
  const args = {
    model:
      process.env.WIKI_AGENT_EVAL_MODEL ||
      process.env.MC_AGENT_OPENROUTER_MODEL ||
      process.env.OPENROUTER_MODEL ||
      DEFAULT_MODEL,
    baseUrl: process.env.OPENROUTER_BASE_URL || DEFAULT_BASE_URL,
    apiKey: process.env.OPENROUTER_API_KEY || "",
    json: false,
    listCases: false,
    caseIds: [],
  };

  for (let i = 2; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--json") args.json = true;
    else if (arg === "--list-cases") args.listCases = true;
    else if (arg === "--model") args.model = requireValue(argv, ++i, "--model");
    else if (arg === "--base-url") args.baseUrl = requireValue(argv, ++i, "--base-url");
    else if (arg === "--case") {
      args.caseIds.push(...requireValue(argv, ++i, "--case").split(",").map((id) => id.trim()));
    } else if (arg === "--help" || arg === "-h") {
      printHelp();
      process.exit(0);
    } else {
      throw new Error(`unknown argument: ${arg}`);
    }
  }
  args.caseIds = args.caseIds.filter(Boolean);
  return args;
}

function requireValue(argv, index, flag) {
  const value = argv[index];
  if (!value || value.startsWith("--")) throw new Error(`${flag} requires a value`);
  return value;
}

function printHelp() {
  process.stdout.write(`Usage:
  OPENROUTER_API_KEY=... npm run eval:wiki --workspace @kobemc/agent-core -- [--model MODEL] [--case ID[,ID]] [--json]
  node scripts/wiki-agent-eval.mjs --list-cases [--json]

Env:
  OPENROUTER_API_KEY          required unless --list-cases is used
  WIKI_AGENT_EVAL_MODEL       candidate model override
  OPENROUTER_BASE_URL         OpenAI-compatible base URL override
`);
}

async function main() {
  let args;
  try {
    args = parseArgs(process.argv);
  } catch (error) {
    process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
    printHelp();
    process.exit(2);
  }

  const selected = selectCases(args.caseIds);
  if (args.listCases) {
    const listed = selected.map(({ id, prompt, checks }) => ({ id, prompt, checks }));
    process.stdout.write(args.json ? `${JSON.stringify(listed, null, 2)}\n` : formatCaseList(listed));
    return;
  }

  if (!args.apiKey) {
    process.stderr.write("OPENROUTER_API_KEY is required to run wiki agent evals.\n");
    process.exit(2);
  }

  ensureTypescriptLoader();

  const [{ createModpackAgent }, { evaluateWikiEvalTranscript }] = await Promise.all([
    import(new URL("../packages/agent-core/src/index.ts", import.meta.url).href),
    import(new URL("../packages/agent-core/src/eval/wiki-eval.ts", import.meta.url).href),
  ]);

  const results = [];
  for (const testCase of selected) {
    const transcript = await runCase(createModpackAgent, args, testCase);
    const verdict = evaluateWikiEvalTranscript(testCase, transcript);
    results.push({ id: testCase.id, promptVersion: transcript.promptVersion, verdict, transcript });
    if (!args.json) printCaseResult(testCase.id, verdict, transcript);
  }

  if (args.json) {
    process.stdout.write(`${JSON.stringify(results, null, 2)}\n`);
  } else {
    const passed = results.filter((item) => item.verdict.passed).length;
    process.stdout.write(`\n${passed}/${results.length} wiki eval cases passed\n`);
  }

  if (results.some((item) => !item.verdict.passed)) process.exit(1);
}

function ensureTypescriptLoader() {
  if (process.env.WIKI_AGENT_EVAL_TS_LOADER === "1") return;
  try {
    import.meta.resolve("tsx");
  } catch {
    process.stderr.write(
      "wiki-agent-eval requires package dependencies for real model runs. Run `npm install` first so `tsx` and agent-core deps are available.\n",
    );
    process.exit(2);
  }
  const child = spawnSync(
    process.execPath,
    ["--import", "tsx", fileURLToPath(import.meta.url), ...process.argv.slice(2)],
    {
      stdio: "inherit",
      env: { ...process.env, WIKI_AGENT_EVAL_TS_LOADER: "1" },
    },
  );
  process.exit(child.status ?? 1);
}

function selectCases(caseIds) {
  if (!caseIds.length) return CASES;
  const wanted = new Set(caseIds);
  const selected = CASES.filter((testCase) => wanted.has(testCase.id));
  const found = new Set(selected.map((testCase) => testCase.id));
  const missing = [...wanted].filter((id) => !found.has(id));
  if (missing.length) throw new Error(`unknown case id(s): ${missing.join(", ")}`);
  return selected;
}

function formatCaseList(cases) {
  return cases.map((testCase) => `${testCase.id}\t${testCase.prompt}\n`).join("");
}

async function runCase(createModpackAgent, args, testCase) {
  const agent = createModpackAgent(
    { apiKey: args.apiKey, model: args.model, baseUrl: args.baseUrl },
    { mode: "wiki" },
  );
  const history = [userMessage(testCase.prompt)];
  const toolCalls = [];
  const errors = [];
  let promptVersion;

  for (let round = 0; round < MAX_ROUNDS; round++) {
    const result = await agent.run(history, () => {});
    promptVersion = result.promptVersion ?? promptVersion;
    history.splice(0, history.length, ...result.messages);
    if (result.error) errors.push(result.error);

    const assistant = history.at(-1);
    const pending = pendingToolParts(assistant);
    if (!pending.length) {
      return { finalText: textOf(assistant), toolCalls, errors, promptVersion };
    }

    for (const part of pending) {
      const name = part.type.replace(/^tool-/, "");
      const input = part.input ?? {};
      let output;
      try {
        output = executeFixtureTool(testCase.id, name, input);
      } catch (error) {
        output = { error: error instanceof Error ? error.message : String(error) };
        errors.push(`${name}: ${output.error}`);
      }
      part.state = "output-available";
      part.output = output;
      toolCalls.push({ name, input, output });
    }
  }

  errors.push(`hit max round limit (${MAX_ROUNDS})`);
  return { finalText: textOf(history.at(-1)), toolCalls, errors, promptVersion };
}

function userMessage(text) {
  return {
    id: `wiki-eval-user-${Date.now()}`,
    role: "user",
    parts: [{ type: "text", text }],
  };
}

function pendingToolParts(message) {
  return (message?.parts ?? []).filter(
    (part) =>
      typeof part.type === "string" &&
      part.type.startsWith("tool-") &&
      part.state === "input-available",
  );
}

function textOf(message) {
  return (message?.parts ?? [])
    .map((part) => (part.type === "text" ? part.text : ""))
    .join("");
}

function executeFixtureTool(caseId, name, input) {
  if (name === "wiki_search") return fixtureSearch(caseId, input);
  if (name === "wiki_open") return fixtureOpen(caseId, input);
  throw new Error(`unexpected tool: ${name}`);
}

function fixtureSearch(caseId, input) {
  const scope = fixtureScope();
  if (caseId === "recipe-card") {
    return { scope, source_count: 1, hits: [recipeHit()] };
  }
  if (caseId === "removed-recipe") {
    const kind = String(input.kind ?? "").toLowerCase();
    const hits = kind === "recipe" ? [staleAndesiteAlloyRecipeHit()] : [andesiteAlloyOverrideHit()];
    return { scope, source_count: 1, hits };
  }
  if (caseId === "quest-search") {
    return { scope, source_count: 1, hits: [questHit()] };
  }
  if (caseId === "no-local-answer") {
    return { scope, source_count: 1, hits: [] };
  }
  throw new Error(`no fixture for case: ${caseId}`);
}

function fixtureOpen(caseId, input) {
  const chunkId = String(input.chunk_id ?? "");
  const hit = [recipeHit(), andesiteAlloyOverrideHit(), staleAndesiteAlloyRecipeHit(), questHit()].find(
    (item) => item.chunk_id === chunkId,
  );
  if (!hit) throw new Error(`fixture chunk not found for ${caseId}: ${chunkId}`);
  return {
    scope: fixtureScope(),
    chunk: {
      chunk_id: hit.chunk_id,
      document_id: hit.document_id,
      title: hit.title,
      source_label: hit.source_label,
      location: hit.location,
      content: hit.snippet,
      kind: hit.kind,
      structured: hit.structured,
    },
  };
}

function fixtureScope() {
  return {
    modpack_id: "wiki-eval-pack",
    instance_id: "wiki-eval-instance",
    corpus_id: "modpack:wiki-eval-pack:instance:wiki-eval-instance",
  };
}

function recipeHit() {
  const structured = {
    kind: "recipe",
    type: "minecraft:crafting_shaped",
    result: { id: "create:andesite_casing", label: "Andesite Casing", count: 1 },
    grid: [
      [
        { id: "#minecraft:planks", label: "#minecraft:planks" },
        { id: "#minecraft:planks", label: "#minecraft:planks" },
        { id: "#minecraft:planks", label: "#minecraft:planks" },
      ],
      [
        { id: "#minecraft:planks", label: "#minecraft:planks" },
        { id: "create:andesite_alloy", label: "Andesite Alloy" },
        { id: "#minecraft:planks", label: "#minecraft:planks" },
      ],
      [
        { id: "#minecraft:planks", label: "#minecraft:planks" },
        { id: "#minecraft:planks", label: "#minecraft:planks" },
        { id: "#minecraft:planks", label: "#minecraft:planks" },
      ],
    ],
    ingredients: {
      P: { kind: "tag", id: "#minecraft:planks", label: "#minecraft:planks" },
      A: { kind: "item", id: "create:andesite_alloy", label: "Andesite Alloy" },
    },
    source: { origin: "local", type: "mod_jar", uri: "fixture", file: "mods/create.jar" },
  };
  return {
    chunk_id: "chunk:recipe-andesite-casing",
    document_id: "doc:recipe-andesite-casing",
    title: "Recipe: Andesite Casing",
    snippet:
      "kind: recipe\nresult: create:andesite_casing\ningredient: create:andesite_alloy\ningredient: #minecraft:planks",
    source_label: "generated:recipe",
    location: "lines 1-4",
    score: 92,
    kind: "recipe",
    structured,
  };
}

function andesiteAlloyOverrideHit() {
  const structured = {
    kind: "recipe_override",
    action: "remove",
    target: { kind: "item", id: "create:andesite_alloy", label: "Andesite Alloy" },
    target_id: "create:andesite_alloy",
    recipe_id: null,
    input: null,
    replacement: null,
    source: { origin: "local", type: "kubejs", uri: "fixture", file: "kubejs/server_scripts/recipes.js" },
  };
  return {
    chunk_id: "chunk:override-andesite-alloy",
    document_id: "doc:override-andesite-alloy",
    title: "KubeJS recipe override: remove create:andesite_alloy",
    snippet: "kind: recipe_override\naction: remove\ntarget: create:andesite_alloy",
    source_label: "generated:recipe-override",
    location: "lines 1-3",
    score: 98,
    kind: "recipe_override",
    structured,
  };
}

function staleAndesiteAlloyRecipeHit() {
  const structured = {
    kind: "recipe",
    type: "minecraft:crafting_shapeless",
    result: { id: "create:andesite_alloy", label: "Andesite Alloy", count: 1 },
    grid: null,
    ingredients: [
      { kind: "item", id: "minecraft:andesite", label: "Andesite" },
      { kind: "tag", id: "#forge:nuggets/iron", label: "#forge:nuggets/iron" },
    ],
    source: { origin: "local", type: "mod_jar", uri: "fixture", file: "mods/create.jar" },
  };
  return {
    chunk_id: "chunk:recipe-andesite-alloy-stale",
    document_id: "doc:recipe-andesite-alloy-stale",
    title: "Recipe: Andesite Alloy",
    snippet: "kind: recipe\nresult: create:andesite_alloy\nsource: mod jar default",
    source_label: "generated:recipe",
    location: "lines 1-3",
    score: 70,
    kind: "recipe",
    structured,
  };
}

function questHit() {
  const structured = {
    kind: "quest",
    title: "Make a Crushing Wheel",
    chapter: "Create Start",
    source: { origin: "local", type: "ftbquests", uri: "config/ftbquests/quests/chapters/create_start.snbt" },
    raw: '{ title: "Make a Crushing Wheel", description: ["Craft Andesite Alloy", "Use Create stress units"] }',
  };
  return {
    chunk_id: "chunk:quest-create-start",
    document_id: "doc:quest-create-start",
    title: "FTB Quest: Make a Crushing Wheel",
    snippet:
      "Quest title: Make a Crushing Wheel\nQuest description: Craft Andesite Alloy\nQuest token: create:crushing_wheel",
    source_label: "generated:ftb-quests",
    location: "lines 1-3",
    score: 88,
    kind: "quest",
    structured,
  };
}

function printCaseResult(id, verdict, transcript) {
  const status = verdict.passed ? "PASS" : "FAIL";
  process.stdout.write(`[${status}] ${id}\n`);
  for (const check of verdict.checks) {
    const mark = check.passed ? "  ok " : "  err";
    process.stdout.write(`${mark} ${check.name}: ${check.detail}\n`);
  }
  if (!verdict.passed) {
    process.stdout.write(`  tools: ${transcript.toolCalls.map((call) => call.name).join(", ") || "none"}\n`);
    process.stdout.write(`  final: ${transcript.finalText.slice(0, 500).replace(/\s+/g, " ")}\n`);
  }
}

main().catch((error) => {
  process.stderr.write(`${error instanceof Error ? error.stack || error.message : String(error)}\n`);
  process.exit(1);
});
