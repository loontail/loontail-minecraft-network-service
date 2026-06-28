// Build-time generator for `public/versions.json` — the static data that powers
// the cascading Minecraft / Forge / Fabric / Java dropdowns in the Build detail
// page. It mirrors the four resolvers the old Strapi `minecraft-versions` plugin
// called, but resolves everything once at build time instead of per-request.
//
// `@loontail/minecraft-kit` is a Node-only, build-script-only dependency (it
// re-exports node:child_process/fs/os and must never enter the browser bundle).
// It is declared as an OPTIONAL dependency: the kit lives in a sibling repo that
// is NOT present in CI / Docker checkouts (only this repo + admin-ui/ are copied),
// so `npm ci` skips it there. When the kit is missing — or the upstream Mojang /
// Forge / Fabric meta endpoints are unreachable — we keep the committed seed
// `public/versions.json` and exit 0, so the build NEVER hard-fails offline.

import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const OUTPUT_PATH = resolve(__dirname, "../public/versions.json");

// Java majors always offered in the picker regardless of what we manage to
// resolve — the common LTS/runtime set plus the newest known. Resolved majors
// are merged on top (deduped, descending) so a build never ships an empty Java
// dropdown.
const DEFAULT_JAVA = [25, 21, 17, 16, 8];

// Bound the per-MC manifest resolution cost: each release needs one extra
// (cached) HTTP round-trip to read its `javaVersion.majorVersion`. Resolve only
// the newest N releases; set to 0 to skip the Java step entirely (offline gate).
const RESOLVE_LIMIT = Number(process.env.VERSIONS_JAVA_LIMIT ?? 30);
const RESOLVE_CONCURRENCY = 6;

// Minimal committed fallback used only when no seed exists yet AND generation
// fails. The real seed (a richer snapshot) is committed to public/versions.json.
const MINIMAL_FALLBACK = {
  version: 2,
  minecraft: [{ id: "1.21.4", type: "release" }],
  fabric: ["0.16.10"],
  forge: {},
  java: [...DEFAULT_JAVA],
  recommended: {},
  generatedAt: new Date(0).toISOString(),
};

function keepExistingOrSeed(reason) {
  if (existsSync(OUTPUT_PATH)) {
    console.warn(
      `[generate-versions] ${reason}; keeping existing ${OUTPUT_PATH}`,
    );
    return;
  }
  console.warn(
    `[generate-versions] ${reason}; writing minimal fallback ${OUTPUT_PATH}`,
  );
  writeFileSync(OUTPUT_PATH, `${JSON.stringify(MINIMAL_FALLBACK, null, 2)}\n`);
}

// Read the prior committed catalog so a smaller run (lower RESOLVE_LIMIT) does
// not drop Java majors a richer prior run already discovered.
function readPriorCatalog() {
  if (!existsSync(OUTPUT_PATH)) return null;
  try {
    return JSON.parse(readFileSync(OUTPUT_PATH, "utf8"));
  } catch {
    return null;
  }
}

// Resolve `tasks` with a small concurrency pool so we never open dozens of
// simultaneous sockets. Each task is a `() => Promise` thunk.
async function runPool(tasks, concurrency) {
  let cursor = 0;
  async function worker() {
    while (cursor < tasks.length) {
      const index = cursor++;
      await tasks[index]();
    }
  }
  const workers = [];
  for (let i = 0; i < Math.min(concurrency, tasks.length); i++) {
    workers.push(worker());
  }
  await Promise.all(workers);
}

async function loadKit() {
  try {
    const mod = await import("@loontail/minecraft-kit");
    return mod;
  } catch {
    return null;
  }
}

