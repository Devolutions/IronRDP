"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");

const {
  AgentFailure, TOOLS, executeTool, isModelUnavailable, providerFailureReason, runAgent,
} = require("../src/agent");

const schema = {
  type: "object",
  additionalProperties: false,
  required: ["answer"],
  properties: { answer: { type: "string" } },
};

const baseConfig = {
  id: "test",
  model: "primary",
  prompt_file: "prompt",
  schema_file: "schema",
  methodology_files: [],
  allowed_roots: [],
  allowed_files: [],
  max_output_bytes: 32 * 1024,
  max_turns: 4,
  max_tool_calls: 4,
};

const sandbox = {
  readFile: (args) => JSON.stringify({ ok: true, read: args.path }),
  listFiles: (args) => JSON.stringify({ ok: true, listed: args.path }),
  searchText: (args) => JSON.stringify({ ok: true, searched: args.query }),
};

function message(content, toolCalls) {
  return { choices: [{ message: {
    role: "assistant",
    content,
    ...(toolCalls ? { tool_calls: toolCalls } : {}),
  } }] };
}

function call(id, name, args) {
  return {
    id,
    type: "function",
    function: { name, arguments: typeof args === "string" ? args : JSON.stringify(args) },
  };
}

function clientFrom(sequence, requests = []) {
  return {
    chat: {
      completions: {
        async create(request) {
          requests.push(structuredClone(request));
          const next = sequence.shift();
          if (next instanceof Error || next?.throw) throw next.throw || next;
          if (typeof next === "function") return next(request);
          return next;
        },
      },
    },
  };
}

test("runtime executes only declared tools and returns schema-validated canonical JSON", async () => {
  const requests = [];
  const client = clientFrom([
    message(null, [
      call("one", "read_file", { path: "root/a" }),
      call("two", "list_files", { path: "root" }),
      call("three", "search_text", { path: "root", query: "needle" }),
    ]),
    message('{ "answer": "done" }'),
  ], requests);
  const result = await runAgent({
    client, config: baseConfig, methodologies: ["method"], prompt: "prompt", sandbox, schema,
  });
  assert.deepEqual(result, {
    output: '{"answer":"done"}',
    model: "primary",
    turnCount: 2,
    toolCallCount: 3,
  });
  assert.deepEqual(requests[0].tools, TOOLS);
  assert.deepEqual(TOOLS.map((tool) => tool.function.name), [
    "read_file", "list_files", "search_text",
  ]);
  assert.equal(requests[0].tool_choice, "auto");
  assert.equal(requests[0].parallel_tool_calls, false);
  assert.match(requests[0].messages[0].content, /required output JSON Schema/);
  assert.match(requests[0].messages[0].content, /"answer"/);
  assert.deepEqual(requests[1].messages.slice(-3).map((entry) => entry.tool_call_id), [
    "one", "two", "three",
  ]);
});

test("runtime reports malformed arguments and unknown tools without executing them", async () => {
  const requests = [];
  let executions = 0;
  const guardedSandbox = {
    readFile() { executions++; },
    listFiles() { executions++; },
    searchText() { executions++; },
  };
  const client = clientFrom([
    message(null, [
      call("bad-json", "read_file", "{"),
      call("unknown", "run_shell", {}),
    ]),
    message('{"answer":"safe"}'),
  ], requests);
  const result = await runAgent({
    client, config: baseConfig, methodologies: [], prompt: "p", sandbox: guardedSandbox, schema,
  });
  assert.equal(result.toolCallCount, 2);
  assert.equal(executions, 0);
  assert.match(requests[1].messages.at(-2).content, /malformed tool arguments/);
  assert.match(requests[1].messages.at(-1).content, /unknown tool/);
});

test("runtime rejects malformed tool call envelopes", async () => {
  const client = clientFrom([message(null, [{ id: "", type: "function", function: {} }])]);
  await assert.rejects(
    runAgent({ client, config: baseConfig, methodologies: [], prompt: "p", sandbox, schema }),
    (error) => error instanceof AgentFailure && error.reason === "provider returned a malformed tool call",
  );
});

test("runtime rejects oversized provider text without attempting repair", async () => {
  const requests = [];
  await assert.rejects(
    runAgent({
      client: clientFrom([message("x".repeat(1024 * 1024 + 1))], requests),
      config: baseConfig, methodologies: [], prompt: "p", sandbox, schema,
    }),
    (error) => error.reason === "provider response exceeded byte limit" && error.turnCount === 1,
  );
  assert.equal(requests.length, 1);
});

test("runtime enforces aggregate tool-call and turn bounds", async () => {
  const calls = [call("one", "read_file", { path: "x" }), call("two", "read_file", { path: "x" })];
  await assert.rejects(
    runAgent({
      client: clientFrom([message(null, calls)]),
      config: { ...baseConfig, max_tool_calls: 1 },
      methodologies: [], prompt: "p", sandbox, schema,
    }),
    (error) => error.reason === "maximum tool call count exceeded" && error.toolCallCount === 0,
  );

  await assert.rejects(
    runAgent({
      client: clientFrom([message(null, [calls[0]])]),
      config: { ...baseConfig, max_turns: 1 },
      methodologies: [], prompt: "p", sandbox, schema,
    }),
    (error) => error.reason === "maximum turn count exceeded" &&
      error.turnCount === 1 && error.toolCallCount === 1,
  );
});

