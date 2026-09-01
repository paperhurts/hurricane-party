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
        library: resolve(import.meta.dirname, "index.html"),
        main: resolve(import.meta.dirname, "main.html"),
        eq: resolve(import.meta.dirname, "eq.html"),
        playlist: resolve(import.meta.dirname, "playlist.html"),
        video: resolve(import.meta.dirname, "video.html"),
        root: resolve(import.meta.dirname, "root.html"),
      },
    },
  },
});
