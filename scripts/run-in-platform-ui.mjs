import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const uiDir = path.join(__dirname, "..", "platform", "ui");

const cmd = process.argv[2];
const map = {
  lint: ["npx", "next", "lint"],
  "type-check": ["npx", "tsc", "--noEmit"],
  build: ["npx", "next", "build"],
  dev: ["npx", "next", "dev"],
  start: ["npx", "next", "start"],
};

const argv = map[cmd];
if (!argv) {
  console.error("Usage: node scripts/run-in-platform-ui.mjs <lint|type-check|build|dev|start>");
  process.exit(1);
}

const [exe, ...args] = argv;
const r = spawnSync(exe, args, {
  cwd: uiDir,
  stdio: "inherit",
  shell: process.platform === "win32",
  env: process.env,
});
process.exit(r.status ?? 1);
