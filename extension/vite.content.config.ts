import { defineConfig } from "vite";
import path from "node:path";

// Content scripts: one self-contained classic script each (Chrome does not load them as modules), added to the dist of
// the main build (vite.config.ts runs first and empties it).
export default defineConfig({
  resolve: { alias: { "@": path.resolve(import.meta.dirname, "../web/src") } },
  publicDir: false,
  build: {
    target: "es2022",
    emptyOutDir: false,
    lib: { entry: path.resolve(import.meta.dirname, "src/content/github.ts"), formats: ["iife"], name: "launchpadGithub", fileName: () => "content-github.js" },
  },
});