test("runtime stops advertising tools after exhausting the configured budget", async () => {
  const requests = [];
  await runAgent({
    client: clientFrom([
      message(null, [call("one", "read_file", { path: "x" })]),
      message('{"answer":"done"}'),
    ], requests),
    config: { ...baseConfig, max_tool_calls: 1 },
    methodologies: [],
    prompt: "p",
    sandbox,
    schema,
  });
  assert.deepEqual(requests[0].tools, TOOLS);
  assert.equal(requests[1].tools, undefined);
});

test("runtime allows exactly one tools-disabled repair for JSON or schema failure", async () => {
  const requests = [];
  const client = clientFrom([
    message('{"wrong":true}'),
    message('{"answer":"repaired"}'),
  ], requests);
  const result = await runAgent({
    client, config: baseConfig, methodologies: [], prompt: "p", sandbox, schema,
  });

  assert.equal(result.output, '{"answer":"repaired"}');
  assert.equal(result.turnCount, 2);
  assert.equal(requests[1].tools, undefined);
  assert.equal(requests[1].tool_choice, undefined);
  assert.equal(requests[1].parallel_tool_calls, undefined);
  assert.match(requests[1].messages.at(-1).content, /Do not call tools/);

  await assert.rejects(
    runAgent({
      client: clientFrom([message("not-json"), message('{"still":"invalid"}')]),
      config: baseConfig, methodologies: [], prompt: "p", sandbox, schema,
    }),
    (error) => error.reason === "repair response was invalid" && error.turnCount === 2,
  );
});

test("runtime repairs schema-valid output that exceeds the configured byte budget", async () => {
  const requests = [];
  const result = await runAgent({
    client: clientFrom([
      message(JSON.stringify({ answer: "x".repeat(2000) })),
      message('{"answer":"bounded"}'),
    ], requests),
    config: { ...baseConfig, max_output_bytes: 1024 },
    methodologies: [],
    prompt: "p",
    sandbox,
    schema,
  });
  assert.equal(result.output, '{"answer":"bounded"}');
  assert.equal(requests.length, 2);
});

test("repair rejects provider tool calls and does not execute them", async () => {
  let executed = false;
  await assert.rejects(
    runAgent({
      client: clientFrom([
        message("not-json"),
        message(null, [call("repair-tool", "read_file", { path: "x" })]),
      ]),
      config: baseConfig,
      methodologies: [],
      prompt: "p",
      sandbox: { ...sandbox, readFile() { executed = true; } },
      schema,
    }),
    (error) => error.reason === "repair response attempted a tool call",
  );
  assert.equal(executed, false);
});

test("zero-tool configuration never exposes filesystem tools", async () => {
  const requests = [];
  const result = await runAgent({
    client: clientFrom([message('{"answer":"done"}')], requests),
    config: { ...baseConfig, max_tool_calls: 0 },
    methodologies: [],
    prompt: "p",
    sandbox,
    schema,
  });
  assert.equal(result.output, '{"answer":"done"}');
  assert.equal(requests[0].tools, undefined);
  assert.equal(requests[0].tool_choice, undefined);
});

test("trusted fallback runs only for a coded model-unavailable first failure", async () => {
  const requests = [];
  const unavailable = Object.assign(new Error("raw secret provider text"), {
    status: 404,
    code: "model_not_found",
  });
  const result = await runAgent({
    client: clientFrom([{ throw: unavailable }, message('{"answer":"fallback"}')], requests),
    config: { ...baseConfig, fallback_model: "fallback" },
    methodologies: [], prompt: "p", sandbox, schema,
  });
  assert.equal(result.model, "fallback");
  assert.equal(result.turnCount, 2);
  assert.deepEqual(requests.map((request) => request.model), ["primary", "fallback"]);
});

test("fallback never runs for authentication, quota, generic 404, or later failures", async () => {
  for (const providerError of [
    { status: 401, code: "model_not_found" },
    { status: 429, code: "model_not_found" },
    { status: 404 },
  ]) {
    const requests = [];
    await assert.rejects(
      runAgent({
        client: clientFrom([{ throw: providerError }], requests),
        config: { ...baseConfig, fallback_model: "fallback" },
        methodologies: [], prompt: "p", sandbox, schema,
      }),
      (error) => error.model === "primary",
    );
    assert.equal(requests.length, 1);
  }

  const requests = [];
  await assert.rejects(
    runAgent({
      client: clientFrom([
        message(null, [call("one", "read_file", { path: "x" })]),
        { throw: { status: 404, code: "model_not_found" } },
      ], requests),
      config: { ...baseConfig, fallback_model: "fallback" },
      methodologies: [], prompt: "p", sandbox, schema,
    }),
    (error) => error.model === "primary" && error.toolCallCount === 1,
  );
  assert.equal(requests.length, 2);
});

test("provider errors are reduced to fixed non-sensitive categories", () => {
  assert.equal(isModelUnavailable({ status: 404, code: "model_not_found" }), true);
  assert.equal(isModelUnavailable({ status: 404, message: "model not found" }), false);
  assert.equal(providerFailureReason({ status: 401, message: "secret" }), "provider authentication failed");
  assert.equal(providerFailureReason({ status: 429, message: "secret" }), "provider rate or quota limit reached");
  assert.equal(providerFailureReason({ status: 503, message: "secret" }), "provider service unavailable");
  assert.equal(providerFailureReason(new Error("secret")), "provider request failed");
});

test("executeTool bounds oversized argument strings", () => {
  const result = executeTool(call("large", "read_file", "x".repeat(16 * 1024 + 1)), sandbox);
  assert.match(result, /tool arguments exceed byte limit/);
});
