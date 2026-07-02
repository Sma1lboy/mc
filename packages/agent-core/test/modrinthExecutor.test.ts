import { describe, it, expect } from "vitest";

import { modrinthExecutor, BUILD_UNSUPPORTED_MESSAGE } from "../src/executors/index";

// --- tiny helpers: mocked Modrinth HTTP + a hand-built stored (.mrpack) zip ---

const jsonRes = (obj: unknown) => new Response(JSON.stringify(obj), { status: 200 });
const bytesRes = (buf: Buffer) => new Response(buf, { status: 200 });

/** Build a minimal zip with STORED (method 0) entries. crc left 0 (reader ignores it). */
function makeStoredZip(entries: { name: string; data: Buffer }[]): Buffer {
  const parts: Buffer[] = [];
  const central: Buffer[] = [];
  let offset = 0;
  for (const { name, data } of entries) {
    const nameBuf = Buffer.from(name, "utf8");
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(0, 8); // method 0 = stored
    local.writeUInt32LE(data.length, 18); // comp size
    local.writeUInt32LE(data.length, 22); // uncomp size
    local.writeUInt16LE(nameBuf.length, 26);
    parts.push(local, nameBuf, data);

    const cd = Buffer.alloc(46);
    cd.writeUInt32LE(0x02014b50, 0);
    cd.writeUInt16LE(0, 10); // method 0
    cd.writeUInt32LE(data.length, 20);
    cd.writeUInt32LE(data.length, 24);
    cd.writeUInt16LE(nameBuf.length, 28);
    cd.writeUInt32LE(offset, 42); // local header offset
    central.push(cd, nameBuf);
    offset += 30 + nameBuf.length + data.length;
  }
  const centralBuf = Buffer.concat(central);
  const eocd = Buffer.alloc(22);
  eocd.writeUInt32LE(0x06054b50, 0);
  eocd.writeUInt16LE(entries.length, 8);
  eocd.writeUInt16LE(entries.length, 10);
  eocd.writeUInt32LE(centralBuf.length, 12);
  eocd.writeUInt32LE(offset, 16); // central-directory start
  return Buffer.concat([...parts, centralBuf, eocd]);
}

/** Route a fake fetch by URL pathname (or full url for the CDN archive). */
function fakeFetch(handler: (u: URL, raw: string) => Response | undefined) {
  return async (input: unknown): Promise<Response> => {
    const raw = String(input);
    const res = handler(new URL(raw), raw);
    if (!res) throw new Error(`unexpected fetch: ${raw}`);
    return res;
  };
}

