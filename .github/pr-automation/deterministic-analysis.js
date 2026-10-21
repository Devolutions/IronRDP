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
// Test markers, matched over whole texts so an attribute may span lines. `#[cfg(...)]` attributes are
// recognised separately, because their predicate needs parsing.
const RUST_TEST_MARKER = /#\[\s*(?:[\w:]+::)?test\b|#\[\s*rstest\b|#\[\s*case\s*\(|\bproptest!|\bexpect!\[/;
const CSHARP_TEST_MARKER = /\[\s*(?:Fact|Theory|Test|TestMethod|TestCase|InlineData)\b/;

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

function isTestSource(path) {
  return /\.(?:rs|cs)$/.test(path);
}

function isTestPath(file) {
  return pathsOf(file).some((path) =>
    (path.endsWith(".rs") && RUST_TEST_PATH.test(path)) || (path.endsWith(".cs") && CSHARP_TEST_PATH.test(path)));
}

// A `cfg` predicate selects test code when it names `test` other than as `not(test)`, such as
// `all(test, feature = "openh264-bundled")`. String values like `feature = "test"` do not count.
function isTestPredicate(predicate) {
  return /\btest\b/.test(predicate
    .replace(/"(?:[^"\\]|\\.)*"/g, "\"\"")
    .replace(/\bnot\s*\(\s*test\s*\)/g, ""));
}

// Offsets spanning each `#[cfg(<test predicate>)]` attribute in the text.
function testCfgAttributes(text) {
  const attributes = [];
  for (const match of text.matchAll(/#\[\s*cfg\s*\(/g)) {
    let depth = 1;
    let index = match.index + match[0].length;
    for (; index < text.length && depth > 0; index += 1) {
      if (text[index] === "\"") {
        for (index += 1; index < text.length && text[index] !== "\""; index += 1) {
          if (text[index] === "\\") index += 1;
        }
      } else if (text[index] === "(") depth += 1;
      else if (text[index] === ")") depth -= 1;
    }
    const close = /^\s*\]/.exec(text.slice(index, index + 64));
    if (depth !== 0 || !close) continue;
    if (isTestPredicate(text.slice(match.index + match[0].length, index - 1))) {
      attributes.push({ start: match.index, end: index + close[0].length });
    }
  }
  return attributes;
}

function hasTestMarker(text, path) {
  if (path.endsWith(".rs")) return RUST_TEST_MARKER.test(text) || testCfgAttributes(text).length > 0;
  return CSHARP_TEST_MARKER.test(text);
}

function addedText(patch) {
  return patch.split("\n")
    .filter((line) => line.startsWith("+") && !line.startsWith("+++"))
    .map((line) => line.slice(1))
    .join("\n");
}

// Locating an edit inside a test kept missing lexical forms, so the rule is coarse instead: a file
// that holds tests on either side of the change selects the review, even for a production-only edit.
// A file without a patch to inspect selects it too.
function isTestChangeWithoutBase(file) {
  if (!isTestSource(file.filename) || file.status === "removed") return false;
  return isTestPath(file) || typeof file.patch !== "string" || hasTestMarker(addedText(file.patch), file.filename);
}

function needsBaseRead(file) {
  return isTestSource(basePathOf(file)) && file.status !== "added" && file.status !== "removed";
}

function isTestChange(file, baseContents) {
  if (isTestChangeWithoutBase(file)) return true;
  if (!isTestSource(file.filename) || !needsBaseRead(file)) return false;
  const base = baseContents.get(basePathOf(file));
  return typeof base !== "string" || hasTestMarker(base, basePathOf(file));
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

// Pull request patches are three-dot diffs, so their old side is the merge base rather than the
// target branch tip, which may have moved since the branch point.
async function resolveMergeBase(github, { owner, repo, baseSha, headSha }) {
  if (!SHA.test(baseSha || "") || !SHA.test(headSha || "")) return null;
  try {
    const { data } = await github.rest.repos.compareCommitsWithBasehead({
      owner, repo, basehead: `${baseSha}...${headSha}`, per_page: 1,
    });
    const mergeBase = data?.merge_base_commit?.sha;
    return SHA.test(mergeBase || "") ? mergeBase : null;
  } catch {
    return null;
  }
}

// Base files are read only when nothing on the head side already selects the test specialist. A read
// that fails is recorded as unavailable, which selects it rather than skipping the review.
async function readBaseFiles(files, { repositoryRoot, mergeBase }) {
  const contents = new Map();
  if (files.some(isTestChangeWithoutBase)) return contents;
  const paths = [...new Set(files.filter(needsBaseRead).map(basePathOf))];
  if (paths.length === 0) return contents;
  const git = (args) => execFileSync("git", args, {
    cwd: repositoryRoot, encoding: "utf8", maxBuffer: 16 * 1024 * 1024, stdio: ["ignore", "pipe", "ignore"],
  });
  const sha = await mergeBase();
  let fetched = SHA.test(sha || "");
  if (fetched) {
    try {
      // `--depth` would turn a complete clone shallow, so only an already shallow checkout uses it.
      const shallow = git(["rev-parse", "--is-shallow-repository"]).trim() === "true";
      git(["fetch", "--no-tags", ...(shallow ? ["--depth=1"] : []), "origin", sha]);
    } catch {
      fetched = false;
    }
  }
  for (const path of paths) {
    let content = null;
    if (fetched) {
      try {
        content = git(["cat-file", "blob", `${sha}:${path}`]);
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
  github, owner, repo, prNumber, authorAssociation, labelerPath, baseSha, headSha, repositoryRoot = ".",
}) {
  try {
    const source = fs.readFileSync(labelerPath, "utf8");
    const files = await listPrFiles(github, { owner, repo, prNumber });
    const result = analyzeFiles(files, {
      labelerRules: parseLabelerRules(source), authorAssociation,
      intentPaths: listIntentFiles(repositoryRoot),
      baseContents: await readBaseFiles(files, {
        repositoryRoot, mergeBase: () => resolveMergeBase(github, { owner, repo, baseSha, headSha }),
      }),
    });
    return result.ok ? result : { ok: false, reason: result.reason };
  } catch {
    return { ok: false, reason: "deterministic analysis unavailable" };
  }
}

module.exports = {
  MAX_FILES, SIZE_LABELS, addedLinesByPath, analyzeFiles, deterministicAnalysis, globToRegExp,
  isGovernedByIntent, listIntentFiles, listPrFiles, parseLabelerRules, readBaseFiles, resolveMergeBase,
};
