import type { IncomingMessage } from "node:http";
import { describe, expect, it } from "vitest";

import { bypassHtmlNavigation, devProxy } from "./devProxy";

function request(
  url: string,
  accept: string,
  method = "GET",
): IncomingMessage {
  return { url, method, headers: { accept } } as unknown as IncomingMessage;
}

const HTML_ACCEPT =
  "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8";

describe("dev proxy", () => {
  it("keeps browser navigations to /admin/* on the dev server", () => {
    // These prefixes are also App.tsx client routes, so a navigation must reach
    // vite's SPA fallback instead of the backend's embedded index.html.
    expect(bypassHtmlNavigation(request("/admin/textures", HTML_ACCEPT))).toBe(
      "/admin/textures",
    );
    expect(bypassHtmlNavigation(request("/admin/logs", HTML_ACCEPT))).toBe(
      "/admin/logs",
    );
    expect(bypassHtmlNavigation(request("/admin/users", HTML_ACCEPT))).toBe(
      "/admin/users",
    );
  });

  it("proxies data requests to the backend", () => {
    expect(
      bypassHtmlNavigation(
        request("/admin/textures/skins", "application/json"),
      ),
    ).toBeUndefined();
    expect(
      bypassHtmlNavigation(request("/admin/logs/tail", "*/*")),
    ).toBeUndefined();
    expect(
      bypassHtmlNavigation(request("/textures/abc/skin", "image/avif,image/*")),
    ).toBeUndefined();
    expect(
      bypassHtmlNavigation(
        request("/admin/textures/purge-missing", HTML_ACCEPT, "POST"),
      ),
    ).toBeUndefined();
  });

  it("gives every /admin/* prefix the navigation bypass", () => {
    const admin = Object.entries(devProxy).filter(([prefix]) =>
      prefix.startsWith("/admin/"),
    );
    expect(admin.length).toBeGreaterThan(0);
    for (const [prefix, options] of admin) {
      expect(
        typeof options === "string" ? undefined : options.bypass,
        `${prefix} may shadow a client route, so it needs the bypass`,
      ).toBe(bypassHtmlNavigation);
    }
  });
});
