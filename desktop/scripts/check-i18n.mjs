// i18n coverage check: every t("key") used in src must exist in the zh dictionary
// (zh is the source of truth; en falls back to zh, so en gaps are reported separately).
// Run from desktop/:  node scripts/check-i18n.mjs
import { build } from "esbuild";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

const res = await build({
  entryPoints: ["src/locales/index.ts"],
  bundle: true,
  format: "esm",
  write: false,
  platform: "neutral",
});
const mod = await import(
  "data:text/javascript;base64," + Buffer.from(res.outputFiles[0].text).toString("base64")
);
const { dictionaries } = mod;

const flatten = (obj, prefix = "", out = {}) => {
  for (const [k, v] of Object.entries(obj)) {
    const key = prefix ? `${prefix}.${k}` : k;
    if (v && typeof v === "object") flatten(v, key, out);
    else out[key] = v;
  }
  return out;
};
const zh = new Set(Object.keys(flatten(dictionaries.zh)));
const en = new Set(Object.keys(flatten(dictionaries.en)));

const walk = (dir, files = []) => {
  for (const f of readdirSync(dir)) {
    const p = join(dir, f);
    if (statSync(p).isDirectory()) {
      if (f !== "node_modules" && f !== "locales") walk(p, files);
    } else if ((f.endsWith(".ts") || f.endsWith(".tsx")) && f !== "i18n.ts") files.push(p);
  }
  return files;
};

const re = /\bt\(\s*["'`]([^"'`$]+)["'`]/g;
const used = new Map();
for (const f of walk("src")) {
  const txt = readFileSync(f, "utf8");
  let m;
  while ((m = re.exec(txt))) if (!used.has(m[1])) used.set(m[1], f);
}

const missing = [...used].filter(([k]) => !zh.has(k));
const enGap = [...used].filter(([k]) => zh.has(k) && !en.has(k));

console.log(`used t() keys: ${used.size} | zh: ${zh.size} | en: ${en.size}`);
if (missing.length) {
  console.log(`\n✗ MISSING in zh (would show raw key in UI): ${missing.length}`);
  for (const [k, f] of missing) console.log(`   ${k}  <-  ${f}`);
} else {
  console.log("✓ every t() key exists in zh");
}
console.log(`\nen gaps (fall back to zh, ok but untranslated): ${enGap.length}`);
for (const [k] of enGap.slice(0, 30)) console.log(`   ${k}`);
process.exit(missing.length ? 1 : 0);
