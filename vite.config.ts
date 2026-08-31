import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { resolve } from "node:path";

// Tauri drives the dev server; the port is fixed and must match
// build.devUrl in src-tauri/tauri.conf.json.
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    rollupOptions: {
      // One HTML entry per OS window. v0.4 adds eq.html and playlist.html here.
      input: {
        main: resolve(__dirname, "index.html"),
        video: resolve(__dirname, "video.html"),
      },
    },
  },
});
