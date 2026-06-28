/// <reference types="vitest/config" />
import path from "node:path";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// The admin SPA is served by the `admin` crate under `/admin`, so all asset URLs
// must be prefixed accordingly. During `vite dev` the dev server proxies `/admin`
// API calls to the Rust backend.
export default defineConfig({
  base: "/admin/",
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  build: {
    rollupOptions: {
      output: {
        // Isolate recharts (the largest dependency, used only by the lazily
        // loaded Dashboard/Logs pages) into its own chunk.
        manualChunks: {
          recharts: ["recharts"],
        },
      },
    },
  },
  server: {
    proxy: {
      "/admin/auth": "http://localhost:8080",
      "/admin/users": "http://localhost:8080",
      "/admin/catalog": "http://localhost:8080",
      "/admin/bundles": "http://localhost:8080",
      "/admin/api-tokens": "http://localhost:8080",
      "/admin/analytics": "http://localhost:8080",
    },
  },
  test: {
    globals: true,
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
    css: true,
  },
});
