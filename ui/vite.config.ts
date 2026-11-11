import { defineConfig } from "vite";

// The dev server proxies the CSIL carrier and the server-rendered auth
// pages to a local vcsstard so cookies stay same-origin.
export default defineConfig({
  server: {
    port: 5173,
    proxy: {
      "/csil": { target: "http://127.0.0.1:8080", ws: true },
      "/auth": "http://127.0.0.1:8080",
      "/hooks": "http://127.0.0.1:8080",
    },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    sourcemap: false,
    // Zest tokens use light-dark(); an older target rewrites it to a
    // page-wide media query and color-scheme on part of a page stops working.
    cssTarget: ["chrome123", "firefox120", "safari17.5"],
  },
});