describe("modrinthExecutor (read-only, mocked HTTP)", () => {
  it("(d) search_base_modpacks output shape matches the wire contract", async () => {
    const exec = modrinthExecutor({
      fetch: fakeFetch((u) =>
        u.pathname.endsWith("/search")
          ? jsonRes({
              hits: [
                {
                  project_id: "abc",
                  slug: "cool-pack",
                  title: "Cool Pack",
                  description: "a pack",
                  author: "someone",
                  downloads: 42,
                },
              ],
            })
          : undefined,
      ),
    });
    const out = (await exec.search_base_modpacks({ query: "tech", mc_version: "1.20.1", loader: "fabric" })) as {
      candidates: Record<string, unknown>[];
    };
    expect(out.candidates).toHaveLength(1);
    expect(out.candidates[0]).toEqual({
      provider: "modrinth",
      project_id: "abc",
      slug: "cool-pack",
      title: "Cool Pack",
      author: "someone",
      downloads: 42,
      description: "a pack",
    });
  });

  it("(d) search_mods and mod_get_detail shapes match", async () => {
    const exec = modrinthExecutor({
      fetch: fakeFetch((u) => {
        if (u.pathname.endsWith("/search"))
          return jsonRes({
            hits: [
              { project_id: "sod", slug: "sodium", title: "Sodium", description: "fast", author: "jelly", downloads: 9 },
            ],
          });
        if (u.pathname === "/v2/project/sod")
          return jsonRes({ slug: "sodium", title: "Sodium", description: "fast", downloads: 9, categories: ["performance"] });
        if (u.pathname === "/v2/project/sod/version")
          return jsonRes([
            {
              id: "v1",
              name: "Sodium 0.5",
              version_number: "0.5",
              game_versions: ["1.20.1"],
              loaders: ["fabric"],
              files: [{ url: "u", filename: "sodium.jar", primary: true, hashes: { sha1: "a", sha512: "b" }, size: 10 }],
              dependencies: [],
            },
          ]);
        return undefined;
      }),
    });

    const mods = (await exec.search_mods({ query: "sodium", mc_version: "1.20.1", loader: "fabric" })) as {
      mods: Record<string, unknown>[];
    };
    expect(mods.mods[0]).toEqual({
      provider: "modrinth",
      project_id: "sod",
      slug: "sodium",
      title: "Sodium",
      downloads: 9,
      description: "fast",
    });

    const detail = (await exec.mod_get_detail({ project_id: "sod", minecraft_version: "1.20.1", loader: "fabric" })) as {
      project: Record<string, unknown>;
      versions: Record<string, unknown>[];
    };
    expect(detail.project).toEqual({
      title: "Sodium",
      slug: "sodium",
      description: "fast",
      categories: ["performance"],
      downloads: 9,
    });
    expect(detail.versions[0]).toEqual({
      version_id: "v1",
      version_number: "0.5",
      game_versions: ["1.20.1"],
      loaders: ["fabric"],
      dependencies_count: 0,
      filename: "sodium.jar",
    });
  });

  it("(d) resolve_mods walks required deps and shapes resolved refs", async () => {
    const version = (id: string, deps: { project_id: string; dependency_type: string }[]) => ({
      id: `${id}-v1`,
      name: id,
      version_number: "1.0",
      game_versions: ["1.20.1"],
      loaders: ["fabric"],
      files: [{ url: `https://x/${id}.jar`, filename: `${id}.jar`, primary: true, hashes: { sha1: "s1", sha512: "s5" }, size: 3 }],
      dependencies: deps,
    });
    const exec = modrinthExecutor({
      fetch: fakeFetch((u) => {
        if (u.pathname === "/v2/project/root/version")
          return jsonRes([version("root", [{ project_id: "lib", dependency_type: "required" }])]);
        if (u.pathname === "/v2/project/lib/version") return jsonRes([version("lib", [])]);
        return undefined;
      }),
    });
    const out = (await exec.resolve_mods({ project_ids: ["root"], mc_version: "1.20.1", loader: "fabric" })) as {
      resolved: { project_id: string; version_id: string; url: string }[];
      unresolved: unknown[];
      conflicts: unknown[];
    };
    const ids = out.resolved.map((r) => r.project_id).sort();
    expect(ids).toEqual(["lib", "root"]);
    expect(out.resolved.find((r) => r.project_id === "lib")).toMatchObject({
      provider: "modrinth",
      version_id: "lib-v1",
      url: "https://x/lib.jar",
      filename: "lib.jar",
      sha1: "s1",
      sha512: "s5",
      size: 3,
    });
    expect(out.unresolved).toEqual([]);
    expect(out.conflicts).toEqual([]);
  });

  it("(d) resolve_mods marks a non-modrinth ref unresolved", async () => {
    const exec = modrinthExecutor({ fetch: fakeFetch(() => undefined) });
    const out = (await exec.resolve_mods({
      project_ids: ["curseforge:12345"],
      mc_version: "1.20.1",
      loader: "fabric",
    })) as { unresolved: Record<string, unknown>[]; resolved: unknown[] };
    expect(out.resolved).toEqual([]);
    expect(out.unresolved).toEqual([{ provider: "curseforge", project_id: "12345" }]);
  });

  it("(d) inspect_base_modpack unzips the .mrpack and reports mods + covered features", async () => {
    const archiveUrl = "https://cdn.modrinth.com/data/PACK/versions/vp/pack.mrpack";
    const index = {
      files: [
        {
          path: "mods/sodium.jar",
          downloads: ["https://cdn.modrinth.com/data/AANobbMI/versions/x/sodium.jar"],
          env: { client: "required", server: "optional" },
        },
        // client-unsupported → skipped (like the Rust parser).
        {
          path: "mods/server-only.jar",
          downloads: ["https://cdn.modrinth.com/data/SERVERON/versions/y/server.jar"],
          env: { client: "unsupported", server: "required" },
        },
      ],
    };
    const zip = makeStoredZip([{ name: "modrinth.index.json", data: Buffer.from(JSON.stringify(index)) }]);
    const exec = modrinthExecutor({
      fetch: fakeFetch((u, raw) => {
        if (raw === archiveUrl) return bytesRes(zip);
        if (u.pathname === "/v2/project/PACK/version")
          return jsonRes([
            {
              id: "vp",
              name: "Cool Pack",
              version_number: "1.0",
              game_versions: ["1.20.1"],
              loaders: ["fabric"],
              files: [{ url: archiveUrl, filename: "pack.mrpack", primary: true, hashes: {} }],
              dependencies: [],
            },
          ]);
        if (u.pathname === "/v2/projects")
          return jsonRes([
            { slug: "sodium", title: "Sodium", description: "fast", downloads: 1, categories: ["performance"] },
          ]);
        return undefined;
      }),
    });
    const out = (await exec.inspect_base_modpack({ project_id: "PACK", mc_version: "1.20.1", loader: "fabric" })) as {
      title: string;
      mc_version: string;
      loader: string;
      mod_count: number;
      mods: { title: string; categories: string[] }[];
      covered_features: string[];
    };
    expect(out.title).toBe("Cool Pack");
    expect(out.mc_version).toBe("1.20.1");
    expect(out.loader).toBe("fabric");
    expect(out.mod_count).toBe(1); // server-only file skipped
    expect(out.mods).toEqual([{ title: "Sodium", categories: ["performance"] }]);
    expect(out.covered_features).toEqual(["performance"]);
  });

  it("(e) build_modpack returns the structured unsupported error", async () => {
    const exec = modrinthExecutor({ fetch: fakeFetch(() => undefined) });
    const out = (await exec.build_modpack({
      target: { mc_version: "1.20.1", loader: "fabric" },
      extra_mods: [],
      output_filename: "x.mrpack",
    })) as { status: string; error: string };
    expect(out.status).toBe("unsupported");
    expect(out.error).toBe(BUILD_UNSUPPORTED_MESSAGE);
    expect(out.error).toContain("use the desktop app");
  });
});
