/// <reference types="vitest/config" />
import path from "node:path";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

import { devProxy } from "./devProxy.ts";

// The admin SPA is served by the `admin` crate under `/admin`, so all asset URLs
// must be prefixed accordingly. During `vite dev` the dev server proxies the
// backend prefixes in `devProxy` to the Rust server.
export default defineConfig({
  base: "/admin/",
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@": path.resolve(import.meta.dirname, "./src"),
    },
  },
  build: {
    rolldownOptions: {
      output: {
        codeSplitting: {
          // Isolate recharts (the largest dependency, used only by the lazily
          // loaded Dashboard/Logs pages) into its own chunk.
          groups: [
            {
              name: "recharts",
              test: /node_modules[\\/](recharts|recharts-scale|react-smooth|victory-vendor|d3-[a-z]+|internmap)[\\/]/,
              // why: this defaults to true, which would drag shared deps
              // (React, clsx) into the chart chunk and make the entry load all
              // 380 kB of it on every route.
              includeDependenciesRecursively: false,
            },
          ],
        },
      },
    },
  },
  server: {
    proxy: devProxy,
  },
  test: {
    globals: true,
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
    css: true,
  },
});
