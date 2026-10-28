import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

const here = (file: string) => fileURLToPath(new URL(file, import.meta.url));

// The interface of the app (index.html) and of the installer (setup.html, see
// apps/desktop/src/setup) share styles and translations. The build goes to
// dist/ at the root, where Tauri picks it up. Tauri loads the dev server at a
// fixed port.
export default defineConfig({
  root: here("."),
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: {
    target: "es2022",
    outDir: here("../dist"),
    emptyOutDir: true,
    rollupOptions: { input: { app: here("index.html"), setup: here("setup.html") } },
  },
});
