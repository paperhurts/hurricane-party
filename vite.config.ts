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
      // One HTML entry per OS window. The three classic 275px windows are
      // separate top-level OS windows that bond to each other, so they cannot
      // share a document -- windows are not routes, and this is the whole
      // reason the frontend is plain Vite rather than SvelteKit.
      input: {
        library: resolve(__dirname, "index.html"),
        main: resolve(__dirname, "main.html"),
        eq: resolve(__dirname, "eq.html"),
        playlist: resolve(__dirname, "playlist.html"),
        video: resolve(__dirname, "video.html"),
      },
    },
  },
});