async function main() {
  const kitModule = await loadKit();
  if (!kitModule) {
    keepExistingOrSeed("@loontail/minecraft-kit is not installed");
    return;
  }
  const { MinecraftKit, asMinecraftVersionId } = kitModule;

  const kit = new MinecraftKit();
  const prior = readPriorCatalog();

  // Vanilla release channel (the picker defaults to releases; snapshots are noisy).
  const mcSummaries = await kit.versions.minecraft.list({ channel: "release" });
  const minecraft = mcSummaries.map((s) => ({ id: s.id, type: s.type }));

  // Fabric loader versions are largely Minecraft-independent — one unfiltered
  // listing covers every MC (the cascade only gates Fabric on "MC is chosen").
  const fabricLoaders = await kit.versions.fabric.list();
  const fabric = fabricLoaders.map((l) => l.version);

  // One unfiltered Forge listing carries `minecraftVersion` per build. Maven
  // metadata is OLDEST-first, so we group by MC, reverse to newest-first for the
  // dropdown, and pick the recommended/latest/newest build mirroring the kit's
  // `pickForge` precedence (recommended -> latest -> last == newest).
  const forgeBuilds = await kit.versions.forge.list();
  const forge = {};
  const forgeRecommended = {};
  const byMc = new Map();
  for (const b of forgeBuilds) {
    if (!byMc.has(b.minecraftVersion)) byMc.set(b.minecraftVersion, []);
    byMc.get(b.minecraftVersion).push(b);
  }
  for (const [mc, builds] of byMc) {
    const newestFirst = [...builds].reverse();
    forge[mc] = newestFirst.map((b) => b.forgeVersion);
    const chosen =
      builds.find((b) => b.isRecommended) ??
      builds.find((b) => b.isLatest) ??
      builds[builds.length - 1];
    forgeRecommended[mc] = chosen.forgeVersion;
  }

  // Resolve the per-MC recommended Java major for the newest releases only,
  // bounded by RESOLVE_LIMIT + a small concurrency pool. A per-MC try/catch keeps
  // any single 404/parse failure from failing the build.
  const resolvedJava = {};
  const resolvedMajors = new Set();
  const releases = mcSummaries.filter((s) => s.type === "release");
  const toResolve = releases.slice(0, Math.max(0, RESOLVE_LIMIT));
  const tasks = toResolve.map((m) => async () => {
    try {
      const r = await kit.versions.minecraft.resolve({
        version: asMinecraftVersionId(m.id),
      });
      const major = r.manifest.javaVersion?.majorVersion ?? 8;
      resolvedJava[m.id] = major;
      resolvedMajors.add(major);
    } catch {
      // No Java for this MC; never fail the build.
    }
  });
  await runPool(tasks, RESOLVE_CONCURRENCY);

  // Merge prior-run Java majors so a smaller run does not regress the per-MC map.
  const priorRecommended = prior?.recommended ?? {};
  const recommended = {};
  for (const m of minecraft) {
    const mc = m.id;
    const priorJava = priorRecommended[mc]?.java;
    const java = resolvedJava[mc] ?? priorJava;
    const entry = {};
    if (java !== undefined) entry.java = java;
    if (mc in forgeRecommended) entry.forge = forgeRecommended[mc];
    else entry.forge = null;
    if (fabric.length > 0) entry.fabric = fabric[0];
    recommended[mc] = entry;
  }

  // Surviving Java majors from a prior run (its `recommended[*].java`) join the
  // default set so the dropdown never loses a major a richer run discovered.
  const priorMajors = Object.values(priorRecommended)
    .map((r) => r?.java)
    .filter((j) => typeof j === "number");
  const java = [
    ...new Set([...resolvedMajors, ...priorMajors, ...DEFAULT_JAVA]),
  ].sort((a, b) => b - a);

  const payload = {
    version: 2,
    minecraft,
    fabric,
    forge,
    java,
    recommended,
    generatedAt: new Date().toISOString(),
  };

  writeFileSync(OUTPUT_PATH, `${JSON.stringify(payload, null, 2)}\n`);
  console.log(
    `[generate-versions] wrote ${OUTPUT_PATH}: ${minecraft.length} MC, ` +
      `${fabric.length} fabric loaders, ${Object.keys(forge).length} forge MC keys, ` +
      `java [${java.join(", ")}], ${Object.keys(resolvedJava).length} resolved Java majors`,
  );
}

main().catch((err) => {
  const message = err instanceof Error ? err.message : String(err);
  keepExistingOrSeed(`generation failed (${message})`);
  // Never hard-fail the build: a usable seed/fallback is guaranteed above.
  process.exit(0);
});
