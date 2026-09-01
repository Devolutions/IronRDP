"use strict";

const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const assert = require("node:assert/strict");

const { loadConfiguration, validateBaseUrl } = require("../src/config");
const { MAX_METHODOLOGY_TOTAL_BYTES } = require("../src/limits");
const { scratchWorkspace, write } = require("./helpers");

function configurationFixture(changes = {}) {
  const workspace = scratchWorkspace();
  fs.mkdirSync(path.join(workspace.directory, "root"));
  write(workspace.directory, "prompt.md", "configured prompt");
  write(workspace.directory, "method.md", "configured method");
  write(workspace.directory, "schema.json", JSON.stringify({
    type: "object",
    required: ["ok"],
    properties: { ok: { type: "boolean" } },
  }));
  const config = {
    id: "generic-test",
    model: "model-1",
    prompt_file: "prompt.md",
    schema_file: "schema.json",
    methodology_files: ["method.md"],
    allowed_roots: ["root"],
    allowed_files: [],
    max_output_bytes: 32 * 1024,
    max_turns: 10,
    max_tool_calls: 20,
    ...changes,
  };
  write(workspace.directory, "config.json", JSON.stringify(config));
  return workspace;
}

test("configuration loads workflow artifacts and constructs capabilities", () => {
  const workspace = configurationFixture();
  try {
    const loaded = loadConfiguration(workspace.directory, "config.json");
    assert.equal(loaded.config.id, "generic-test");
    assert.equal(loaded.prompt, "configured prompt");
    assert.deepEqual(loaded.methodologies, ["configured method"]);
    assert.equal(loaded.schema.type, "object");
    assert.doesNotThrow(() => loaded.sandbox.listFiles({ path: "root" }));
  } finally {
    workspace.cleanup();
  }
});

test("configuration rejects unknown fields, unsafe models, and empty capabilities", () => {
  for (const changes of [
    { unexpected: true },
    { model: "unsafe model\n" },
    { allowed_roots: [], allowed_files: [] },
    { max_output_bytes: 1023 },
    { max_turns: 51 },
    { max_tool_calls: 201 },
  ]) {
    const workspace = configurationFixture(changes);
    try {
      assert.throws(() => loadConfiguration(workspace.directory, "config.json"));
    } finally {
      workspace.cleanup();
    }
  }
});

test("configuration bounds aggregate methodology bytes", () => {
  const workspace = configurationFixture({ methodology_files: ["one.md", "two.md", "three.md", "four.md", "five.md"] });
  try {
    for (const name of ["one.md", "two.md", "three.md", "four.md", "five.md"]) {
      write(workspace.directory, name, "x".repeat(Math.floor(MAX_METHODOLOGY_TOTAL_BYTES / 4)));
    }
    assert.throws(
      () => loadConfiguration(workspace.directory, "config.json"),
      /methodology files exceed byte limit/,
    );
  } finally {
    workspace.cleanup();
  }
});

test("base URL validation permits HTTPS and local HTTP but rejects credential-bearing or redirect-like URLs", () => {
  assert.equal(validateBaseUrl("https://provider.example/v1/"), "https://provider.example/v1");
  assert.equal(validateBaseUrl("http://localhost:8080/v1"), "http://localhost:8080/v1");
  for (const invalid of [
    "http://provider.example/v1",
    "https://user:secret@provider.example/v1",
    "https://provider.example/v1?key=secret",
    "https://provider.example/v1#fragment",
    "file:///etc/passwd",
    "not a URL",
  ]) {
    assert.throws(() => validateBaseUrl(invalid), undefined, invalid);
  }
});
