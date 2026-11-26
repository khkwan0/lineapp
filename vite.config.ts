import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { defineConfig, type Plugin } from "vite";

const root = fileURLToPath(new URL(".", import.meta.url));
const buildInfoPath = fileURLToPath(new URL("./build-info.json", import.meta.url));

type BuildInfo = { number: number; time: string };

function readBuildInfo(): BuildInfo {
  try {
    const parsed = JSON.parse(readFileSync(buildInfoPath, "utf8")) as Partial<BuildInfo>;
    const number = typeof parsed.number === "number" && Number.isFinite(parsed.number) ? Math.max(0, Math.floor(parsed.number)) : 0;
    const time = typeof parsed.time === "string" ? parsed.time : "";
    return { number, time };
  } catch {
    return { number: 0, time: "" };
  }
}

function buildInfoPlugin(): Plugin {
  return {
    name: "build-info",
    config(_config, { command }) {
      const info = readBuildInfo();
      if (command === "build") {
        info.number += 1;
        info.time = new Date().toISOString();
        writeFileSync(buildInfoPath, `${JSON.stringify(info, null, 2)}\n`);
      }
      return {
        define: {
          __BUILD_NUMBER__: JSON.stringify(info.number),
          __BUILD_TIME__: JSON.stringify(info.time),
        },
      };
    },
  };
}

export default defineConfig({
  plugins: [buildInfoPlugin()],
  clearScreen: false,
  server: {
    host: "127.0.0.1",
    port: 1420,
    strictPort: true,
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "es2022",
    rollupOptions: {
      input: {
        main: `${root}index.html`,
        profile: `${root}profile.html`,
        members: `${root}members.html`,
      },
    },
  },
});
