import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "node:path";

// The folio-embed build (`pnpm build` runs it after the app): the folio reader alone, with its marks and comments,
// for a native client's WebView (the Android app; src/embed/main.tsx). Relative asset paths, so it works at
// any address. Output: dist/folio-embed/, embedded in the binary with the UI.
export default defineConfig({
  root: path.resolve(import.meta.dirname, "folio-embed"),
  base: "./",
  plugins: [react(), tailwindcss()],
  resolve: { alias: { "@": path.resolve(import.meta.dirname, "src") } },
  build: { target: "es2022", outDir: path.resolve(import.meta.dirname, "dist/folio-embed"), emptyOutDir: true, chunkSizeWarningLimit: 4000 },
});
