import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// Loaded via `--config config/vite.config.ts` from the npm scripts, which run
// in the project root, so Vite's root (index.html, src/) is the project root.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: { "@": fileURLToPath(new URL("../frontend", import.meta.url)) },
  },
});
