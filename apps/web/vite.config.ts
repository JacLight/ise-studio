import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// In development the UI runs on Vite and talks to the engine server on 7420.
export default defineConfig({
  plugins: [react()],
  server: {
    proxy: {
      "/ws": { target: "ws://127.0.0.1:7420", ws: true },
      "/export": { target: "http://127.0.0.1:7420" },
    },
  },
});
