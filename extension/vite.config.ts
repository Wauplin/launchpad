import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "node:path";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: { "@": path.resolve(import.meta.dirname, "../web/src") },
    dedupe: ["react", "react-dom", "@tanstack/react-query", "@tanstack/react-router"],
  },
  build: {
    target: "es2022",
    chunkSizeWarningLimit: 2000,
    rolldownOptions: {
      input: { panel: path.resolve(import.meta.dirname, "panel.html"), background: path.resolve(import.meta.dirname, "src/background.ts") },
      output: { entryFileNames: (chunk) => chunk.name === "background" ? "background.js" : "assets/[name]-[hash].js" },
    },
  },
  worker: { format: "es" },
});
