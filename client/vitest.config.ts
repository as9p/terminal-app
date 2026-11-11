import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.ts"],
    // Without this, CSS imports read as empty, so a test cannot check styles.css as text.
    css: { include: [/styles\.css/] },
  },
});
