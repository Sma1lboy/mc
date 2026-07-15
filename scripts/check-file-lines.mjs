import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { extname } from "node:path";

const MAX_LINES = 700;

const CHECKED_EXTENSIONS = new Set([
  ".cjs",
  ".css",
  ".html",
  ".js",
  ".jsx",
  ".mjs",
  ".py",
  ".rs",
  ".sh",
  ".ts",
  ".tsx",
]);

const EXCLUDED_FILES = new Set([
  "desktop/design-previews/glass-home.html",
  "desktop/src/ipc/bindings.ts",
]);

const EXCLUDED_DIRECTORY_PREFIXES = [
  "build/",
  "dist/",
  "node_modules/",
  "ref/",
  "target/",
];

function isExcluded(file) {
  if (EXCLUDED_FILES.has(file)) return true;
  return EXCLUDED_DIRECTORY_PREFIXES.some((prefix) => file.startsWith(prefix));
}

function countLines(contents) {
  if (contents.length === 0) return 0;

  let lines = 0;
  for (const byte of contents) {
    if (byte === 0x0a) lines += 1;
  }

  if (contents.at(-1) !== 0x0a) lines += 1;
  return lines;
}

let trackedFiles;
try {
  trackedFiles = execFileSync("git", ["ls-files", "-z"], {
    encoding: "utf8",
  })
    .split("\0")
    .filter(Boolean);
} catch (error) {
  console.error("Failed to list tracked files with git ls-files.");
  if (error instanceof Error && error.message) console.error(error.message);
  process.exit(1);
}

const violations = [];
let checkedFileCount = 0;

for (const file of trackedFiles) {
  if (!CHECKED_EXTENSIONS.has(extname(file)) || isExcluded(file)) continue;

  let contents;
  try {
    contents = readFileSync(file);
  } catch (error) {
    // git ls-files can still report a tracked file deleted in the working tree.
    if (error && typeof error === "object" && error.code === "ENOENT") continue;
    throw error;
  }

  checkedFileCount += 1;
  const lines = countLines(contents);
  if (lines > MAX_LINES) violations.push({ file, lines });
}

if (violations.length > 0) {
  console.error(`Files exceeding the ${MAX_LINES}-line limit:`);
  for (const { file, lines } of violations) {
    console.error(`  ${file}: ${lines} lines`);
  }
  console.error(`Found ${violations.length} violation(s).`);
  process.exit(1);
}

console.log(
  `Checked ${checkedFileCount} maintained source/test file(s); all are at most ${MAX_LINES} lines.`,
);
