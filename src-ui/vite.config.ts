import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri expects a fixed dev port; the Tauri config's `devUrl` points here.
const host = process.env.TAURI_DEV_HOST;

// https://vitejs.dev/config/
export default defineConfig(async () => ({
  plugins: [react()],

  // Tauri dev-server conventions (https://v2.tauri.app):
  // 1. fixed port, fail if unavailable so Tauri can find it
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    host: host || false,
    hmr: host
      ? { protocol: "ws", host, port: 5174 }
      : undefined,
    watch: {
      // tell Vite to ignore the Rust workspace
      ignored: ["**/target/**", "**/crates/**"],
    },
  },
}));
