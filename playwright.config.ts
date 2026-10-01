import { defineConfig } from "@playwright/test";

// End-to-end tests run the Vite dev server against a throwaway SpacetimeDB
// database (`asimov-e2e`, see `e2e/global-setup.ts`). Needs `npm run infra:up`.
const PORT = 5174;

export default defineConfig({
  testDir: "e2e",
  globalSetup: "./e2e/global-setup.ts",
  globalTeardown: "./e2e/global-teardown.ts",
  use: { baseURL: `http://localhost:${PORT}` },
  webServer: {
    command: `npx vite --config config/vite.config.ts --port ${PORT} --strictPort`,
    url: `http://localhost:${PORT}`,
    env: { VITE_SPACETIMEDB_DB_NAME: "asimov-e2e" },
    reuseExistingServer: false,
  },
});
