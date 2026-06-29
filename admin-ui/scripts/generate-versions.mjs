// Build-time generator for `public/versions.json` — the static data that powers
// the cascading Minecraft / Forge / Fabric / Java dropdowns in the Build detail
// page. Resolves the Minecraft / Forge / Fabric / Java options once at build
// time instead of per-request.
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

// Java runtime COMPONENTS always offered in the picker regardless of what we
// manage to resolve — the well-known Mojang java-runtime set with their majors.
// Resolved components are merged on top (deduped, newest versionName per
// component, descending by major) so a build never ships an empty Java dropdown.
// The launcher passes the chosen `component` straight to the kit as
// `runtime.component`; a bare major would crash install ("Runtime component N
// not available"), so the picker must speak component NAMES, never numbers.
const DEFAULT_JAVA = [
  { component: "java-runtime-epsilon", major: 24 },
  { component: "java-runtime-delta", major: 21 },
  { component: "java-runtime-gamma", major: 17 },
  { component: "java-runtime-beta", major: 17 },
  { component: "java-runtime-alpha", major: 16 },
  { component: "jre-legacy", major: 8 },
];

// The non-JRE pseudo-component is never a launchable Java runtime — exclude it.
const EXCLUDED_COMPONENTS = new Set(["minecraft-java-exe"]);

// Bound the per-MC manifest resolution cost: each release needs one extra
// (cached) HTTP round-trip to read its `javaVersion.component`. Resolve only
// the newest N releases; set to 0 to skip the Java step entirely (offline gate).
const RESOLVE_LIMIT = Number(process.env.VERSIONS_JAVA_LIMIT ?? 30);
const RESOLVE_CONCURRENCY = 6;

function javaLabel(component, major) {
  return Number.isFinite(major) ? `Java ${major} — ${component}` : component;
}

// Minimal committed fallback used only when no seed exists yet AND generation
// fails. The real seed (a richer snapshot) is committed to public/versions.json.
const MINIMAL_FALLBACK = {
  version: 3,
  minecraft: [{ id: "1.21.4", type: "release" }],
  fabric: ["0.16.10"],
  forge: {},
  java: DEFAULT_JAVA.map(({ component, major }) => ({
    component,
    label: javaLabel(component, major),
    major,
  })),
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
// not drop Java components a richer prior run already discovered.
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
  const { MinecraftKit, asMinecraftVersionId, detectSystem } = kitModule;

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

  // Enumerate the host platform's runtime components (the kit resolves Java by
  // component NAME). Dedupe by component, keeping the newest versionName per
  // component, and derive a major from the leading versionName integer. The
  // non-JRE pseudo-component is excluded; failures keep the default set.
  const componentEntries = new Map();
  try {
    const runtimes = await kit.versions.runtime.list({ system: detectSystem() });
    for (const r of runtimes) {
      if (EXCLUDED_COMPONENTS.has(r.component)) continue;
      const major = Number.parseInt(r.versionName, 10);
      const prev = componentEntries.get(r.component);
      if (!prev || r.versionName > prev.versionName) {
        componentEntries.set(r.component, {
          component: r.component,
          versionName: r.versionName,
          major: Number.isFinite(major) ? major : undefined,
        });
      }
    }
  } catch {
    // Runtime index unreachable; fall back to the default component set below.
  }

  // Resolve the per-MC recommended Java COMPONENT for the newest releases only,
  // bounded by RESOLVE_LIMIT + a small concurrency pool. A per-MC try/catch keeps
  // any single 404/parse failure from failing the build.
  const resolvedJava = {};
  const releases = mcSummaries.filter((s) => s.type === "release");
  const toResolve = releases.slice(0, Math.max(0, RESOLVE_LIMIT));
  const tasks = toResolve.map((m) => async () => {
    try {
      const r = await kit.versions.minecraft.resolve({
        version: asMinecraftVersionId(m.id),
      });
      const component = r.manifest.javaVersion?.component ?? null;
      if (component) resolvedJava[m.id] = component;
    } catch {
      // No Java for this MC; never fail the build.
    }
  });
  await runPool(tasks, RESOLVE_CONCURRENCY);

  // Merge prior-run Java components so a smaller run does not regress the per-MC map.
  const priorRecommended = prior?.recommended ?? {};
  const recommended = {};
  for (const m of minecraft) {
    const mc = m.id;
    const priorJava =
      typeof priorRecommended[mc]?.java === "string"
        ? priorRecommended[mc].java
        : undefined;
    const java = resolvedJava[mc] ?? priorJava ?? null;
    const entry = { java };
    if (mc in forgeRecommended) entry.forge = forgeRecommended[mc];
    else entry.forge = null;
    if (fabric.length > 0) entry.fabric = fabric[0];
    recommended[mc] = entry;
  }

  // Surviving Java components from a prior run (its component-shaped `java`
  // array) join the resolved + default set so a smaller run never loses a
  // component a richer run discovered. Keep the highest known major per component.
  const merged = new Map();
  function offer(component, major) {
    const prev = merged.get(component);
    const best =
      Number.isFinite(major) && (!prev || !Number.isFinite(prev) || major > prev)
        ? major
        : prev;
    merged.set(component, best);
  }
  for (const e of componentEntries.values()) offer(e.component, e.major);
  for (const e of Array.isArray(prior?.java) ? prior.java : []) {
    if (e && typeof e.component === "string")
      offer(e.component, typeof e.major === "number" ? e.major : undefined);
  }
  for (const e of DEFAULT_JAVA) offer(e.component, e.major);

  const java = [...merged.entries()]
    .map(([component, major]) => ({
      component,
      label: javaLabel(component, major),
      major: Number.isFinite(major) ? major : undefined,
    }))
    .sort((a, b) => (b.major ?? -1) - (a.major ?? -1));

  const payload = {
    version: 3,
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
      `java [${java.map((j) => j.component).join(", ")}], ` +
      `${Object.keys(resolvedJava).length} resolved Java components`,
  );
}

main().catch((err) => {
  const message = err instanceof Error ? err.message : String(err);
  keepExistingOrSeed(`generation failed (${message})`);
  // Never hard-fail the build: a usable seed/fallback is guaranteed above.
  process.exit(0);
});
