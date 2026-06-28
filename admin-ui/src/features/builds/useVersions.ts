import { useQuery } from "@tanstack/react-query";

// Build-time-generated version catalog served as a static asset under the SPA base.
// The minecraft-kit it is generated from never reaches the browser bundle — only this JSON.

export interface MinecraftVersionEntry {
  id: string;
  type: string;
}

// Each field optional so a partial/legacy entry never breaks the UI.
export interface RecommendedVersions {
  java?: number;
  forge?: string | null;
  fabric?: string | null;
}

export interface VersionsCatalog {
  version?: number;
  minecraft: MinecraftVersionEntry[];
  fabric: string[];
  forge: Record<string, string[]>;
  java: number[];
  recommended: Record<string, RecommendedVersions>;
  generatedAt: string;
}

const EMPTY_CATALOG: VersionsCatalog = {
  minecraft: [],
  fabric: [],
  forge: {},
  java: [],
  recommended: {},
  generatedAt: new Date(0).toISOString(),
};

export function versionsUrl(): string {
  // `import.meta.env.BASE_URL` is "/admin/" in prod (vite `base`) and "/" in tests.
  return `${import.meta.env.BASE_URL}versions.json`;
}

export function useVersions() {
  return useQuery({
    queryKey: ["versions"],
    queryFn: async (): Promise<VersionsCatalog> => {
      const res = await fetch(versionsUrl(), {
        headers: { Accept: "application/json" },
      });
      if (!res.ok) {
        // A missing/broken static asset must not break the page.
        return EMPTY_CATALOG;
      }
      // Normalize an older payload to the current shape; defaults must follow the
      // spread so they win over any `undefined` field.
      const data = (await res.json()) as Partial<VersionsCatalog>;
      return {
        ...data,
        java: data.java ?? [],
        recommended: data.recommended ?? {},
        version: data.version ?? 1,
      } as VersionsCatalog;
    },
    staleTime: Number.POSITIVE_INFINITY,
  });
}
