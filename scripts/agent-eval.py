#!/usr/bin/env python3
"""Local eval harness for the modpack agent's intent + requirement extraction.

Runs each test case through `mc agent start` on one or more candidate models,
then asks a judge model (via OpenRouter) to score the result. Judge cost is a
handful of calls per run.

Usage:
    OPENROUTER_API_KEY=... python3 scripts/agent-eval.py
    AGENT_MODELS="deepseek/deepseek-v4-pro,deepseek/deepseek-v4-flash" python3 scripts/agent-eval.py

Env knobs:
    AGENT_MODELS  comma-separated candidate models for the agent (default: pro,flash)
    JUDGE_MODEL   the judge model (default: deepseek/deepseek-v4-flash)
    MC_BIN        path to the mc binary (default: target/debug/mc)
"""
import json
import os
import subprocess
import sys

MC_BIN = os.environ.get("MC_BIN", "target/debug/mc")
AGENT_MODELS = [m.strip() for m in os.environ.get(
    "AGENT_MODELS", "deepseek/deepseek-v4-pro,deepseek/deepseek-v4-flash"
).split(",") if m.strip()]
JUDGE_MODEL = os.environ.get("JUDGE_MODEL", "deepseek/deepseek-v4-flash")
API_KEY = os.environ.get("OPENROUTER_API_KEY", "")

# id, prompt, expected_intent, what a good extraction looks like
TEST_CASES = [
    ("adventure", "帮我做一个 Minecraft 1.20.1 Fabric 的冒险探索整合包,想要更多地牢、新结构和生物群系,再加一点生活质量优化(小地图、背包整理)",
     "build_modpack", "loader=fabric, mc=1.20.1, tags cover adventure/exploration/dungeons/structures/biomes/qol/minimap/inventory"),
    ("tech", "做一个 1.20.1 Fabric 的科技自动化整合包,要机械动力 Create、物流、电力网络和自动化农场",
     "build_modpack", "loader=fabric, mc=1.20.1, tags cover create/automation/logistics/power/farming"),
    ("named-mods", "我要一个 1.19.2 Forge 整合包,必须包含 Create 和 Applied Energistics 2,再配点装饰类 mod",
     "build_modpack", "loader=forge, mc=1.19.2, tags/notes capture create + applied-energistics-2 + decoration"),
    ("non-modpack", "今天北京天气怎么样,适合出门吗?",
     "unknown", "should NOT be build_modpack; this is unrelated to Minecraft modpacks"),
]


def run_agent(model, case_id, prompt):
    sid = f"eval-{case_id}-{model.split('/')[-1]}"
    out = subprocess.run(
        [MC_BIN, "agent", "start", prompt, "--session-id", sid, "--model", model, "--json"],
        capture_output=True, text=True, timeout=180,
    )
    if out.returncode != 0:
        return {"error": (out.stderr or out.stdout).strip()[:300]}
    try:
        snap = json.loads(out.stdout)
    except Exception as e:
        return {"error": f"bad json: {e}; head={out.stdout[:200]}"}
    intent = snap.get("intent") or {}
    r = snap.get("restrictions") or {}
    return {
        "intent_kind": intent.get("kind"),
        "confidence": intent.get("confidence"),
        "loader": r.get("loader"),
        "mc_version": r.get("minecraft_version") or r.get("minecraft_version_requirement"),
        "feature_tags": r.get("feature_tags") or [],
        "notes": r.get("notes"),
    }


def judge(prompt, expected, rubric, result):
    if "error" in result:
        return {"intent_correct": False, "requirements_score": 0, "issues": "agent error: " + result["error"]}
    sys_msg = ("You are a strict evaluator for a Minecraft modpack-building agent's "
               "intent classification and requirement extraction. Be terse and honest.")
    user_msg = f"""User request: {prompt}
Expected intent: {expected}
Good extraction looks like: {rubric}

Agent produced:
- intent: {result.get('intent_kind')} (confidence {result.get('confidence')})
- loader: {result.get('loader')}
- minecraft_version: {result.get('mc_version')}
- feature_tags: {result.get('feature_tags')}
- notes: {result.get('notes')}

Evaluate and respond with ONLY a JSON object, no prose:
{{"intent_correct": true|false, "requirements_score": 0-5, "issues": "one short line; '-' if none"}}
requirements_score rubric: accuracy of loader/version (from the request), completeness of feature coverage,
and whether tags are concise, English, and search-friendly. For a non-modpack request, score requirements 0 and judge only intent_correct."""
    body = json.dumps({
        "model": JUDGE_MODEL,
        "messages": [{"role": "system", "content": sys_msg}, {"role": "user", "content": user_msg}],
        "temperature": 0,
        "max_tokens": 1200,
    })
    out = subprocess.run(
        ["curl", "-s", "--max-time", "90", "https://openrouter.ai/api/v1/chat/completions",
         "-H", f"Authorization: Bearer {API_KEY}", "-H", "Content-Type: application/json", "-d", body],
        capture_output=True, text=True, timeout=100,
    )
    try:
        msg = json.loads(out.stdout)["choices"][0]["message"]
        # reasoning models can return content=null with the answer in `reasoning`
        content = msg.get("content") or msg.get("reasoning") or ""
        start, end = content.find("{"), content.rfind("}")
        return json.loads(content[start:end + 1])
    except Exception as e:
        return {"intent_correct": None, "requirements_score": None, "issues": f"judge parse fail: {e}; raw={out.stdout[:160]}"}


def main():
    if not API_KEY:
        print("OPENROUTER_API_KEY not set"); sys.exit(1)
    print(f"judge = {JUDGE_MODEL}   candidates = {AGENT_MODELS}\n")
    rows = []
    for cid, prompt, expected, rubric in TEST_CASES:
        for model in AGENT_MODELS:
            res = run_agent(model, cid, prompt)
            verdict = judge(prompt, expected, rubric, res)
            rows.append((cid, model.split("/")[-1], res, verdict))
            tags = ",".join(res.get("feature_tags", [])[:6]) if "error" not in res else "ERR"
            print(f"[{cid:11} | {model.split('/')[-1]:16}] intent={res.get('intent_kind','?'):13} "
                  f"conf={res.get('confidence','?')}  loader={res.get('loader')} mc={res.get('mc_version')}")
            print(f"    tags: {tags}")
            print(f"    JUDGE: intent_ok={verdict.get('intent_correct')}  req_score={verdict.get('requirements_score')}/5  "
                  f"issues: {verdict.get('issues')}\n")
    # aggregate per model
    print("=== aggregate (req_score avg over modpack cases, intent accuracy over all) ===")
    for model in AGENT_MODELS:
        m = model.split("/")[-1]
        mr = [v for (c, mm, r, v) in rows if mm == m]
        intent_ok = sum(1 for v in mr if v.get("intent_correct") is True)
        scores = [v.get("requirements_score") for v in mr if isinstance(v.get("requirements_score"), int) and v.get("requirements_score") > 0]
        avg = sum(scores) / len(scores) if scores else 0
        print(f"  {m:16}  intent {intent_ok}/{len(mr)}  req_score avg {avg:.2f}/5")


if __name__ == "__main__":
    main()
