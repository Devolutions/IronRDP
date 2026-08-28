"use strict";

const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const assert = require("node:assert/strict");

const {
  MAX_LIST_ENTRIES, MAX_RECURSION_DEPTH, MAX_SEARCH_RESULTS, MAX_SOURCE_FILE_BYTES,
  MAX_TOOL_RESULT_BYTES,
} = require("../src/limits");
const { WorkspaceSandbox, boundJson, normalizeRepositoryPath } = require("../src/sandbox");
const { scratchWorkspace, write } = require("./helpers");

function fixture() {
  const workspace = scratchWorkspace();
  write(workspace.directory, "root/a.txt", "alpha\nneedle here\nomega\n");
  write(workspace.directory, "root/nested/b.txt", "another needle\n");
  write(workspace.directory, "single.txt", "single\n");
  return {
    ...workspace,
    sandbox: new WorkspaceSandbox(workspace.directory, {
      allowedRoots: ["root"],
      allowedFiles: ["single.txt"],
    }),
  };
}

test("read_file enforces capabilities and bounded line ranges", () => {
  const current = fixture();
  try {
    assert.deepEqual(JSON.parse(current.sandbox.readFile({
      path: "root/a.txt", start_line: 2, end_line: 3,
    })), {
      ok: true,
      path: "root/a.txt",
      start_line: 2,
      end_line: 3,
      truncated: true,
      content: "2: needle here\n3: omega",
    });
    assert.match(current.sandbox.readFile({ path: "single.txt" }), /1: single/);
    assert.throws(() => current.sandbox.readFile({ path: "outside.txt" }), /path is unavailable/);
    write(current.directory, "other.txt", "denied");
    assert.throws(() => current.sandbox.readFile({ path: "other.txt" }), /outside allowed capabilities/);
    assert.throws(
      () => current.sandbox.readFile({ path: "root/a.txt", start_line: 2, end_line: 502 }),
      /invalid line range/,
    );
  } finally {
    current.cleanup();
  }
});

test("repository paths reject absolute, traversal, control, and git paths", () => {
  for (const invalid of [
    "/etc/passwd", "../secret", "root/../secret", "root//a", "root/./a", ".git/config",
    "root/.GIT/config", "C:/Windows/System32", "root\\a.txt", "root/\0bad",
  ]) {
    assert.throws(() => normalizeRepositoryPath(invalid), /invalid path/, invalid);
  }
  assert.equal(normalizeRepositoryPath("root/nested/a.txt"), "root/nested/a.txt");
});

test("sandbox rejects symlinks and junctions before realpath access", (t) => {
  const current = fixture();
  const external = scratchWorkspace();
  try {
    write(external.directory, "secret.txt", "secret");
    const link = path.join(current.directory, "root", "escape");
    try {
      fs.symlinkSync(external.directory, link, process.platform === "win32" ? "junction" : "dir");
    } catch (error) {
      t.skip(`symlink creation unavailable: ${error.code}`);
      return;
    }
    assert.throws(
      () => current.sandbox.readFile({ path: "root/escape/secret.txt" }),
      /symbolic links and junctions are not allowed/,
    );
    assert.doesNotMatch(current.sandbox.listFiles({ path: "root", recursive: true }), /secret\.txt/);
  } finally {
    current.cleanup();
    external.cleanup();
  }
});

test("sandbox rejects binary, invalid UTF-8, oversized, overlong-line, and wrong-type reads", () => {
  const current = fixture();
  try {
    write(current.directory, "root/binary.txt", Buffer.from([65, 0, 66]));
    write(current.directory, "root/invalid.txt", Buffer.from([0xc3, 0x28]));
    write(current.directory, "root/large.txt", Buffer.alloc(MAX_SOURCE_FILE_BYTES + 1, 65));
    write(current.directory, "root/line.txt", "x".repeat(8 * 1024 + 1));
    fs.mkdirSync(path.join(current.directory, "root", "directory"));
    assert.throws(() => current.sandbox.readFile({ path: "root/binary.txt" }), /binary/);
    assert.throws(() => current.sandbox.readFile({ path: "root/invalid.txt" }), /UTF-8/);
    assert.throws(() => current.sandbox.readFile({ path: "root/large.txt" }), /byte limit/);
    assert.throws(() => current.sandbox.readFile({ path: "root/line.txt" }), /line exceeds/);
    assert.throws(() => current.sandbox.readFile({ path: "root/directory" }), /regular file/);
  } finally {
    current.cleanup();
  }
});

