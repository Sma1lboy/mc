#!/usr/bin/env node
import { readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const dataPath = resolve(root, "docs/agent-hardening-report.json");
const templatePath = resolve(root, "docs/agent-hardening-report.template.html");
const outputPath = resolve(root, "docs/agent-hardening-report.html");

const [rawData, template] = await Promise.all([
  readFile(dataPath, "utf8"),
  readFile(templatePath, "utf8"),
]);

const data = JSON.parse(rawData);
const serialized = JSON.stringify(data).replaceAll("<", "\\u003c");
if (!template.includes("__REPORT_DATA__")) {
  throw new Error("report template is missing __REPORT_DATA__");
}

await writeFile(outputPath, template.replace("__REPORT_DATA__", serialized));
process.stdout.write(`${outputPath}\n`);
