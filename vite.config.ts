import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri uses this environment variable on mobile and LAN dev flows.
const tauriHost = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 5183,
    strictPort: true,
    host: tauriHost || "127.0.0.1",
    hmr: tauriHost
      ? {
          protocol: "ws",
          host: tauriHost,
          port: 1421,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
});
