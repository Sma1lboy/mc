#!/usr/bin/env node

import {
  buildTools,
  promptForMode,
  toolSchemasForMode,
} from "../packages/agent-core/src/index.ts";

const asJson = process.argv.includes("--json");

const fixtures = [
  {
    id: "local-recipe-question",
    request: "这个实例里的安山合金配方是什么？",
    evaluate() {
      const tools = names("instance");
      const prompt = promptForMode("instance");
      return [
        check(tools.has("wiki_search") && tools.has("wiki_open"), "wiki tools are available"),
        check(prompt.includes("# Wiki flow"), "instance prompt defines the local wiki flow"),
        check(!tools.has("build_modpack"), "instance profile cannot build a modpack"),
      ];
    },
  },
  {
    id: "crash-diagnosis",
    request: "游戏启动就崩了，帮我看看。",
    evaluate() {
      const tools = names("instance");
      const prompt = promptForMode("instance");
      const schema = toolSchemasForMode("instance").diagnose_instance;
      return [
        check(tools.has("diagnose_instance"), "diagnose_instance is available"),
        check(prompt.includes("call `diagnose_instance` first"), "prompt routes symptoms to diagnosis"),
        check(schema.safeParse({ include_log_tail: true }).success, "model may request a log tail"),
        check(
          !schema.safeParse({ root: "/tmp", instance_id: "pack" }).success,
          "diagnosis rejects host-owned root and instance id",
        ),
      ];
    },
  },
  {
    id: "confirmed-instance-change",
    request: "把内存调高并停用冲突模组。",
    evaluate() {
      const tools = names("instance");
      const schemas = toolSchemasForMode("instance");
      const validPlan = {
        summary: "Increase memory and disable the conflicting mod",
        operations: [
          { type: "set_memory", memory_mb: 4096 },
          { type: "set_mod_enabled", file_name: "conflict.jar", enabled: false },
        ],
      };
      return [
        check(tools.has("show_instance_changes"), "confirmation tool is available"),
        check(schemas.show_instance_changes.safeParse(validPlan).success, "change plan is valid"),
        check(
          !schemas.show_instance_changes.safeParse({ ...validPlan, instance_id: "pack" }).success,
          "change plan rejects host-owned instance id",
        ),
      ];
    },
  },
  {
    id: "instance-mod-search",
    request: "给这个实例找一个兼容的性能优化模组。",
    evaluate() {
      const tools = names("instance");
      const schemas = toolSchemasForMode("instance");
      return [
        check(
          ["search_mods", "mod_get_detail", "resolve_mods"].every((tool) => tools.has(tool)),
          "provider search and resolution tools are available",
        ),
        check(schemas.search_mods.safeParse({ query: "performance" }).success, "query-only search is valid"),
        check(
          !schemas.search_mods.safeParse({
            query: "performance",
            mc_version: "1.20.1",
            loader: "fabric",
          }).success,
          "instance search rejects model-supplied target fields",
        ),
      ];
    },
  },
  {
    id: "custom-build-validation",
    request: "按这个最终清单构建整合包。",
    evaluate() {
      const tools = names("build");
      const prompt = promptForMode("build");
      return [
        check(tools.has("validate_modpack_plan"), "build validation is available"),
        check(tools.has("build_modpack"), "build tool is available"),
        check(
          prompt.indexOf("validate_modpack_plan") < prompt.indexOf("build_modpack"),
          "prompt introduces validation before build",
        ),
        check(!tools.has("diagnose_instance"), "build profile cannot diagnose an instance"),
      ];
    },
  },
];

const cases = fixtures.map((fixture) => {
  try {
    const checks = fixture.evaluate();
    return {
      id: fixture.id,
      request: fixture.request,
      passed: checks.every((item) => item.passed),
      checks,
    };
  } catch (error) {
    return {
      id: fixture.id,
      request: fixture.request,
      passed: false,
      checks: [],
      error: String(error),
    };
  }
});

const promptsAvoidActivation = ["build", "instance"].every(
  (mode) => !promptForMode(mode).includes("activate_tools"),
);
cases.push({
  id: "no-progressive-tool-activation",
  request: "All relevant tools are available immediately in each explicit entry.",
  passed: promptsAvoidActivation,
  checks: [check(promptsAvoidActivation, "prompts do not introduce activate_tools")],
});

const result = {
  suite: "instance-agent-routing",
  passed: cases.every((item) => item.passed),
  passed_cases: cases.filter((item) => item.passed).length,
  total_cases: cases.length,
  cases,
};

if (asJson) {
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
} else {
  for (const item of cases) {
    process.stdout.write(`${item.passed ? "PASS" : "FAIL"} ${item.id}\n`);
    for (const assertion of item.checks) {
      process.stdout.write(`  ${assertion.passed ? "ok" : "not ok"} - ${assertion.message}\n`);
    }
    if (item.error) process.stdout.write(`  error - ${item.error}\n`);
  }
  process.stdout.write(`\n${result.passed_cases}/${result.total_cases} cases passed\n`);
}

process.exitCode = result.passed ? 0 : 1;

function names(mode) {
  return new Set(Object.keys(buildTools(mode)));
}

function check(passed, message) {
  return { passed: Boolean(passed), message };
}
