import { spawnSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { delimiter, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const cargoBin = join(homedir(), ".cargo", "bin");
if (existsSync(cargoBin)) {
  process.env.PATH = `${cargoBin}${delimiter}${process.env.PATH ?? ""}`;
}

const flatpakDir = join(root, "flatpak");
const stage = join(flatpakDir, "stage");

function statMtime(path) {
  return statSync(path).mtimeMs;
}

function fail(message) {
  console.error(message);
  process.exit(1);
}

function run(command, args, env = process.env) {
  const result = spawnSync(command, args, { cwd: root, stdio: "inherit", env });
  if (result.error) {
    fail(result.error.message);
  }
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

function capture(command, args) {
  const result = spawnSync(command, args, { cwd: root, encoding: "utf8" });
  if (result.error || result.status !== 0) {
    return "";
  }
  return result.stdout;
}

function ok(command, args) {
  const result = spawnSync(command, args, { cwd: root, encoding: "utf8" });
  return !result.error && result.status === 0;
}

function targetRoots() {
  const roots = [];
  if (process.env.CARGO_TARGET_DIR) {
    roots.push(process.env.CARGO_TARGET_DIR);
  }
  const local = join(root, "src-tauri", "target");
  if (!roots.includes(local)) {
    roots.push(local);
  }
  return roots;
}

function debPath(version) {
  const found = [];
  for (const target of targetRoots()) {
    const dir = join(target, "release", "bundle", "deb");
    if (!existsSync(dir)) {
      continue;
    }
    for (const name of readdirSync(dir)) {
      if (name.endsWith(".deb") && name.includes(version)) {
        found.push(join(dir, name));
      }
    }
  }
  found.sort((left, right) => {
    const leftTime = existsSync(left) ? statMtime(left) : 0;
    const rightTime = existsSync(right) ? statMtime(right) : 0;
    return rightTime - leftTime;
  });
  return found[0] ?? "";
}

const tauri = JSON.parse(readFileSync(join(root, "src-tauri", "tauri.conf.json"), "utf8"));
const version = String(tauri.version || "");
if (!/^\d+\.\d+\.\d+$/.test(version)) {
  fail(`Unexpected app version in src-tauri/tauri.conf.json: ${version}`);
}

let deb = debPath(version);
if (!deb) {
  run("npm", ["run", "tauri", "--", "build", "--bundles", "deb"]);
  deb = debPath(version);
}
if (!deb) {
  fail(`No ${version} .deb to pack. The Flatpak is built from that package.`);
}

if (capture("flatpak", ["--version"]).trim() === "") {
  fail("flatpak is not installed.");
}

const userRemote = capture("flatpak", ["remotes", "--user"]);
if (!userRemote.includes("flathub")) {
  run("flatpak", ["remote-add", "--user", "--if-not-exists", "flathub", "https://dl.flathub.org/repo/flathub.flatpakrepo"]);
}

function installed(ref) {
  return ok("flatpak", ["info", "--user", ref]) || ok("flatpak", ["info", ref]);
}

const need = [];
if (!installed("org.flatpak.Builder")) {
  need.push("org.flatpak.Builder");
}
if (!installed("org.gnome.Sdk//51")) {
  need.push("org.gnome.Sdk//51");
}
if (!installed("org.gnome.Platform//51")) {
  need.push("org.gnome.Platform//51");
}
if (need.length > 0) {
  run("flatpak", ["install", "--user", "-y", "flathub", ...need]);
}

rmSync(stage, { recursive: true, force: true });
mkdirSync(stage, { recursive: true });
cpSync(deb, join(stage, "line.deb"));

const speech = join(root, "src-tauri", "speech");
const speechDest = join(stage, "speech");
if (existsSync(join(speech, "whisper-cli"))) {
  cpSync(speech, speechDest, { recursive: true, verbatimSymlinks: true });
} else {
  mkdirSync(speechDest);
}

// Keep the builder tree out of the repo. Flatpak's build root contains
// symlink loops, and Vite crashes if it stats one while tauri dev is running.
const work = join(homedir(), ".cache", "lineapp", "flatpak");
const outDir = join(root, "src-tauri", "target", "release", "bundle", "flatpak");
mkdirSync(outDir, { recursive: true });
const bundle = join(outDir, `LINE_${version}_amd64.flatpak`);
rmSync(bundle, { force: true });

mkdirSync(work, { recursive: true });
const builderEnv = { ...process.env };
delete builderEnv.CCACHE_DIR;
run("flatpak", [
  "run",
  "--filesystem=host",
  "org.flatpak.Builder",
  "--force-clean",
  "--disable-rofiles-fuse",
  "--user",
  "--install-deps-from=flathub",
  "--state-dir",
  join(work, "state"),
  "--repo",
  join(work, "repo"),
  join(work, "build"),
  join(flatpakDir, "dev.lineapp.desktop.yml"),
], builderEnv);

run("flatpak", [
  "build-bundle",
  "--runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo",
  join(work, "repo"),
  bundle,
  "dev.lineapp.desktop",
]);

if (process.env.CARGO_TARGET_DIR) {
  const cargoOut = join(process.env.CARGO_TARGET_DIR, "release", "bundle", "flatpak");
  if (cargoOut !== outDir) {
    mkdirSync(cargoOut, { recursive: true });
    cpSync(bundle, join(cargoOut, `LINE_${version}_amd64.flatpak`));
  }
}

console.log(bundle);
