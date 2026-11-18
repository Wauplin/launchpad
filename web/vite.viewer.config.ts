import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "node:path";

// The folio-viewer build (`pnpm build` runs it after the app): the folio renderer and theme without Launchpad's API,
// for a folio's shared snapshot (a static Space that also holds folio.json). Relative asset paths, so it works at
// any address. Output: dist/folio-viewer/, embedded in the binary with the UI.
export default defineConfig({
  root: path.resolve(import.meta.dirname, "folio-viewer"),
  base: "./",
  plugins: [react(), tailwindcss()],
  resolve: { alias: { "@": path.resolve(import.meta.dirname, "src") } },
  build: { target: "es2022", outDir: path.resolve(import.meta.dirname, "dist/folio-viewer"), emptyOutDir: true, chunkSizeWarningLimit: 4000 },
});
