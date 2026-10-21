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
// Attributes and macros that open a test region in the base revision of a source file.
const RUST_TEST_REGION = /#\[cfg\(test\)\]|#\[(?:[\w:]+::)?test\b|#\[rstest\b|\bproptest!/g;
const CSHARP_TEST_REGION = /\[(?:Fact|Theory|Test|TestMethod|TestCase)\b/g;
const SHA = /^[0-9a-f]{40}$/;

// A rename moves content out of its old path, so path rules apply to both names.
function pathsOf(file) {
  return typeof file.previous_filename === "string" ? [file.filename, file.previous_filename] : [file.filename];
}

function basePathOf(file) {
  return file.status === "renamed" && typeof file.previous_filename === "string"
    ? file.previous_filename
    : file.filename;
}

function changedPatchLines(file) {
  return (file.patch || "").split("\n")
    .filter((line) => /^[+-]/.test(line) && !/^(?:\+\+\+|---)/.test(line));
}

function isTestPathOrMarker(file) {
  const testPath = pathsOf(file).some((path) =>
    (path.endsWith(".rs") && RUST_TEST_PATH.test(path)) || (path.endsWith(".cs") && CSHARP_TEST_PATH.test(path)));
  if (testPath) return true;
  const marker = file.filename.endsWith(".rs") ? RUST_TEST_MARKER
    : file.filename.endsWith(".cs") ? CSHARP_TEST_MARKER
      : null;
  return marker !== null && changedPatchLines(file).some((line) => marker.test(line));
}

// An edit inside an existing test can change no marker line, so it is located in the base revision.
function needsBaseRead(file) {
  return /\.(?:rs|cs)$/.test(basePathOf(file)) && file.status !== "added";
}

// Offsets of braces, brackets, parentheses, and semicolons outside comments and literals.
function* structuralOffsets(text, from) {
  for (let index = from; index < text.length; index += 1) {
    const char = text[index];
    const next = text[index + 1];
    if (char === "/" && next === "/") {
      index = text.indexOf("\n", index);
      if (index < 0) return;
    } else if (char === "/" && next === "*") {
      index = text.indexOf("*/", index + 2);
      if (index < 0) return;
      index += 1;
    } else if (char === "r" && !/\w/.test(text[index - 1] || "") && /^r#*"/.test(text.slice(index, index + 130))) {
      const hashes = /^r(#*)"/.exec(text.slice(index, index + 130))[1];
      index = text.indexOf(`"${hashes}`, index + hashes.length + 2);
      if (index < 0) return;
      index += hashes.length;
    } else if (char === "\"") {
      for (index += 1; index < text.length && text[index] !== "\""; index += 1) {
        if (text[index] === "\\") index += 1;
      }
    } else if (char === "'") {
      // A lifetime has no closing quote; an escaped character literal ends at the next quote.
      if (next === "\\") index = Math.max(index, text.indexOf("'", index + 3));
      else if (text[index + 2] === "'") index += 2;
    } else if ("{};()[]".includes(char)) {
      yield index;
    }
  }
}

// Each region runs from a test attribute to the end of the item it annotates. When the item's end
// cannot be found, the region runs to the end of the file so the change is still reviewed.
function testRegions(text, pattern) {
  const lineStarts = [0];
  for (let index = text.indexOf("\n"); index >= 0; index = text.indexOf("\n", index + 1)) {
    lineStarts.push(index + 1);
  }
  const lineAt = (offset) => {
    let low = 0;
    let high = lineStarts.length - 1;
    while (low < high) {
      const middle = Math.ceil((low + high) / 2);
      if (lineStarts[middle] <= offset) low = middle; else high = middle - 1;
    }
    return low + 1;
  };
  const regions = [];
  for (const match of text.matchAll(pattern)) {
    let end = Infinity;
    let depth = 0;
    // Parentheses and brackets hide semicolons such as the one in `[u8; 4]`; the attribute's own
    // closing bracket may be the first one seen, so the count never drops below zero.
    let nesting = 0;
    for (const offset of structuralOffsets(text, match.index + match[0].length)) {
      const char = text[offset];
      if (char === "(" || char === "[") {
        nesting += 1;
        continue;
      }
      if (char === ")" || char === "]") {
        nesting = Math.max(0, nesting - 1);
        continue;
      }
      if (char === "{") {
        depth += 1;
        continue;
      }
      if (char === "}" && depth > 1) {
        depth -= 1;
        continue;
      }
      // The first semicolon outside braces ends a braceless item; the outermost closing brace ends the rest.
      if ((char === ";" && depth === 0 && nesting === 0) || (char === "}" && depth === 1)) {
        end = lineAt(offset);
        break;
      }
    }
    regions.push([lineAt(match.index), end]);
  }
  return regions;
}

// Removed lines carry their base line number; an added line sits between two base lines.
function touchesTestRegion(file, base) {
  if (typeof base !== "string") return true;
  const regions = testRegions(base, basePathOf(file).endsWith(".rs") ? RUST_TEST_REGION : CSHARP_TEST_REGION);
  if (regions.length === 0) return false;
  if (!file.patch) return true;
  let line = null;
  for (const patchLine of file.patch.split("\n")) {
    const hunk = /^@@ -(\d+)(?:,(\d+))? \+\d+(?:,\d+)? @@/.exec(patchLine);
    if (hunk) {
      // An empty old side names the line the insertion follows.
      line = Number(hunk[1]) + (hunk[2] === "0" ? 1 : 0);
    } else if (line === null || patchLine.startsWith("\\")) {
      continue;
    } else if (patchLine.startsWith("-")) {
      const removed = line;
      if (regions.some(([start, end]) => start <= removed && removed <= end)) return true;
      line += 1;
    } else if (patchLine.startsWith("+")) {
      const after = line - 1;
      if (regions.some(([start, end]) => start <= after && line <= end)) return true;
    } else {
      line += 1;
    }
  }
  return false;
}

function isTestChange(file, baseContents) {
  return isTestPathOrMarker(file) ||
    (needsBaseRead(file) && touchesTestRegion(file, baseContents.get(basePathOf(file))));
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

function diffReviewers(files, intentPaths, baseContents) {
  const present = files.filter((file) => file.status !== "removed");
  const governing = [...new Set([
    ...intentPaths,
    ...present.map((file) => file.filename).filter((path) => INTENT_FILE.test(path)),
  ])];
  const intentEdited = (file) => file.status === "renamed"
    ? pathsOf(file).some((path) => INTENT_FILE.test(path))
    : file.status !== "removed" && INTENT_FILE.test(file.filename);
  const selected = [];
  if (files.some((file) => pathsOf(file).some((path) => isGovernedByIntent(path, governing)))) {
    selected.push("intent-drift");
  }
  if (files.some(intentEdited)) selected.push("intent-review");
  if (files.some((file) => isTestChange(file, baseContents))) selected.push("test");
  if (files.some((file) => file.additions > 0 && pathsOf(file).some((path) => DEPENDENCY_MANIFEST.test(path)))) {
    selected.push("dependency");
  }
  if (files.some((file) => file.filename.endsWith(".rs") && file.additions > 0)) selected.push("rust-style");
  return selected;
}

// Base files are read only when no path or marker already selects the test specialist. A read that
// fails is recorded as unavailable, which selects it rather than skipping the review.
function readBaseFiles(files, { repositoryRoot, baseSha }) {
  const contents = new Map();
  if (files.some(isTestPathOrMarker)) return contents;
  const paths = [...new Set(files.filter(needsBaseRead).map(basePathOf))];
  if (paths.length === 0) return contents;
  const git = (args) => execFileSync("git", args, {
    cwd: repositoryRoot, encoding: "utf8", maxBuffer: 16 * 1024 * 1024, stdio: ["ignore", "pipe", "ignore"],
  });
  let fetched = SHA.test(baseSha || "");
  if (fetched) {
    try {
      git(["fetch", "--no-tags", "--depth=1", "origin", baseSha]);
    } catch {
      fetched = false;
    }
  }
  for (const path of paths) {
    let content = null;
    if (fetched) {
      try {
        content = git(["cat-file", "blob", `${baseSha}:${path}`]);
      } catch {
        content = null;
      }
    }
    contents.set(path, content);
  }
  return contents;
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

function analyzeFiles(files, {
  labelerRules, authorAssociation, intentPaths = [], baseContents = new Map(),
} = {}) {
  if (!Array.isArray(files) || files.length > MAX_FILES) return { ok: false, reason: "too many files" };
  if (!labelerRules || typeof labelerRules !== "object") return { ok: false, reason: "invalid configured rules" };
  if (!Array.isArray(intentPaths) || intentPaths.some((path) => typeof path !== "string")) {
    return { ok: false, reason: "invalid intent paths" };
  }
  if (!(baseContents instanceof Map)) return { ok: false, reason: "invalid base contents" };
  for (const file of files) {
    if (!file || typeof file.filename !== "string" || file.filename.length > MAX_FILENAME ||
        !Number.isSafeInteger(file.additions) || file.additions < 0 ||
        !Number.isSafeInteger(file.deletions) || file.deletions < 0 ||
        (file.previous_filename !== undefined &&
          (typeof file.previous_filename !== "string" || file.previous_filename.length > MAX_FILENAME))) {
      return { ok: false, reason: "invalid file metadata" };
    }
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
    reviewers: diffReviewers(files, intentPaths, baseContents),
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
  github, owner, repo, prNumber, authorAssociation, labelerPath, baseSha, repositoryRoot = ".",
}) {
  try {
    const source = fs.readFileSync(labelerPath, "utf8");
    const files = await listPrFiles(github, { owner, repo, prNumber });
    const result = analyzeFiles(files, {
      labelerRules: parseLabelerRules(source), authorAssociation,
      intentPaths: listIntentFiles(repositoryRoot),
      baseContents: readBaseFiles(files, { repositoryRoot, baseSha }),
    });
    return result.ok ? result : { ok: false, reason: result.reason };
  } catch {
    return { ok: false, reason: "deterministic analysis unavailable" };
  }
}

module.exports = {
  MAX_FILES, SIZE_LABELS, addedLinesByPath, analyzeFiles, deterministicAnalysis, globToRegExp,
  isGovernedByIntent, listIntentFiles, listPrFiles, parseLabelerRules, readBaseFiles,
};
