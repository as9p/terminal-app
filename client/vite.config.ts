import { defineConfig } from "vite";

export default defineConfig({
  clearScreen: false,
  server: {
    // A debug host accepts exactly this origin, so "localhost" and the port must not drift.
    host: "localhost",
    port: 1420,
    strictPort: true,
  },
});
