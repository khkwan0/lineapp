import { spawnSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { homedir, platform } from "node:os";
import { delimiter, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const cargoBin = join(homedir(), ".cargo", "bin");
if (existsSync(cargoBin)) {
  process.env.PATH = `${cargoBin}${delimiter}${process.env.PATH ?? ""}`;
}

function fail(message) {
  console.error(message);
  process.exit(1);
}

function run(command, args) {
  const result = spawnSync(command, args, { cwd: root, stdio: "inherit" });
  if (result.error) {
    fail(result.error.message);
  }
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

function capture(command, args) {
  const result = spawnSync(command, args, { cwd: root, encoding: "utf8" });
  if (result.error) {
    fail(result.error.message);
  }
  if (result.status !== 0) {
    fail((result.stderr || result.stdout || `${command} failed`).trim());
  }
  return result.stdout;
}

function changedPaths() {
  return capture("git", ["status", "--porcelain"])
    .split("\n")
    .map((line) => line.slice(3).trim())
    .filter((path) => path.length > 0);
}

function requireReleaseTree() {
  const unexpected = changedPaths().filter((path) => path !== "build-info.json");
  if (unexpected.length > 0) {
    fail(`Commit or stash these changes before releasing:\n${unexpected.join("\n")}`);
  }
}

function bundleFile(dir, version, extension) {
  const names = readdirSync(dir).filter((name) => name.endsWith(extension) && name.includes(version));
  if (names.length !== 1) {
    fail(`Expected one ${extension} for ${version} in ${dir}, found ${names.length}.`);
  }
  return join(dir, names[0]);
}

function builtAt(iso) {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) {
    return iso;
  }
  return date.toLocaleString("en-US", {
    month: "short",
    day: "numeric",
    year: "numeric",
    hour: "numeric",
    minute: "2-digit",
    hour12: true,
    timeZone: "Asia/Bangkok",
  });
}

const gh = spawnSync("gh", ["--version"], { encoding: "utf8" });
if (gh.error || gh.status !== 0) {
  fail("GitHub CLI (gh) is required. Install it, then run: gh auth login");
}

run("gh", ["auth", "status"]);

const branch = capture("git", ["branch", "--show-current"]).trim();
if (!branch) {
  fail("Check out a branch before releasing.");
}

requireReleaseTree();

const tauri = JSON.parse(readFileSync(join(root, "src-tauri", "tauri.conf.json"), "utf8"));
const version = String(tauri.version || "");
if (!/^\d+\.\d+\.\d+$/.test(version)) {
  fail(`Unexpected app version in src-tauri/tauri.conf.json: ${version}`);
}

const current = JSON.parse(readFileSync(join(root, "build-info.json"), "utf8"));
const nextNumber = Number(current.number) + 1;
if (!Number.isInteger(nextNumber) || nextNumber < 1) {
  fail("build-info.json does not contain a usable build number.");
}

const tag = `v${version}-build.${nextNumber}`;
const localTag = spawnSync("git", ["rev-parse", "-q", "--verify", `refs/tags/${tag}`], { cwd: root, encoding: "utf8" });
if (localTag.status === 0) {
  fail(`Tag ${tag} already exists.`);
}
const remoteTag = capture("git", ["ls-remote", "--tags", "origin", `refs/tags/${tag}`]).trim();
if (remoteTag) {
  fail(`Tag ${tag} already exists on origin.`);
}

const host = platform();
const bundleDir = join(root, "src-tauri", "target", "release", "bundle");
let bundleArgs;
let packages;
if (host === "darwin") {
  bundleArgs = "app,dmg";
  packages = () => [bundleFile(join(bundleDir, "dmg"), version, ".dmg")];
} else if (host === "linux") {
  bundleArgs = "deb,appimage";
  packages = () => [
    bundleFile(join(bundleDir, "deb"), version, ".deb"),
    bundleFile(join(bundleDir, "appimage"), version, ".AppImage"),
  ];
} else {
  fail("Release packages are built on Linux or macOS.");
}

run("npm", ["run", "tauri", "--", "build", "--bundles", bundleArgs]);

requireReleaseTree();
const info = JSON.parse(readFileSync(join(root, "build-info.json"), "utf8"));
if (info.number !== nextNumber) {
  fail(`Expected build ${nextNumber} after the production build, found ${info.number}.`);
}

const built = packages();

run("git", ["add", "--", "build-info.json"]);
run("git", ["commit", "-m", `Record production build ${info.number}.`]);
run("git", ["tag", "-a", tag, "-m", `LINE ${version} build ${info.number}`]);
run("git", ["push", "origin", "HEAD"]);
run("git", ["push", "origin", `refs/tags/${tag}`]);
run("gh", [
  "release",
  "create",
  tag,
  ...built,
  "--title",
  `LINE ${version} (build ${info.number})`,
  "--notes",
  `Build ${info.number}\nBuilt ${builtAt(info.time)}`,
]);

console.log(`Published ${tag}`);
