"use strict";

const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const assert = require("node:assert/strict");

const { main } = require("../src/main");
const { scratchWorkspace, write } = require("./helpers");

test("action metadata exposes only configured inputs and required outputs on node24", () => {
  const action = fs.readFileSync(path.join(__dirname, "..", "action.yml"), "utf8");
  assert.match(action, /runs:\r?\n  using: node24\r?\n  main: dist\/index\.js/);
  for (const input of ["api-key", "base-url", "config-file"]) {
    assert.match(action, new RegExp(`^  ${input}:\\r?$`, "m"));
  }
  for (const output of [
    "structured-output", "failure-reason", "turn-count", "tool-call-count",
  ]) {
    assert.match(action, new RegExp(`^  ${output}:\\r?$`, "m"));
  }
  assert.match(action, /Number of model turns attempted, excluding SDK retries/);
  assert.equal(fs.existsSync(path.join(__dirname, "..", "dist", "index.js")), true);
  assert.equal(fs.existsSync(path.join(__dirname, "..", "dist", "licenses.txt")), true);
});

function actionFixture() {
  const workspace = scratchWorkspace();
  fs.mkdirSync(`${workspace.directory}/evidence`);
  write(workspace.directory, "prompt.md", "PROMPT_SECRET_SENTINEL");
  write(workspace.directory, "schema.json", JSON.stringify({
    type: "object",
    additionalProperties: false,
    required: ["answer"],
    properties: { answer: { type: "string" } },
  }));
  write(workspace.directory, "config.json", JSON.stringify({
    id: "safe-id",
    model: "safe-model",
    prompt_file: "prompt.md",
    schema_file: "schema.json",
    methodology_files: [],
    allowed_roots: ["evidence"],
    allowed_files: [],
    max_output_bytes: 32 * 1024,
    max_turns: 3,
    max_tool_calls: 2,
  }));
  return workspace;
}

function mockCore(inputs) {
  const events = [];
  const outputs = new Map();
  return {
    events,
    outputs,
    getInput(name) {
      events.push(["input", name]);
      return inputs[name] || "";
    },
    setSecret(value) {
      events.push(["secret", value]);
    },
    setOutput(name, value) {
      events.push(["output", name, value]);
      outputs.set(name, value);
    },
    info(value) {
      events.push(["info", value]);
    },
    setFailed(value) {
      events.push(["failed", value]);
    },
  };
}

test("main masks the key immediately, rejects redirects, and emits only bounded metadata", async () => {
  const workspace = actionFixture();
  const core = mockCore({
    "api-key": "API_KEY_SECRET_SENTINEL",
    "base-url": "https://provider.example/v1",
    "config-file": "config.json",
  });
  let options;
  let request;
  class MockOpenAI {
    constructor(received) {
      options = received;
      this.chat = { completions: { create: async (value) => {
        request = value;
        return { choices: [{ message: { content: '{"answer":"MODEL_RESPONSE_SENTINEL"}' } }] };
      } } };
    }
  }
  try {
    await main(core, { GITHUB_WORKSPACE: workspace.directory }, MockOpenAI);
    const inputIndex = core.events.findIndex((event) => event[0] === "input" && event[1] === "api-key");
    const secretIndex = core.events.findIndex((event) => event[0] === "secret");
    const secondInputIndex = core.events.findIndex(
      (event, index) => index > inputIndex && event[0] === "input",
    );
    assert.equal(secretIndex, inputIndex + 1);
    assert.equal(secretIndex < secondInputIndex, true);
    assert.equal(options.apiKey, "API_KEY_SECRET_SENTINEL");
    assert.equal(options.baseURL, "https://provider.example/v1");
    assert.equal(options.maxRetries, 2);
    assert.equal(options.timeout, 120_000);
    assert.deepEqual(options.fetchOptions, { redirect: "error" });
    assert.equal(request.model, "safe-model");
    assert.equal(core.outputs.get("structured-output"), '{"answer":"MODEL_RESPONSE_SENTINEL"}');
    assert.equal(core.outputs.get("failure-reason"), "");
    assert.equal(core.outputs.get("turn-count"), "1");
    assert.equal(core.outputs.get("tool-call-count"), "0");
    assert.equal(core.events.some((event) => event[0] === "failed"), false);

    const logs = core.events.filter((event) => event[0] === "info").map((event) => event[1]).join("\n");
    for (const forbidden of [
      "API_KEY_SECRET_SENTINEL", "PROMPT_SECRET_SENTINEL", "MODEL_RESPONSE_SENTINEL",
      "provider.example",
    ]) {
      assert.doesNotMatch(logs, new RegExp(forbidden));
    }
  } finally {
    workspace.cleanup();
  }
});

test("main never logs or outputs raw provider errors", async () => {
  const workspace = actionFixture();
  const core = mockCore({
    "api-key": "API_KEY_SECRET_SENTINEL",
    "base-url": "https://provider.example/v1",
    "config-file": "config.json",
  });
  class FailingOpenAI {
    constructor() {
      this.chat = { completions: { create: async () => {
        throw Object.assign(new Error("RAW_PROVIDER_SECRET_SENTINEL"), {
          status: 401,
          error: { message: "RAW_PROVIDER_SECRET_SENTINEL" },
        });
      } } };
    }
  }
  try {
    await main(core, { GITHUB_WORKSPACE: workspace.directory }, FailingOpenAI);
    assert.equal(core.outputs.get("structured-output"), "");
    assert.equal(core.outputs.get("failure-reason"), "provider authentication failed");
    assert.equal(core.outputs.get("turn-count"), "1");
    assert.deepEqual(
      core.events.filter((event) => event[0] === "failed").map((event) => event[1]),
      ["provider authentication failed"],
    );
    const observable = JSON.stringify(core.events);
    assert.doesNotMatch(observable, /RAW_PROVIDER_SECRET_SENTINEL/);
  } finally {
    workspace.cleanup();
  }
});

test("main reports configuration failures without constructing a provider client", async () => {
  const workspace = actionFixture();
  const core = mockCore({
    "api-key": "key",
    "base-url": "https://provider.example/v1",
    "config-file": "../config.json",
  });
  let constructed = false;
  class UnexpectedOpenAI {
    constructor() {
      constructed = true;
    }
  }
  try {
    await main(core, { GITHUB_WORKSPACE: workspace.directory }, UnexpectedOpenAI);
    assert.equal(constructed, false);
    assert.equal(core.outputs.get("failure-reason"), "invalid path");
    assert.equal(core.events.some((event) => event[0] === "failed"), true);
  } finally {
    workspace.cleanup();
  }
});
