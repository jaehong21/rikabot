import { defineConfig } from "@rsbuild/core";
import { pluginReact } from "@rsbuild/plugin-react";

export default defineConfig({
  plugins: [pluginReact()],
  html: {
    template: "./index.html",
  },
  source: {
    define: {
      "process.env.RIKA_DEV_BACKEND_WS_HOSTPORT": JSON.stringify(
        process.env.RIKA_DEV_BACKEND_WS_HOSTPORT ?? "",
      ),
      "process.env.RIKA_DEV_BACKEND_WS_URL": JSON.stringify(
        process.env.RIKA_DEV_BACKEND_WS_URL ?? "",
      ),
    },
    entry: {
      index: "./src/main.tsx",
    },
  },
  output: {
    distPath: {
      root: "dist",
    },
    cleanDistPath: true,
  },
});
