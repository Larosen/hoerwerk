import { defineConfig } from "@playwright/test";

// Click-through tests against the real server (Python app + ffmpeg) with throwaway folders.
// Run: npm run build && npx playwright test   (CI installs Chromium via `npx playwright install`)
const PORT = Number(process.env.E2E_PORT || 8099);
// E2E_URL: test an already running server (e.g. the Docker image, folders prepared with e2e/setup.mjs)
const EXTERNAL = process.env.E2E_URL;
const ROOT = process.env.E2E_ROOT || "e2e/.run";

export default defineConfig({
  testDir: "e2e",
  timeout: 60_000,
  expect: { timeout: 20_000 },
  workers: 1,                       // one server, shared folders: tests run one after another
  fullyParallel: false,
  retries: 0,
  reporter: process.env.CI ? [["list"], ["html", { open: "never" }]] : "list",
  use: {
    baseURL: EXTERNAL || `http://127.0.0.1:${PORT}`,
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
    launchOptions: process.env.PW_CHROMIUM ? { executablePath: process.env.PW_CHROMIUM } : {},
  },
  webServer: EXTERNAL ? undefined : {
    command: "node e2e/setup.mjs && python3 app/main.py",
    url: `http://127.0.0.1:${PORT}/api/health`,
    reuseExistingServer: false,
    timeout: 60_000,
    env: {
      PORT: String(PORT),
      DATA_DIR: `${ROOT}/data`, CONFIG_DIR: `${ROOT}/config`, INBOX_DIR: `${ROOT}/inbox`, LIBRARY_DIR: `${ROOT}/library`,
      // offline and deterministic: files are filed with their own tags, no shop lookups
      MATCH_MODE: "off", METADATA_FALLBACK: "false", SETTLE_SECONDS: "0", SCAN_INTERVAL: "2",
      DUPLICATES: "ask", AUDIOBOOK_WATCH: "true", LIBRARY_RESCAN_HOURS: "6",
    },
  },
});
