// Micro-benchmark: JSON.stringify + JSON.parse of a ~100KB SearchModsOutput-shaped
// payload, ×1000. This approximates ONLY the webview-side serialization share of a
// Tauri IPC call (the JS heap ⇄ JSON cost). It does NOT measure the native Tauri
// transport itself: crossing the webview↔Rust boundary adds ~0.5–2 ms per call per
// published Tauri/IPC measurements (estimate — cited, not measured here).
//
// Relevance: the Rust brain's tool results cross this IPC boundary (webview asks
// Tauri to run a tool); the TS brain running in-webview would call the SAME Tauri
// commands for tools, so this cost is largely COMMON to both. Run: node serde-payload.mjs

const N = 1000;

// One SearchMods hit, matching the fields the TS core/tools surface.
function hit(i) {
  return {
    provider: "modrinth",
    project_id: `proj-${i}-aaaaaaaa`,
    slug: `some-mod-${i}`,
    title: `Some Mod ${i}`,
    description:
      "A representative mod description with enough prose to be realistic. " +
      "It mentions performance, compatibility, and a few feature categories. ".repeat(2),
    author: `author_${i}`,
    downloads: 1234567 + i,
    icon_url: `https://cdn.modrinth.com/data/proj-${i}/icon.png`,
    gallery_url: null,
    categories: ["optimization", "utility", "library"],
    client_side: "required",
    server_side: "optional",
  };
}

// Size to ~100KB stringified.
function buildPayload() {
  const mods = [];
  for (let i = 0; mods.length < 4096; i++) {
    mods.push(hit(i));
    if (JSON.stringify({ mods }).length >= 100 * 1024) break;
  }
  return { mods };
}

const payload = buildPayload();
const asString = JSON.stringify(payload);
const sizeKB = asString.length / 1024;

// Self-check: round-trip is lossless and the payload is ~100KB.
{
  const rt = JSON.parse(JSON.stringify(payload));
  if (JSON.stringify(rt) !== asString) throw new Error("serde round-trip not lossless");
  if (sizeKB < 90 || sizeKB > 130) throw new Error(`payload size off target: ${sizeKB.toFixed(1)} KB`);
}

const absMs = () => performance.timeOrigin + performance.now();

// Warmup.
for (let i = 0; i < 100; i++) JSON.parse(JSON.stringify(payload));

const t0 = absMs();
let sink = 0;
for (let i = 0; i < N; i++) {
  const s = JSON.stringify(payload);
  const o = JSON.parse(s);
  sink += o.mods.length; // defeat DCE
}
const t1 = absMs();

const totalMs = t1 - t0;
const perOpMs = totalMs / N;
const mbPerSec = (sizeKB / 1024) / (perOpMs / 1000);

console.log(
  JSON.stringify(
    {
      payloadKB: Number(sizeKB.toFixed(1)),
      iterations: N,
      totalMs: Number(totalMs.toFixed(2)),
      perOpMs_stringifyPlusParse: Number(perOpMs.toFixed(4)),
      throughputMBps: Number(mbPerSec.toFixed(1)),
      _sink: sink,
    },
    null,
    2,
  ),
);
