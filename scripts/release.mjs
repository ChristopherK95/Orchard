// Pushes a new release: bumps the version on a `release-X.Y.Z` branch off origin/main, merges its
// PR, tags the merge commit `vX.Y.Z` (which starts the Installers workflow), waits for the
// installers and publishes the draft release with the merged PRs' titles as its notes. Run as the
// "Release" Action or by hand, in any Worktree with no uncommitted changes; a run that stopped
// after pushing the release branch picks it up where it left off.
//
//   pnpm release [patch|minor|major|X.Y.Z]    (default: patch)
//
// Needs `gh` logged in with push and merge rights.
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const root = join(import.meta.dirname, "..");

// pnpm is a .cmd shim on Windows, which only runs through a shell.
const run = (cmd, args, opts = {}) =>
  execFileSync(cmd, args, { cwd: root, encoding: "utf8", shell: cmd === "pnpm", ...opts });
const out = (cmd, ...args) => run(cmd, args).trim();
const step = (cmd, ...args) => run(cmd, args, { stdio: "inherit" });
const fail = (message) => {
  console.error(`release: ${message}`);
  process.exit(1);
};
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

const parse = (v) => v.split(".").map(Number);
const newer = (a, b) => {
  const [x, y] = [parse(a), parse(b)];
  for (let i = 0; i < 3; i++) if (x[i] !== y[i]) return x[i] > y[i];
  return false;
};

// Preflight. The release is cut from origin/main, so this Worktree can be on any branch.
if (out("git", "status", "--porcelain")) fail("the working tree isn't clean");
step("git", "fetch", "--tags", "origin");

const workspaceVersion = /(\[workspace\.package\][^[]*?\nversion = ")([^"]+)(")/;
const current = out("git", "show", "origin/main:Cargo.toml").match(workspaceVersion)?.[2];
if (!current) fail("no [workspace.package] version in Cargo.toml");
const latestTag = out("git", "tag", "--list", "v*", "--sort=-v:refname").split("\n")[0];
if (latestTag !== `v${current}`) fail(`Cargo.toml says ${current} but the newest tag is ${latestTag}`);
if (!out("git", "log", "--oneline", `${latestTag}..origin/main`)) fail(`nothing on main since ${latestTag}`);

const bump = process.argv[2] ?? "patch";
let next;
if (/^\d+\.\d+\.\d+$/.test(bump)) {
  if (!newer(bump, current)) fail(`${bump} isn't newer than ${current}`);
  next = bump;
} else {
  const part = ["major", "minor", "patch"].indexOf(bump);
  if (part < 0) fail(`expected patch, minor, major or X.Y.Z, got "${bump}"`);
  next = parse(current).map((n, i) => (i < part ? n : i === part ? n + 1 : 0)).join(".");
}
const tag = `v${next}`;
if (out("git", "ls-remote", "--tags", "origin", tag)) fail(`${tag} already exists on origin`);
console.log(`\nReleasing ${current} → ${next}\n`);

const startedOn = out("git", "rev-parse", "--abbrev-ref", "HEAD");
const startedAt = out("git", "rev-parse", "HEAD");
const branch = `release-${next}`;
const pushed = out("git", "ls-remote", "--heads", "origin", branch);

if (pushed) {
  console.log(`Continuing the pushed ${branch}`);
  step("git", "switch", "-C", branch, `origin/${branch}`);
} else {
  // Bump.
  step("git", "switch", "-C", branch, "origin/main");
  const replace = (file, pattern) => {
    const path = join(root, file);
    const text = readFileSync(path, "utf8");
    const bumped = text.replace(pattern, `$1${next}$3`);
    if (bumped === text) fail(`couldn't find the version in ${file}`);
    writeFileSync(path, bumped);
  };
  replace("Cargo.toml", workspaceVersion);
  replace("package.json", /^(  "version": ")([^"]+)(")/m);
  replace("src-tauri/tauri.conf.json", /^(  "version": ")([^"]+)(")/m);
  // Refreshes only the workspace members' entries in Cargo.lock.
  step("cargo", "update", "--workspace");
  const changed = out("git", "diff", "--name-only").split("\n").sort().join(" ");
  const expected = "Cargo.lock Cargo.toml package.json src-tauri/tauri.conf.json";
  if (changed !== expected) fail(`expected only ${expected} to change, got ${changed}`);
}

step("cargo", "test", "-p", "editor-core");
step("pnpm", "typecheck");

// Release PR. `gh pr merge --delete-branch` would switch this Worktree to main, which another
// Worktree may have checked out, so the branch is deleted after tagging instead.
if (!pushed) {
  step("git", "commit", "-am", `Version ${next}`);
  step("git", "push", "-u", "origin", branch);
}
const prState = out("gh", "pr", "list", "--head", branch, "--state", "all", "--json", "state", "--jq", ".[0].state");
if (!prState) {
  step("gh", "pr", "create", "--base", "main", "--head", branch, "--title", `Version ${next}`, "--body", `Bumps the version to ${next}.`);
}
if (prState !== "MERGED") step("gh", "pr", "merge", branch, "--merge");
const mergeCommit = out("gh", "pr", "view", branch, "--json", "mergeCommit", "--jq", ".mergeCommit.oid");
if (!mergeCommit) fail(`the PR for ${branch} isn't merged`);

// Tag the merge commit; pushing the tag starts the Installers workflow.
step("git", "fetch", "origin");
step("git", "tag", tag, mergeCommit);
step("git", "push", "origin", tag);

run("git", startedOn === "HEAD" ? ["switch", "--detach", startedAt] : ["switch", startedOn], { stdio: "inherit" });
step("git", "branch", "-D", branch);
step("git", "push", "origin", "--delete", branch);

// Wait for the installers. The run takes a few seconds to show up after the push.
let runId = "";
for (let i = 0; i < 30 && !runId; i++) {
  await sleep(5000);
  runId = out("gh", "run", "list", "--workflow", "release.yml", "--branch", tag, "--limit", "1",
    "--json", "databaseId", "--jq", ".[0].databaseId // empty");
}
if (!runId) fail(`the Installers run for ${tag} didn't start; check GitHub Actions`);
try {
  step("gh", "run", "watch", runId, "--exit-status");
} catch {
  fail(`the Installers run failed (gh run view ${runId} --log-failed); the draft release is left as is`);
}

const assets = out("gh", "release", "view", tag, "--json", "assets", "--jq", ".assets[].name");
for (const ext of [".exe", ".msi", ".deb", ".rpm", ".AppImage"]) {
  if (!assets.split("\n").some((name) => name.endsWith(ext))) fail(`the draft release has no ${ext}`);
}

// Notes: one bullet per change on main since the last release — a merged PR's title (the first
// line of its merge commit's body), or a direct commit's subject — leaving out version bumps.
const log = out("git", "log", "--first-parent", "--reverse", "--format=%P%x1f%s%x1f%b%x1e", `${latestTag}..${tag}`);
const notes = log
  .split("\x1e")
  .map((entry) => entry.trim())
  .filter(Boolean)
  .map((entry) => {
    const [parents, subject, body] = entry.split("\x1f");
    return parents.includes(" ") ? body.split("\n")[0].trim() || subject : subject;
  })
  .filter((line) => !/^Version \d+\.\d+\.\d+$/.test(line))
  .map((line) => `- ${line}`)
  .join("\n");

step("gh", "release", "edit", tag, "--draft=false", "--latest", "--notes", notes);
const url = out("gh", "release", "view", tag, "--json", "url", "--jq", ".url");
console.log(`\nReleased Orchard ${tag}: ${url}`);
