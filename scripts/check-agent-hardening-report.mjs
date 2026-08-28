#!/usr/bin/env node
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const sealed = process.argv.includes("--sealed");
const [rawData, html] = await Promise.all([
  readFile(resolve(root, "docs/agent-hardening-report.json"), "utf8"),
  readFile(resolve(root, "docs/agent-hardening-report.html"), "utf8"),
]);

const data = JSON.parse(rawData);
const errors = [];
const allowedStatuses = new Set(["running", "verified", "rejected", "blocked"]);
const ids = new Set();
const taskIds = new Set();

if (!Array.isArray(data.timeline) || data.timeline.length < 3) {
  errors.push("report must contain an evidence timeline");
}
if (!Array.isArray(data.verification) || data.verification.length < 5) {
  errors.push("report must contain the verification matrix");
}
for (const check of data.verification ?? []) {
  if (!check.command || !check.result) errors.push("verification row is missing command or result");
  if (sealed && check.status !== "pass") errors.push(`verification failed: ${check.command}`);
}

if (!Array.isArray(data.workstreams) || data.workstreams.length < 4) {
  errors.push("report must contain at least four workstreams");
}

for (const stream of data.workstreams ?? []) {
  if (!stream.id || ids.has(stream.id)) errors.push(`duplicate or missing workstream id: ${stream.id}`);
  ids.add(stream.id);
  if (!allowedStatuses.has(stream.status)) errors.push(`${stream.id}: invalid status ${stream.status}`);
  if (!stream.taskId || taskIds.has(stream.taskId)) errors.push(`${stream.id}: duplicate or missing task id`);
  taskIds.add(stream.taskId);
  for (const field of ["rootCause", "before", "after"]) {
    if (typeof stream[field] !== "string" || stream[field].trim().length < 12) {
      errors.push(`${stream.id}: ${field} is not evidence-grade`);
    }
  }
  if (stream.status === "verified") {
    if (!Array.isArray(stream.tests) || stream.tests.length === 0) errors.push(`${stream.id}: verified without tests`);
    if (!/^[0-9a-f]{7,40}$/i.test(stream.commit ?? "")) errors.push(`${stream.id}: verified without commit`);
  }
  if (sealed && stream.status === "running") errors.push(`${stream.id}: still running in sealed report`);
}

if (sealed && data.status !== "verified") errors.push("sealed report status must be verified");
if (!html.includes(data.baselineCommit)) errors.push("generated HTML does not contain baseline commit");
if (html.includes("__REPORT_DATA__")) errors.push("generated HTML still contains template placeholder");
if (/<script\s+[^>]*src=/i.test(html)) errors.push("report must not load external scripts");
if (!html.includes("prefers-reduced-motion")) errors.push("report lacks reduced-motion support");
if (!html.includes("focus-visible")) errors.push("report lacks visible keyboard focus");

if (errors.length) {
  process.stderr.write(`${errors.map((error) => `- ${error}`).join("\n")}\n`);
  process.exitCode = 1;
} else {
  process.stdout.write(`report-check: ${sealed ? "sealed" : "draft"} ok (${data.workstreams.length} workstreams)\n`);
}
