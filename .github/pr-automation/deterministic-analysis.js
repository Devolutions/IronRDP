"use strict";

const fs = require("node:fs");
const { execFileSync } = require("node:child_process");
const { isDocumentationPath } = require("./validate-classifier");

const MAX_FILES = 3000;
const MAX_FILENAME = 500;
const SIZE_LABELS = ["size/XS", "size/S", "size/M", "size/L", "size/XL", "size/XXL"];
const SOURCE_FILE = /\.(?:rs|cs|[cm]?js|jsx|[cm]?ts|tsx|svelte|ya?ml|toml)$/i;

// Diff facts that select the conditional specialists, mirroring the `code-review` skill triggers.
const DEPENDENCY_MANIFEST = /(?:^|\/)(?:Cargo\.toml|Cargo\.lock|package\.json|package-lock\.json|[^/]+\.csproj)$/;
const INTENT_FILE = /(?:^|\/)(?:INTENT[^/]*|[^/]+\.intent[^/]*)\.md$/;
const RUST_TEST_PATH = /(?:^|\/)tests\/|^crates\/ironrdp-testsuite-[^/]+\/|(?:^|\/)(?:[^/]+_)?tests?\.rs$/;
const CSHARP_TEST_PATH = /(?:^|\/)(?:tests|[^/]*\.Tests)\//i;
const RUST_TEST_MARKER = /#\[(?:[\w:]+::)?test\b|#\[cfg\(test\)\]|#\[rstest\b|\bproptest!|\bexpect!\[/;
const CSHARP_TEST_MARKER = /\[(?:Fact|Theory|Test|TestMethod|TestCase)\b/;

function changedPatchLines(file) {
  return (file.patch || "").split("\n")
    .filter((line) => /^[+-]/.test(line) && !/^(?:\+\+\+|---)/.test(line));
}

function isTestChange(file) {
  const { filename } = file;
  if (filename.endsWith(".rs")) {
    return RUST_TEST_PATH.test(filename) ||
      changedPatchLines(file).some((line) => RUST_TEST_MARKER.test(line));
  }
  if (filename.endsWith(".cs")) {
    return CSHARP_TEST_PATH.test(filename) ||
      changedPatchLines(file).some((line) => CSHARP_TEST_MARKER.test(line));
  }
  return false;
}

// `intent-files` placement: `D/INTENT*.md` governs everything under `D`, and `D/<name>.intent*.md`
// governs the module `D/<name>` or `D/<name>.<extension>` beside it.
function isGovernedByIntent(path, intentPaths) {
  return intentPaths.some((intentPath) => {
    const slash = intentPath.lastIndexOf("/");
    const directory = intentPath.slice(0, slash + 1);
    const name = intentPath.slice(slash + 1);
    if (/^INTENT[^/]*\.md$/.test(name)) return path.startsWith(directory);
    const module = /^(.+?)\.intent[^/]*\.md$/.exec(name)?.[1];
    if (!module) return false;
    const target = `${directory}${module}`;
    return path === target || path.startsWith(`${target}.`);
  });
}

function diffReviewers(files, intentPaths) {
  const present = files.filter((file) => file.status !== "removed");
  const governing = [...new Set([
    ...intentPaths,
    ...present.map((file) => file.filename).filter((path) => INTENT_FILE.test(path)),
  ])];
  const selected = [];
  if (files.some((file) => isGovernedByIntent(file.filename, governing))) selected.push("intent-drift");
  if (present.some((file) => INTENT_FILE.test(file.filename))) selected.push("intent-review");
  if (files.some(isTestChange)) selected.push("test");
  if (files.some((file) => DEPENDENCY_MANIFEST.test(file.filename) && file.additions > 0)) {
    selected.push("dependency");
  }
  if (files.some((file) => file.filename.endsWith(".rs") && file.additions > 0)) selected.push("rust-style");
  return selected;
}

function listIntentFiles(repositoryRoot) {
  return execFileSync("git", ["ls-files", "-z"], {
    cwd: repositoryRoot, encoding: "utf8", maxBuffer: 64 * 1024 * 1024,
  }).split("\0").filter((path) => INTENT_FILE.test(path));
}

function globToRegExp(pattern) {
  let output = "^";
  for (let index = 0; index < pattern.length; index += 1) {
    const char = pattern[index];
    if (char === "*" && pattern[index + 1] === "*") {
      index += 1;
      if (pattern[index + 1] === "/") { output += "(?:.*/)?"; index += 1; } else output += ".*";
    } else if (char === "*") output += "[^/]*";
    else if (char === "?") output += "[^/]";
    else if (char === "{") {
      const end = pattern.indexOf("}", index);
      if (end < 0) throw new Error("unclosed glob brace");
      output += `(?:${pattern.slice(index + 1, end).split(",").map((part) =>
        globToRegExp(part).source.slice(1, -1)).join("|")})`;
      index = end;
    } else output += char.replace(/[\\^$+.[\]()|]/g, "\\$&");
  }
  return new RegExp(`${output}$`, "i");
}

function parseLabelerRules(source) {
  const rules = {};
  let label = null;
  for (const line of source.replace(/\r\n?/g, "\n").split("\n")) {
    const topLevel = /^(?:"([^"]+)"|'([^']+)'|([^:\s][^:]*)):\s*$/.exec(line);
    if (topLevel) { label = topLevel[1] ?? topLevel[2] ?? topLevel[3]; rules[label] = []; continue; }
    if (!label) continue;
    const inline = /^\s*-\s*any-glob-to-any-file:\s*["']?(.+?)["']?\s*$/.exec(line);
    const item = /^\s*-\s*["'](.+)["']\s*$/.exec(line);
    if (inline) rules[label].push(globToRegExp(inline[1]));
    else if (item) rules[label].push(globToRegExp(item[1]));
  }
  return rules;
}

function sourceSizeLabel(files) {
  const lines = files.reduce((total, file) => SOURCE_FILE.test(file.filename) ?
    total + file.additions + file.deletions : total, 0);
  const lineSize = lines < 50 ? 0 : lines < 200 ? 1 : lines < 450 ? 2 :
    lines < 900 ? 3 : lines < 1300 ? 4 : 5;
  const fileSize = files.length < 3 ? 0 : files.length < 6 ? 1 : files.length < 11 ? 2 :
    files.length < 21 ? 3 : files.length < 50 ? 4 : 5;
  return {
    changedLines: lines,
    touchedFiles: files.length,
    label: SIZE_LABELS[Math.max(lineSize, fileSize)],
  };
}

function analyzeFiles(files, { labelerRules, authorAssociation, intentPaths = [] } = {}) {
  if (!Array.isArray(files) || files.length > MAX_FILES) return { ok: false, reason: "too many files" };
  if (!labelerRules || typeof labelerRules !== "object") return { ok: false, reason: "invalid configured rules" };
  if (!Array.isArray(intentPaths) || intentPaths.some((path) => typeof path !== "string")) {
    return { ok: false, reason: "invalid intent paths" };
  }
  for (const file of files) {
    if (!file || typeof file.filename !== "string" || file.filename.length > MAX_FILENAME ||
        !Number.isSafeInteger(file.additions) || file.additions < 0 ||
        !Number.isSafeInteger(file.deletions) || file.deletions < 0) return { ok: false, reason: "invalid file metadata" };
  }
  const pathLabels = Object.entries(labelerRules).filter(([, patterns]) =>
    Array.isArray(patterns) && files.some((file) => patterns.some((pattern) => pattern.test(file.filename)))).map(([label]) => label);
  if (pathLabels.length > 100 || Object.keys(labelerRules).length > 100) return { ok: false, reason: "too many configured labels" };
  const size = sourceSizeLabel(files);
  return {
    ok: true, pathLabels, ownedPathLabels: Object.keys(labelerRules), sizeLabel: size.label,
    sizeLabels: SIZE_LABELS, changedLines: size.changedLines, touchedFiles: size.touchedFiles,
    firstTime: ["FIRST_TIME_CONTRIBUTOR", "FIRST_TIMER"].includes(authorAssociation),
    documentationOnlyPaths: files.every((file) => isDocumentationPath(file.filename)),
    reviewers: diffReviewers(files, intentPaths),
  };
}

// Maps each file to the head-side line numbers it adds, derived from the unified diff hunks. These
// are the only lines an inline review comment may target.
function addedLinesByPath(files) {
  const added = {};
  for (const file of files) {
    const lines = [];
    let line = null;
    for (const patchLine of (file.patch || "").split("\n")) {
      const hunk = /^@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(patchLine);
      if (hunk) line = Number(hunk[1]);
      else if (line === null || patchLine.startsWith("\\")) continue;
      else if (patchLine.startsWith("+++")) continue;
      else if (patchLine.startsWith("+")) lines.push(line++);
      else if (!patchLine.startsWith("-")) line += 1;
    }
    added[file.filename] = lines;
  }
  return added;
}

async function listPrFiles(github, { owner, repo, prNumber }) {
  const files = [];
  for await (const response of github.paginate.iterator(github.rest.pulls.listFiles, {
    owner, repo, pull_number: prNumber, per_page: 100,
  })) {
    files.push(...response.data);
    if (files.length > MAX_FILES) throw new Error("too many files");
  }
  return files;
}

async function deterministicAnalysis({
  github, owner, repo, prNumber, authorAssociation, labelerPath, repositoryRoot = ".",
}) {
  try {
    const source = fs.readFileSync(labelerPath, "utf8");
    const result = analyzeFiles(await listPrFiles(github, { owner, repo, prNumber }), {
      labelerRules: parseLabelerRules(source), authorAssociation,
      intentPaths: listIntentFiles(repositoryRoot),
    });
    return result.ok ? result : { ok: false, reason: result.reason };
  } catch {
    return { ok: false, reason: "deterministic analysis unavailable" };
  }
}

module.exports = {
  MAX_FILES, SIZE_LABELS, addedLinesByPath, analyzeFiles, deterministicAnalysis, globToRegExp,
  isGovernedByIntent, listIntentFiles, listPrFiles, parseLabelerRules,
};
