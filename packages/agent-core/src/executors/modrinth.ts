// A real, read-only `ToolExecutor` backed by the Modrinth HTTP API (v2).
//
// This is the host backend a server would use to run the brain WITHOUT a
// desktop app: the five non-writing tools hit Modrinth directly and return JSON
// whose field names match the Rust tools (`mc_core::agent::tools`) exactly
// (snake_case), so the same UI/model round-trips through either host. It never
// writes to disk; `build_modpack` returns a structured "unsupported here" error
// (the trust-critical .mrpack writer stays in Rust — see README, "Hosting").
//
// Everything is a straight port of the Rust primitives: search facets
// (`modplatform::modrinth`), the "best compatible version" pick + dependency BFS
// (`modplatform::dependency`), and the base-modlist parse (`agent::build::
// base_modlist`). No new dependencies — global `fetch` + node:zlib for the one
// zip entry `inspect_base_modpack` reads out of a `.mrpack`.

import { inflateRawSync } from "node:zlib";

import type { ToolExecutor } from "../types";

/** Modrinth API v2 root. */
const API_BASE = "https://api.modrinth.com/v2";
/** Modrinth asks for a contactable UA (else it may rate-limit); mirror Rust's. */
const USER_AGENT = "mc-launcher/0.1 (github.com/sma1lboy/mc-launcher)";

// Caps mirrored from the Rust tools so payloads stay bounded identically.
const BASE_SEARCH_TOTAL_CAP = 8;
const MOD_SEARCH_PER_QUERY_CAP = 8;
const MOD_SEARCH_TOTAL_CAP = 12;
const MOD_DETAIL_VERSION_CAP = 10;
const MAX_DEPTH = 32; // dependency-BFS guard (matches dependency::MAX_DEPTH)
/** Hard ceiling on a base `.mrpack` we download to inspect (matches Rust 96 MiB). */
const MAX_BASE_ARCHIVE_BYTES = 96 * 1024 * 1024;

/** Message returned by `build_modpack` here: assembly lives in the desktop host. */
export const BUILD_UNSUPPORTED_MESSAGE =
  "building is not available in this host — use the desktop app";

// --- Minimal Modrinth wire shapes (only the fields the tools consume). --------

interface RawFile {
  url: string;
  filename: string;
  hashes?: { sha1?: string; sha512?: string };
  size?: number;
  primary?: boolean;
}
interface RawDependency {
  project_id?: string | null;
  dependency_type?: string;
}
interface RawVersion {
  id: string;
  name: string;
  version_number: string;
  game_versions?: string[];
  loaders?: string[];
  files?: RawFile[];
  dependencies?: RawDependency[];
}
interface RawSearchHit {
  project_id: string;
  slug: string;
  title: string;
  description: string;
  author: string;
  downloads: number;
}
interface RawProject {
  slug: string;
  title: string;
  description: string;
  downloads: number;
  categories?: string[];
}

// --- HTTP client --------------------------------------------------------------

type FetchLike = typeof fetch;

class ModrinthClient {
  constructor(
    private readonly base: string,
    private readonly fetchImpl: FetchLike,
  ) {}

  private async json<T>(path: string, params: Record<string, string>): Promise<T> {
    const url = new URL(this.base + path);
    for (const [k, v] of Object.entries(params)) url.searchParams.set(k, v);
    const res = await this.fetchImpl(url.toString(), { headers: { "user-agent": USER_AGENT } });
    if (!res.ok) throw new Error(`Modrinth ${path} failed: HTTP ${res.status}`);
    return (await res.json()) as T;
  }

  /** `GET /search` — `project_type` + optional version/loader facets. */
  async search(
    query: string,
    projectType: "mod" | "modpack",
    mcVersion: string | undefined,
    loader: string | undefined,
    limit: number,
  ): Promise<RawSearchHit[]> {
    const resp = await this.json<{ hits?: RawSearchHit[] }>("/search", {
      query,
      facets: buildFacets(projectType, mcVersion, loader),
      limit: String(clamp(limit, 1, 100)),
      index: "relevance",
    });
    return resp.hits ?? [];
  }

  /** `GET /project/{id}`. */
  getProject(id: string): Promise<RawProject> {
    return this.json<RawProject>(`/project/${encodeURIComponent(id)}`, {});
  }

  /** `GET /projects?ids=[...]`. */
  getProjects(ids: string[]): Promise<RawProject[]> {
    if (ids.length === 0) return Promise.resolve([]);
    return this.json<RawProject[]>("/projects", { ids: JSON.stringify(ids) });
  }

