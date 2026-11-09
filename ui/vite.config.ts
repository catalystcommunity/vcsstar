import { defineConfig } from "vite";
import solid from "vite-plugin-solid";

// The dev server proxies the CSIL carrier and the server-rendered auth
// pages to a local vcsstard so cookies stay same-origin.
export default defineConfig({
  plugins: [solid()],
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
  },
});
