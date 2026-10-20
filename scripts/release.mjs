// Cuts a release: bumps the version in Cargo.toml, Cargo.lock, package.json and
// src-tauri/tauri.conf.json on a `release-<version>` branch off origin/main, runs the core tests
// and the typecheck, opens and merges the "Version <version>" PR, then tags the merge commit
// `v<version>`, which starts the Installers workflow (a draft GitHub release). Run as the
// "Release" Action, or by hand; the Worktree it runs in must have no uncommitted changes.
//
//   node scripts/release.mjs [patch|minor|major|<x.y.z>]   (default: patch)
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const root = join(import.meta.dirname, "..");

// pnpm is a .cmd on Windows, which only runs through a shell; git, gh and cargo don't need one,
// and going without keeps arguments such as the commit message unquoted.
const run = (command, args, { capture = false, shell = false } = {}) =>
  execFileSync(command, args, {
    cwd: root,
    encoding: "utf8",
    stdio: capture ? ["ignore", "pipe", "inherit"] : "inherit",
    shell: shell && process.platform === "win32",
  })?.trim();
const git = (...args) => run("git", args, { capture: true });
const step = (text) => console.log(`\n▸ ${text}`);
const fail = (text) => {
  console.error(`\n✗ ${text}`);
  process.exit(1);
};

if (git("status", "--porcelain")) fail("This Worktree has uncommitted changes; commit or stash them first.");

step("Fetching origin");
run("git", ["fetch", "--tags", "origin"]);

const current = JSON.parse(git("show", "origin/main:package.json")).version;
const bump = process.argv[2] ?? "patch";
const next = nextVersion(current, bump);
const branch = `release-${next}`;
const tag = `v${next}`;
if (git("tag", "--list", tag)) fail(`${tag} already exists.`);
console.log(`${current} → ${next}`);

const startedOn = git("rev-parse", "--abbrev-ref", "HEAD");
const startedAt = git("rev-parse", "HEAD");
const remoteBranch = git("ls-remote", "--heads", "origin", branch);

// A release branch already pushed (a run that stopped after pushing) is picked up where it left off.
if (remoteBranch) {
  step(`Continuing the pushed ${branch}`);
  run("git", ["switch", "-C", branch, `origin/${branch}`]);
} else {
  step(`Bumping the version on ${branch}`);
  run("git", ["switch", "-C", branch, "origin/main"]);
  setVersion("Cargo.toml", /(\[workspace\.package\][^[]*?\nversion = ")[^"]+(")/);
  setVersion("package.json", /^(  "version": ")[^"]+(")/m);
  setVersion("src-tauri/tauri.conf.json", /^(  "version": ")[^"]+(")/m);
  run("cargo", ["update", "--workspace", "--quiet"]);
}

step("Testing");
run("cargo", ["test", "-p", "editor-core"]);
run("pnpm", ["typecheck"], { shell: true });

if (!remoteBranch) {
  step("Committing and opening the PR");
  run("git", ["commit", "--quiet", "--all", "--message", `Version ${next}`]);
  run("git", ["push", "--quiet", "--set-upstream", "origin", branch]);
}
const prState = run("gh", ["pr", "list", "--head", branch, "--state", "all", "--json", "state", "--jq", ".[0].state"], {
  capture: true,
});
if (!prState) {
  run("gh", ["pr", "create", "--base", "main", "--head", branch, "--title", `Version ${next}`, "--body", `Bumps the version to ${next}.`]);
}
if (prState !== "MERGED") {
  step("Merging the PR");
  run("gh", ["pr", "merge", branch, "--merge"]);
}
const mergeCommit = run("gh", ["pr", "view", branch, "--json", "mergeCommit", "--jq", ".mergeCommit.oid"], {
  capture: true,
});
if (!mergeCommit) fail(`The PR for ${branch} isn't merged.`);

step(`Tagging ${tag}`);
run("git", ["fetch", "--quiet", "origin"]);
run("git", ["tag", tag, mergeCommit]);
run("git", ["push", "--quiet", "origin", tag]);

run("git", startedOn === "HEAD" ? ["switch", "--quiet", "--detach", startedAt] : ["switch", "--quiet", startedOn]);
run("git", ["branch", "--quiet", "-D", branch]);

const repo = run("gh", ["repo", "view", "--json", "url", "--jq", ".url"], { capture: true });
console.log(`\n✓ ${tag} tagged. The installers build into a draft release: ${repo}/actions/workflows/release.yml`);

function nextVersion(version, bump) {
  if (/^\d+\.\d+\.\d+$/.test(bump)) return bump;
  const [major, minor, patch] = version.split(".").map(Number);
  if (bump === "patch") return `${major}.${minor}.${patch + 1}`;
  if (bump === "minor") return `${major}.${minor + 1}.0`;
  if (bump === "major") return `${major + 1}.0.0`;
  fail(`Unknown bump "${bump}": use patch, minor, major or a version such as 1.2.3.`);
}

function setVersion(file, pattern) {
  const path = join(root, file);
  const text = readFileSync(path, "utf8");
  if (!pattern.test(text)) fail(`No version found in ${file}.`);
  writeFileSync(path, text.replace(pattern, `$1${next}$2`));
}
