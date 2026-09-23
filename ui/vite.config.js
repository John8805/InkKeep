import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

export default defineConfig({
  plugins: [svelte()],
  build: { outDir: "dist", emptyOutDir: true, target: "chrome110" },
  server: { port: 5173, strictPort: true },
  clearScreen: false,
});