  /** `GET /project/{id}/version` filtered by loader/game version (newest first). */
  listVersions(
    id: string,
    mcVersion?: string,
    loader?: string,
  ): Promise<RawVersion[]> {
    const params: Record<string, string> = {};
    const loaders = loader ? acceptedLoaders(loader) : [];
    if (loaders.length) params.loaders = JSON.stringify(loaders);
    if (mcVersion) params.game_versions = JSON.stringify([mcVersion]);
    return this.json<RawVersion[]>(`/project/${encodeURIComponent(id)}/version`, params);
  }

  /** Download a file (the base `.mrpack`), capped so a hostile size can't OOM us. */
  async getBytes(url: string, cap: number): Promise<Buffer> {
    const res = await this.fetchImpl(url, { headers: { "user-agent": USER_AGENT } });
    if (!res.ok) throw new Error(`download failed: HTTP ${res.status} for ${url}`);
    const buf = Buffer.from(await res.arrayBuffer());
    if (buf.length > cap) throw new Error(`base archive exceeds maximum size of ${cap} bytes`);
    return buf;
  }
}

// --- Facets / loader / version helpers (ported from Rust) ---------------------

/** Quilt instances also accept Fabric builds; every other loader is just itself. */
function acceptedLoaders(loader: string): string[] {
  const l = loader.trim().toLowerCase();
  if (l === "") return [];
  if (l === "quilt") return ["quilt", "fabric"];
  return [l];
}

/** Modrinth `facets`: an AND of OR-groups (see `modplatform::modrinth::build_facets`). */
function buildFacets(
  projectType: string,
  mcVersion: string | undefined,
  loader: string | undefined,
): string {
  const groups: string[][] = [[`project_type:${projectType}`]];
  const loaders = loader ? acceptedLoaders(loader) : [];
  if (loaders.length) groups.push(loaders.map((l) => `categories:${l}`));
  if (mcVersion) groups.push([`versions:${mcVersion}`]);
  return JSON.stringify(groups);
}

function primaryFile(v: RawVersion): RawFile | undefined {
  const files = v.files ?? [];
  return files.find((f) => f.primary) ?? files[0];
}

/**
 * Pick the "best" version for a target, matching `dependency::pick_best_version`:
 * mc+loader match > mc match > loader match > first (list is newest-first).
 */
function pickBestVersion(
  versions: RawVersion[],
  mcVersion: string,
  loader: string,
): RawVersion | undefined {
  const accepted = acceptedLoaders(loader).map((l) => l.toLowerCase());
  const mcOk = (v: RawVersion) => (v.game_versions ?? []).includes(mcVersion);
  const loaderOk = (v: RawVersion) =>
    (v.loaders ?? []).some((l) => accepted.includes(String(l).toLowerCase()));
  return (
    versions.find((v) => mcOk(v) && loaderOk(v)) ??
    versions.find(mcOk) ??
    versions.find(loaderOk) ??
    versions[0]
  );
}

function clamp(n: number, lo: number, hi: number): number {
  return Math.min(Math.max(n, lo), hi);
}

// --- Base-modlist: read one entry out of a `.mrpack` (a zip) ------------------

/**
 * Read the shallowest zip entry whose basename is `basename` (matches Rust
 * `read_shallow_zip_entry`), or `null` if absent. Supports STORED (method 0) and
 * DEFLATE (method 8) — the only methods `.mrpack` files use.
 * ponytail: no ZIP64 (64-bit sizes); `.mrpack` files are small and 32-bit.
 * Add ZIP64 handling only if a real pack ever trips the "not a zip" error.
 */
