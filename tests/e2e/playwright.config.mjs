import path from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig, devices } from "@playwright/test";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const ROOT_DIR = path.resolve(__dirname, "..", "..");

const resultsDir = path.join(ROOT_DIR, ".tmp", "e2e", "playwright", "results");
const reportDir = path.join(ROOT_DIR, ".tmp", "e2e", "playwright", "report");

export default defineConfig({
  testDir: path.join(__dirname, "specs"),
  outputDir: resultsDir,
  fullyParallel: false,
  workers: 1,
  timeout: 30_000,
  expect: {
    timeout: 12_000,
  },
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI
    ? [["line"], ["html", { open: "never", outputFolder: reportDir }]]
    : [["list"], ["html", { open: "never", outputFolder: reportDir }]],
  use: {
    baseURL: "http://127.0.0.1:13000",
    trace: "on-first-retry",
    video: "retain-on-failure",
  },
  webServer: [
    {
      command: "bun tests/e2e/support/mock-openai.mjs",
      cwd: ROOT_DIR,
      url: "http://127.0.0.1:8797/health",
      reuseExistingServer: false,
      timeout: 30_000,
    },
    {
      command: "bash tests/e2e/scripts/start-backend-e2e.sh",
      cwd: ROOT_DIR,
      url: "http://127.0.0.1:14728/health",
      env: {
        RIKA_DEV_MODE: "1",
        RIKA_DEV_CORS_ORIGINS: "http://localhost:13000,http://127.0.0.1:13000",
      },
      reuseExistingServer: false,
      timeout: 180_000,
    },
    {
      command: "bun run dev -- --host 127.0.0.1 --port 13000",
      cwd: ROOT_DIR + "/web",
      url: "http://127.0.0.1:13000",
      env: {
        RIKA_DEV_MODE: "1",
        RIKA_DEV_BACKEND_HOSTPORT: "127.0.0.1:14728",
      },
      reuseExistingServer: false,
      timeout: 120_000,
    },
  ],
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
});
