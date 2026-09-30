import { defineConfig } from "vite";
import solid from "vite-plugin-solid";

// Tauri expects a fixed dev port and must not have its Rust sources watched by Vite.
export default defineConfig({
  plugins: [solid()],
  clearScreen: false,
  server: { port: 1420, strictPort: true, watch: { ignored: ["**/src-tauri/**", "**/crates/**", "**/target/**"] } },
});
