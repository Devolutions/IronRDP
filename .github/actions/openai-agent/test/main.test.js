"use strict";

const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const assert = require("node:assert/strict");
const { APIConnectionError, APIConnectionTimeoutError } = require("openai");

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
    assert.equal(core.outputs.get("failure-reason"), "provider credential rejected");
    assert.equal(core.outputs.get("turn-count"), "1");
    assert.deepEqual(
      core.events.filter((event) => event[0] === "failed").map((event) => event[1]),
      ["provider credential rejected"],
    );
    const observable = JSON.stringify(core.events);
    assert.doesNotMatch(observable, /RAW_PROVIDER_SECRET_SENTINEL/);
    assert.deepEqual(
      core.events.filter((event) => event[0] === "info").map((event) => JSON.parse(event[1]))
        .find((event) => event.event === "openai-agent.provider-failure"),
      {
        event: "openai-agent.provider-failure",
        reason: "provider credential rejected",
        status: 401,
      },
    );
  } finally {
    workspace.cleanup();
  }
});

test("main emits a bounded provider request ID without raw errors", async () => {
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
          status: 403,
          requestID: "req_safe-123",
          headers: { get: () => "RAW_HEADER_SECRET_SENTINEL" },
        });
      } } };
    }
  }
  try {
    await main(core, { GITHUB_WORKSPACE: workspace.directory }, FailingOpenAI);
    assert.equal(core.outputs.get("failure-reason"), "provider access forbidden");
    const observable = JSON.stringify(core.events);
    assert.deepEqual(
      core.events.filter((event) => event[0] === "info").map((event) => JSON.parse(event[1]))
        .find((event) => event.event === "openai-agent.provider-failure"),
      {
        event: "openai-agent.provider-failure",
        reason: "provider access forbidden",
        status: 403,
        requestId: "req_safe-123",
      },
    );
    assert.doesNotMatch(observable, /RAW_PROVIDER_SECRET_SENTINEL|RAW_HEADER_SECRET_SENTINEL/);
  } finally {
    workspace.cleanup();
  }
});

test("main distinguishes provider quota and service failures", async () => {
  const workspace = actionFixture();
  const inputs = {
    "api-key": "API_KEY_SECRET_SENTINEL",
    "base-url": "https://provider.example/v1",
    "config-file": "config.json",
  };
  try {
    for (const [status, reason] of [
      [429, "provider rate or quota limit reached"],
      [503, "provider service unavailable"],
    ]) {
      const core = mockCore(inputs);
      class FailingOpenAI {
        constructor() {
          this.chat = { completions: { create: async () => {
            throw Object.assign(new Error("RAW_PROVIDER_SECRET_SENTINEL"), { status });
          } } };
        }
      }
      await main(core, { GITHUB_WORKSPACE: workspace.directory }, FailingOpenAI);
      assert.equal(core.outputs.get("failure-reason"), reason);
      assert.deepEqual(
        core.events.filter((event) => event[0] === "info").map((event) => JSON.parse(event[1]))
          .find((event) => event.event === "openai-agent.provider-failure"),
        { event: "openai-agent.provider-failure", reason, status },
      );
      assert.doesNotMatch(JSON.stringify(
        core.events.filter((event) => event[0] !== "secret"),
      ), /RAW_PROVIDER_SECRET_SENTINEL|API_KEY_SECRET_SENTINEL/);
    }
  } finally {
    workspace.cleanup();
  }
});

test("main safely distinguishes provider transport failures", async () => {
  const workspace = actionFixture();
  const inputs = {
    "api-key": "API_KEY_SECRET_SENTINEL",
    "base-url": "https://provider.example/v1",
    "config-file": "config.json",
  };
  try {
    for (const [error, reason] of [
      [
        new APIConnectionTimeoutError({ message: "RAW_TIMEOUT_SECRET_SENTINEL" }),
        "provider request timed out",
      ],
      [
        new APIConnectionError({
          message: "RAW_CONNECTION_SECRET_SENTINEL",
          cause: new Error("RAW_CAUSE_SECRET_SENTINEL"),
        }),
        "provider connection failed",
      ],
    ]) {
      const core = mockCore(inputs);
      class FailingOpenAI {
        constructor() {
          this.chat = { completions: { create: async () => { throw error; } } };
        }
      }
      await main(core, { GITHUB_WORKSPACE: workspace.directory }, FailingOpenAI);
      assert.equal(core.outputs.get("failure-reason"), reason);
      assert.deepEqual(
        core.events.filter((event) => event[0] === "info").map((event) => JSON.parse(event[1]))
          .find((event) => event.event === "openai-agent.provider-failure"),
        { event: "openai-agent.provider-failure", reason },
      );
      assert.doesNotMatch(
        JSON.stringify(core.events.filter((event) => event[0] !== "secret")),
        /RAW_TIMEOUT_SECRET_SENTINEL|RAW_CONNECTION_SECRET_SENTINEL|RAW_CAUSE_SECRET_SENTINEL|API_KEY_SECRET_SENTINEL/,
      );
    }
  } finally {
    workspace.cleanup();
  }
});