function readZipEntry(buf: Buffer, basename: string): Buffer | null {
  const EOCD_SIG = 0x06054b50;
  let eocd = -1;
  for (let i = buf.length - 22; i >= 0; i--) {
    if (buf.readUInt32LE(i) === EOCD_SIG) {
      eocd = i;
      break;
    }
  }
  if (eocd < 0) throw new Error("base archive is not a valid zip (no end-of-central-directory)");
  const cdCount = buf.readUInt16LE(eocd + 10);
  let p = buf.readUInt32LE(eocd + 16);

  let best: { localOffset: number; compSize: number; method: number; depth: number } | null = null;
  for (let n = 0; n < cdCount; n++) {
    if (buf.readUInt32LE(p) !== 0x02014b50) break; // central-directory header
    const method = buf.readUInt16LE(p + 10);
    const compSize = buf.readUInt32LE(p + 20);
    const nameLen = buf.readUInt16LE(p + 28);
    const extraLen = buf.readUInt16LE(p + 30);
    const commentLen = buf.readUInt16LE(p + 32);
    const localOffset = buf.readUInt32LE(p + 42);
    const name = buf.toString("utf8", p + 46, p + 46 + nameLen).replace(/\\/g, "/");
    const tail = name.split("/").pop() ?? name;
    if (tail === basename) {
      const depth = (name.match(/\//g) ?? []).length;
      if (!best || depth < best.depth) best = { localOffset, compSize, method, depth };
    }
    p += 46 + nameLen + extraLen + commentLen;
  }
  if (!best) return null;

  const lo = best.localOffset;
  if (buf.readUInt32LE(lo) !== 0x04034b50) throw new Error("bad local zip header");
  const dataStart = lo + 30 + buf.readUInt16LE(lo + 26) + buf.readUInt16LE(lo + 28);
  const comp = buf.subarray(dataStart, dataStart + best.compSize);
  if (best.method === 0) return Buffer.from(comp);
  if (best.method === 8) return inflateRawSync(comp);
  throw new Error(`unsupported zip compression method ${best.method}`);
}

/** A `.mrpack` file is client-relevant unless it declares `env.client == "unsupported"`. */
function clientSupported(file: { env?: { client?: string } }): boolean {
  return file.env?.client !== "unsupported";
}

/** Pull the Modrinth project id out of a CDN download url (`…/data/<id>/…`). */
function modrinthProjectIdFromUrl(url: string): string | null {
  const marker = "/data/";
  const at = url.indexOf(marker);
  if (at < 0) return null;
  const id = url.slice(at + marker.length).split("/")[0]?.trim();
  return id ? id : null;
}

// --- Cross-provider ref parsing (ported from tools.rs::parse_mod_ref) ---------

interface Ref {
  provider: string;
  id: string;
}
function parseModRef(raw: string): Ref {
  const at = raw.indexOf(":");
  if (at > 0) {
    const slug = raw.slice(0, at);
    if (slug === "modrinth" || slug === "curseforge") return { provider: slug, id: raw.slice(at + 1).trim() };
  }
  return { provider: "modrinth", id: raw.trim() };
}
const refKey = (r: Ref) => `${r.provider}:${r.id}`;

// --- Executor ----------------------------------------------------------------

export interface ModrinthExecutorOptions {
  /** Override the API root (tests / mirrors). Defaults to Modrinth v2. */
  baseUrl?: string;
  /** Override the fetch implementation (tests). Defaults to the global `fetch`. */
  fetch?: FetchLike;
}

/**
 * A read-only `ToolExecutor` over the Modrinth HTTP API. Suitable for a hosted
 * server that runs the brain; `build_modpack` is intentionally unsupported here.
 */
export function modrinthExecutor(opts: ModrinthExecutorOptions = {}): ToolExecutor {
  const client = new ModrinthClient(opts.baseUrl ?? API_BASE, opts.fetch ?? fetch);

  return {
    async search_base_modpacks(a) {
      const { query, mc_version, loader } = a as {
        query: string;
        mc_version?: string;
        loader?: string;
      };
      const hits = await client.search(query.trim(), "modpack", mc_version, loader, BASE_SEARCH_TOTAL_CAP);
      const candidates = hits.slice(0, BASE_SEARCH_TOTAL_CAP).map((h) => ({
        provider: "modrinth",
        project_id: h.project_id,
        slug: h.slug,
        title: h.title,
        author: h.author,
        downloads: h.downloads,
        description: h.description,
      }));
      return { candidates };
    },

    async search_mods(a) {
      const { query, mc_version, loader } = a as {
        query: string;
        mc_version: string;
        loader: string;
      };
      const hits = await client.search(query.trim(), "mod", mc_version, loader, MOD_SEARCH_PER_QUERY_CAP);
      const mods = hits.slice(0, MOD_SEARCH_TOTAL_CAP).map((h) => ({
        provider: "modrinth",
        project_id: h.project_id,
        slug: h.slug,
        title: h.title,
        downloads: h.downloads,
        description: h.description,
      }));
      return { mods };
    },

    async mod_get_detail(a) {
      const { provider, project_id, minecraft_version, loader } = a as {
        provider?: string;
        project_id: string;
        minecraft_version?: string;
        loader?: string;
      };
      const p = (provider ?? "modrinth").toLowerCase();
      if (p !== "modrinth") throw new Error(`provider ${p} is not supported by modrinthExecutor`);
      const id = project_id.trim();
      const proj = await client.getProject(id);
      const project = {
        title: proj.title,
        slug: proj.slug,
        description: proj.description,
        categories: proj.categories ?? [],
        downloads: proj.downloads,
      };
      const versionsRaw = await client.listVersions(id, minecraft_version, loader);
      const versions = versionsRaw.slice(0, MOD_DETAIL_VERSION_CAP).map((v) => ({
        version_id: v.id,
        version_number: v.version_number,
        game_versions: v.game_versions ?? [],
        loaders: v.loaders ?? [],
        dependencies_count: (v.dependencies ?? []).length,
        filename: primaryFile(v)?.filename ?? null,
      }));
      return { project, versions };
    },

    async resolve_mods(a) {
      const { project_ids, mc_version, loader, already_installed } = a as {
        project_ids: string[];
        mc_version: string;
        loader: string;
        already_installed?: string[];
      };
      const already = new Set((already_installed ?? []).map((s) => refKey(parseModRef(s))));
      const visited = new Set<string>();
      const queue: { ref: Ref; depth: number }[] = [];
      const resolved: unknown[] = [];
      const unresolved: unknown[] = [];
      const conflicts: unknown[] = [];

      for (const raw of project_ids) {
        const r = parseModRef(raw);
        if (!visited.has(refKey(r))) {
          visited.add(refKey(r));
          queue.push({ ref: r, depth: 0 });
        }
      }

      while (queue.length) {
        const { ref, depth } = queue.shift()!;
        if (already.has(refKey(ref))) continue; // satisfied — dropped from output (as in Rust)
        if (ref.provider !== "modrinth") {
          unresolved.push({ provider: ref.provider, project_id: ref.id });
          continue;
        }
        const versions = await client.listVersions(ref.id, mc_version.trim(), loader.trim());
        const picked = pickBestVersion(versions, mc_version.trim(), loader.trim());
        const file = picked ? primaryFile(picked) : undefined;
        if (!picked || !file) {
          unresolved.push({ provider: "modrinth", project_id: ref.id });
          continue;
        }
        resolved.push({
          provider: "modrinth",
          project_id: ref.id,
          version_id: picked.id,
          filename: file.filename,
          url: file.url,
          sha1: file.hashes?.sha1 ?? null,
          sha512: file.hashes?.sha512 ?? null,
          size: file.size ?? null,
        });
        if (depth >= MAX_DEPTH) continue;
        for (const dep of picked.dependencies ?? []) {
          const depId = dep.project_id?.trim();
          if (!depId) continue;
          const depRef: Ref = { provider: ref.provider, id: depId };
          if (dep.dependency_type === "required") {
            if (!visited.has(refKey(depRef))) {
              visited.add(refKey(depRef));
              queue.push({ ref: depRef, depth: depth + 1 });
            }
          } else if (dep.dependency_type === "incompatible" && !visited.has(refKey(depRef))) {
            visited.add(refKey(depRef));
            conflicts.push({ provider: depRef.provider, project_id: depRef.id });
          }
        }
      }
      return { resolved, unresolved, conflicts };
    },

    async inspect_base_modpack(a) {
      const { project_id, mc_version, loader } = a as {
        project_id: string;
        mc_version?: string;
        loader?: string;
      };
      const versions = await client.listVersions(project_id.trim(), mc_version, loader);
      const version = versions.find((v) => primaryFile(v));
      if (!version) {
        throw new Error(
          `no downloadable version found for base pack ${project_id} (mc=${mc_version ?? "any"}, loader=${loader ?? "any"})`,
        );
      }
      const archive = primaryFile(version)!;
      const bytes = await client.getBytes(archive.url.trim(), MAX_BASE_ARCHIVE_BYTES);
      const indexBytes = readZipEntry(bytes, "modrinth.index.json");
      if (!indexBytes) {
        if (readZipEntry(bytes, "manifest.json")) {
          throw new Error("CurseForge base packs are not supported by modrinthExecutor");
        }
        throw new Error("base archive missing modrinth.index.json");
      }
      const index = JSON.parse(indexBytes.toString("utf8")) as {
        files?: { downloads?: string[]; env?: { client?: string } }[];
      };

      const ids: string[] = [];
      const seen = new Set<string>();
      for (const file of index.files ?? []) {
        if (!clientSupported(file)) continue;
        const id = (file.downloads ?? []).map(modrinthProjectIdFromUrl).find((x) => x);
        if (id && !seen.has(id)) {
          seen.add(id);
          ids.push(id);
        }
      }

      const hits = await client.getProjects(ids);
      const categories = new Set<string>();
      const mods = hits.map((h) => {
        for (const c of h.categories ?? []) categories.add(c);
        return { title: h.title, categories: h.categories ?? [] };
      });
      mods.sort((x, y) => x.title.toLowerCase().localeCompare(y.title.toLowerCase()));

      return {
        title: version.name,
        mc_version: mc_version ?? version.game_versions?.[0] ?? null,
        loader: loader ?? version.loaders?.[0] ?? null,
        mod_count: ids.length,
        mods,
        covered_features: [...categories].sort(),
      };
    },

    // The only writing tool — not available without the desktop host. Returns a
    // structured error the model can relay, rather than throwing.
    async build_modpack() {
      return { status: "unsupported", error: BUILD_UNSUPPORTED_MESSAGE };
    },

    // Launcher-side tool — no instances without the desktop host.
    async list_instances() {
      return { instances: [] };
    },
  };
}