test("list_files is deterministic, bounded by recursion, and requires an allowed root", () => {
  const current = fixture();
  try {
    let relative = "root/deep";
    for (let depth = 0; depth < MAX_RECURSION_DEPTH + 3; depth++) {
      write(current.directory, `${relative}/level-${depth}.txt`, `${depth}`);
      relative += `/d${depth}`;
    }
    const listed = JSON.parse(current.sandbox.listFiles({ path: "root", recursive: true }));
    assert.equal(listed.entries[0].path, "root/a.txt");
    assert.equal(listed.entries.some((entry) => entry.path.includes(`d${MAX_RECURSION_DEPTH}`)), false);
    assert.throws(
      () => current.sandbox.listFiles({ path: "root/a.txt" }),
      /directory/,
    );
    assert.throws(
      () => current.sandbox.listFiles({ path: "single.txt" }),
      /directory/,
    );
  } finally {
    current.cleanup();
  }
});

test("search_text is literal, bounded, skips binary files, and supports exact allowed files", () => {
  const current = fixture();
  try {
    write(current.directory, "root/binary.dat", Buffer.from([0, 1, 2]));
    const recursive = JSON.parse(current.sandbox.searchText({ path: "root", query: "needle" }));
    assert.deepEqual(recursive.matches.map((match) => [match.path, match.line]), [
      ["root/a.txt", 2],
      ["root/nested/b.txt", 1],
    ]);
    const exact = JSON.parse(current.sandbox.searchText({ path: "single.txt", query: "single" }));
    assert.equal(exact.matches.length, 1);
    assert.throws(
      () => current.sandbox.searchText({ path: "root", query: "" }),
      /invalid search_text arguments/,
    );
    assert.throws(
      () => current.sandbox.searchText({ path: "root", query: "x".repeat(257) }),
      /invalid search_text arguments/,
    );
  } finally {
    current.cleanup();
  }
});

test("tool argument objects reject inherited and unexpected properties", () => {
  const current = fixture();
  try {
    assert.throws(
      () => current.sandbox.readFile(Object.assign(Object.create({ inherited: true }), {
        path: "root/a.txt",
      })),
      /malformed tool arguments/,
    );
    assert.throws(
      () => current.sandbox.listFiles({ path: "root", extra: true }),
      /malformed tool arguments/,
    );
  } finally {
    current.cleanup();
  }
});

test("listing, searching, and encoded tool results enforce hard result bounds", () => {
  const current = fixture();
  try {
    for (let index = 0; index <= MAX_LIST_ENTRIES; index++) {
      write(current.directory, `root/many/file-${String(index).padStart(4, "0")}.txt`, "match\n");
    }
    const listed = JSON.parse(current.sandbox.listFiles({ path: "root/many" }));
    assert.equal(listed.entries.length, MAX_LIST_ENTRIES);
    assert.equal(listed.truncated, true);

    write(
      current.directory,
      "root/matches.txt",
      Array.from({ length: MAX_SEARCH_RESULTS + 1 }, (_, index) => `match ${index}`).join("\n"),
    );
    const searched = JSON.parse(current.sandbox.searchText({ path: "root/matches.txt", query: "match" }));
    assert.equal(searched.matches.length, MAX_SEARCH_RESULTS);
    assert.equal(searched.truncated, true);

    assert.throws(
      () => boundJson({ content: "x".repeat(MAX_TOOL_RESULT_BYTES) }),
      /tool result exceeds byte limit/,
    );
  } finally {
    current.cleanup();
  }
});