test("main reports why repaired output remains invalid", async () => {
  const workspace = actionFixture();
  const core = mockCore({
    "api-key": "key",
    "base-url": "https://provider.example/v1",
    "config-file": "config.json",
  });
  const responses = [
    { choices: [{ message: { content: '{"wrong":true}' } }] },
    { choices: [{ message: { content: "not-json" } }] },
  ];
  class InvalidRepairOpenAI {
    constructor() {
      this.chat = { completions: { create: async () => responses.shift() } };
    }
  }
  const reason = "repair response was invalid: response was not valid JSON";
  try {
    await main(core, { GITHUB_WORKSPACE: workspace.directory }, InvalidRepairOpenAI);
    assert.equal(core.outputs.get("structured-output"), "");
    assert.equal(core.outputs.get("failure-reason"), reason);
    assert.equal(core.outputs.get("turn-count"), "2");
    assert.deepEqual(
      core.events.filter((event) => event[0] === "failed").map((event) => event[1]),
      [reason],
    );
    assert.deepEqual(
      core.events.filter((event) => event[0] === "info").map((event) => JSON.parse(event[1]))
        .find((event) => event.event === "openai-agent.failure"),
      { event: "openai-agent.failure", phase: "runtime", reason },
    );
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
    assert.deepEqual(
      core.events.filter((event) => event[0] === "info").map((event) => JSON.parse(event[1]))
        .find((event) => event.event === "openai-agent.failure"),
      { event: "openai-agent.failure", phase: "configuration", reason: "invalid path" },
    );
  } finally {
    workspace.cleanup();
  }
});

test("main reports each missing input without constructing a provider client", async () => {
  const workspace = actionFixture();
  const cases = [
    [{}, "api key input is missing"],
    [{ "api-key": "key" }, "base URL input is missing"],
    [{
      "api-key": "key",
      "base-url": "https://provider.example/v1",
    }, "config file input is missing"],
  ];
  try {
    for (const [inputs, reason] of cases) {
      const core = mockCore(inputs);
      let constructed = false;
      class UnexpectedOpenAI {
        constructor() {
          constructed = true;
        }
      }
      await main(core, { GITHUB_WORKSPACE: workspace.directory }, UnexpectedOpenAI);
      assert.equal(constructed, false);
      assert.equal(core.outputs.get("failure-reason"), reason);
      assert.deepEqual(
        core.events.filter((event) => event[0] === "info").map((event) => JSON.parse(event[1]))
          .find((event) => event.event === "openai-agent.failure"),
        { event: "openai-agent.failure", phase: "input", reason },
      );
    }
  } finally {
    workspace.cleanup();
  }
});

test("main reports workspace and provider client initialization failures safely", async () => {
  const workspace = actionFixture();
  const inputs = {
    "api-key": "key",
    "base-url": "https://provider.example/v1",
    "config-file": "config.json",
  };
  try {
    const missingWorkspaceCore = mockCore(inputs);
    await main(missingWorkspaceCore, {}, class UnexpectedOpenAI {});
    assert.equal(missingWorkspaceCore.outputs.get("failure-reason"), "workspace is unavailable");
    assert.deepEqual(
      missingWorkspaceCore.events.filter((event) => event[0] === "info")
        .map((event) => JSON.parse(event[1]))
        .find((event) => event.event === "openai-agent.failure"),
      {
        event: "openai-agent.failure",
        phase: "configuration",
        reason: "workspace is unavailable",
      },
    );

    const initializationCore = mockCore(inputs);
    class FailingOpenAI {
      constructor() {
        throw new Error("CLIENT_INITIALIZATION_SECRET_SENTINEL");
      }
    }
    await main(initializationCore, { GITHUB_WORKSPACE: workspace.directory }, FailingOpenAI);
    assert.equal(
      initializationCore.outputs.get("failure-reason"),
      "provider client initialization failed",
    );
    assert.deepEqual(
      initializationCore.events.filter((event) => event[0] === "info")
        .map((event) => JSON.parse(event[1]))
        .find((event) => event.event === "openai-agent.failure"),
      {
        event: "openai-agent.failure",
        phase: "initialization",
        reason: "provider client initialization failed",
      },
    );
    assert.doesNotMatch(
      JSON.stringify(initializationCore.events),
      /CLIENT_INITIALIZATION_SECRET_SENTINEL/,
    );
  } finally {
    workspace.cleanup();
  }
});
