import { defineConfig } from "@rsbuild/core";
import { pluginReact } from "@rsbuild/plugin-react";

export default defineConfig({
  plugins: [pluginReact()],
  html: {
    template: "./index.html",
  },
  source: {
    define: {
      "process.env.RIKA_DEV_BACKEND_HOSTPORT": JSON.stringify(
        process.env.RIKA_DEV_BACKEND_HOSTPORT ?? "",
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
