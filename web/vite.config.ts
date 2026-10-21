import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "node:path";

// `pnpm dev` serves the UI with hot reload and proxies the API to the dev daemon (port 7318).
// Sign in once with the address printed by `LAUNCHPAD_ENV=dev launchpad url` (port 5173 instead of 7318).
const daemon = "http://127.0.0.1:7318";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: { alias: { "@": path.resolve(import.meta.dirname, "src") } },
  server: {
    port: 5173,
    strictPort: true,
    proxy: {
      "/api": { target: daemon, changeOrigin: true },
      "/auth": { target: daemon, changeOrigin: true },
      "/img": { target: daemon, changeOrigin: true },
      "/ws": { target: daemon.replace("http", "ws"), ws: true, changeOrigin: true },
    },
  },
  build: {
    target: "es2022",
    chunkSizeWarningLimit: 2000,
    // Lazy pages and xterm stay in their own chunks; everything the first paint needs goes into one
    // chunk instead of many tiny shared ones, to save round trips on slow (mobile) networks.
    rolldownOptions: { output: { codeSplitting: { groups: [{ name: "app", tags: ["$initial"] }] } } },
  },
  // The Shiki worker loads one grammar chunk per language, which needs an ES module worker.
  worker: { format: "es" },
});
