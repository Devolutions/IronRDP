"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const OpenAI = require("openai");
const {
  APIConnectionError, APIConnectionTimeoutError, APIUserAbortError,
} = OpenAI;

const {
  AgentFailure, TOOLS, compileOutputValidator, executeTool, providerFailureDiagnostic,
  providerFailureReason, runAgent,
} = require("../src/agent");
const { RuntimeMetrics, createProviderClient } = require("../src/provider");

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
    turnCount: 2,
    toolCallCount: 3,
    outputRepairCount: 0,
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
      client: clientFrom([message("not-json")]),
      config: { ...baseConfig, max_turns: 1 },
      methodologies: [], prompt: "p", sandbox, schema,
    }),
    (error) => error.reason === "maximum turn count exceeded" &&
      error.turnCount === 1 && error.toolCallCount === 0,
  );
});

test("runtime reserves tool-free finalization and repair turns", async () => {
  const requests = [];
  const result = await runAgent({
    client: clientFrom([
      message(null, [call("one", "read_file", { path: "x" })]),
      message(null, [call("two", "read_file", { path: "x" })]),
      message("not-json"),
      message('{"answer":"repaired"}'),
    ], requests),
    config: baseConfig,
    methodologies: [],
    prompt: "p",
    sandbox,
    schema,
  });

  assert.equal(result.output, '{"answer":"repaired"}');
  assert.equal(result.turnCount, 4);
  assert.equal(result.toolCallCount, 2);
  assert.deepEqual(requests.map((request) => request.tools !== undefined), [true, true, false, false]);
  assert.deepEqual(requests[2].response_format, { type: "json_object" });
  assert.deepEqual(requests[3].response_format, { type: "json_object" });
  assert.match(requests[2].messages.at(-1).content, /Investigation is complete/);
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
  assert.deepEqual(requests[1].response_format, { type: "json_object" });
  assert.match(requests[1].messages.at(-1).content, /Do not call tools/);
  assert.match(requests[1].messages.at(-1).content, /Correct every reported validation error/);
  assert.match(requests[1].messages.at(-1).content, /no Markdown fences/);

  await assert.rejects(
    runAgent({
      client: clientFrom([message('{"wrong":true}'), message("not-json")]),
      config: baseConfig, methodologies: [], prompt: "p", sandbox, schema,
    }),
    (error) => error.reason ===
      "output remained invalid after the repair limit" && error.category === "output-invalid" &&
      error.turnCount === 2 && error.outputRepairCount === 1,
  );
});

test("runtime rejects fenced repair output despite requesting JSON mode", async () => {
  const requests = [];
  await assert.rejects(
    runAgent({
      client: clientFrom([
        message('{"wrong":true}'),
        message('```json\n{"answer":"wrapped"}\n```'),
      ], requests),
      config: baseConfig,
      methodologies: [],
      prompt: "p",
      sandbox,
      schema,
    }),
    (error) => error.reason ===
      "output remained invalid after the repair limit" && error.category === "output-invalid" &&
      error.turnCount === 2,
  );
  assert.deepEqual(requests[1].response_format, { type: "json_object" });
});

test("validator-directed repair preserves the previous candidate and may make bounded reads", async () => {
  const requests = [];
  const observed = [];
  const validator = async (candidate, context) => {
    observed.push({ candidate, ...context });
    if (candidate.answer === "missing citation") {
      return { ok: false, reason: "citation requires source verification" };
    }
    return { ok: true };
  };
  const result = await runAgent({
    client: clientFrom([
      message('{"answer":"missing citation"}'),
      message(null, [call("citation", "read_file", { path: "evidence.txt" })]),
      message('{"answer":"cited"}'),
    ], requests),
    config: { ...baseConfig, max_turns: 4, max_output_repair_attempts: 1 },
    methodologies: [],
    prompt: "p",
    sandbox,
    schema,
    validator,
  });

  assert.equal(result.output, '{"answer":"cited"}');
  assert.equal(result.outputRepairCount, 1);
  assert.deepEqual(observed, [
    {
      candidate: { answer: "missing citation" },
      previousCandidate: null,
      repairAttempt: 0,
    },
    {
      candidate: { answer: "cited" },
      previousCandidate: { answer: "missing citation" },
      repairAttempt: 1,
    },
  ]);
  assert.deepEqual(requests.map((request) => request.tools !== undefined), [true, true, true]);
  assert.match(requests[1].messages.at(-1).content, /not to begin a new investigation/);
  assert.equal(requests[2].messages.at(-1).role, "tool");
});

test("validator execution failures are terminal and strict output is opt-in", async () => {
  const terminal = new Error("validation context is stale");
  terminal.code = "VALIDATOR_TERMINAL";
  await assert.rejects(
    runAgent({
      client: clientFrom([message('{"answer":"candidate"}')]),
      config: baseConfig,
      methodologies: [],
      prompt: "p",
      sandbox,
      schema,
      validator: async () => { throw Object.assign(terminal, {
        reason: "validation context is stale",
        category: "validator-terminal",
      }); },
    }),
    (error) => error.category === "validator-terminal" &&
      error.reason === "validation context is stale" && error.outputRepairCount === 0,
  );

  const requests = [];
  await runAgent({
    client: clientFrom([message('{"answer":"candidate"}')], requests),
    config: { ...baseConfig, max_tool_calls: 0, output_format: "json_schema" },
    methodologies: [],
    prompt: "p",
    sandbox,
    schema,
  });
  assert.deepEqual(requests[0].response_format, {
    type: "json_schema",
    json_schema: { name: "structured_output", strict: true, schema },
  });
});

test("runtime repairs a final response with no text", async () => {
  const requests = [];
  const result = await runAgent({
    client: clientFrom([
      message(null),
      message('{"answer":"repaired"}'),
    ], requests),
    config: baseConfig,
    methodologies: [],
    prompt: "p",
    sandbox,
    schema,
  });

  assert.equal(result.output, '{"answer":"repaired"}');
  assert.equal(result.turnCount, 2);
  assert.equal(requests[1].tools, undefined);
  assert.match(requests[1].messages.at(-1).content, /response was empty/);
});

test("runtime rejects malformed content without attempting repair", async () => {
  const requests = [];
  await assert.rejects(
    runAgent({
      client: clientFrom([message({ text: '{"answer":"invalid"}' })], requests),
      config: baseConfig,
      methodologies: [],
      prompt: "p",
      sandbox,
      schema,
    }),
    (error) => error.reason === "provider response did not contain text",
  );
  assert.equal(requests.length, 1);
});

test("validation diagnostics do not expose model-provided property names", () => {
  const validateOutput = compileOutputValidator({
    type: "object",
    patternProperties: { "^.+$": { type: "string" } },
  });
  const candidate = validateOutput('{"MODEL_RESPONSE_SECRET_SENTINEL":42}');
  assert.equal(candidate.ok, false);
  assert.match(candidate.reason, /^response did not match the schema: #\//);
  assert.doesNotMatch(candidate.reason, /MODEL_RESPONSE_SECRET_SENTINEL/);
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
  assert.deepEqual(requests[0].response_format, { type: "json_object" });
});

test("runtime does not request object-only JSON mode for non-object schemas", async () => {
  const requests = [];
  const result = await runAgent({
    client: clientFrom([message('["done"]')], requests),
    config: { ...baseConfig, max_tool_calls: 0 },
    methodologies: [],
    prompt: "p",
    sandbox,
    schema: { type: "array", items: { type: "string" } },
  });
  assert.equal(result.output, '["done"]');
  assert.equal(requests[0].response_format, undefined);
});

test("provider errors are reduced to fixed non-sensitive categories", () => {
  assert.equal(providerFailureReason({ status: 401, message: "secret" }), "provider credential rejected");
  assert.equal(providerFailureReason({ status: 403, message: "secret" }), "provider access forbidden");
  assert.equal(providerFailureReason({ status: 429, message: "secret" }), "provider rate limit reached");
  assert.equal(providerFailureReason({ status: 408, message: "secret" }), "provider request timed out");
  assert.equal(providerFailureReason({ status: 409, message: "secret" }), "provider request conflict");
  assert.equal(providerFailureReason({ status: 503, message: "secret" }), "provider service unavailable");
  assert.equal(
    providerFailureReason(new APIConnectionTimeoutError({ message: "secret" })),
    "provider request timed out",
  );
  assert.equal(
    providerFailureReason(new APIConnectionError({
      message: "secret",
      cause: new Error("nested secret"),
    })),
    "provider connection failed",
  );
  assert.equal(
    providerFailureReason(new APIUserAbortError({ message: "secret" })),
    "provider request failed",
  );
  class UnknownConnectionError extends APIConnectionError {}
  assert.equal(
    providerFailureReason(new UnknownConnectionError({ message: "secret" })),
    "provider request failed",
  );
  assert.equal(providerFailureReason(new Error("secret")), "provider request failed");
});

test("real SDK classifies interrupted response bodies as recoverable connections", async () => {
  const metrics = new RuntimeMetrics();
  let calls = 0;
  const client = createProviderClient(OpenAI, {
    apiKey: "test-key",
    baseURL: "https://provider.example/v1",
    maxRetries: 0,
    timeout: 100,
  }, metrics, async () => {
    calls++;
    return new Response(new ReadableStream({
      start(controller) {
        setTimeout(() => {
          controller.error(Object.assign(new TypeError("terminated"), {
            cause: { code: "UND_ERR_SOCKET" },
          }));
        }, 40);
      },
    }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  });

  await assert.rejects(
    runAgent({
      client, config: baseConfig, methodologies: [], prompt: "p", sandbox, schema, metrics,
    }),
    (error) => error instanceof AgentFailure && error.category === "provider-connection" &&
      error.retryable && error.turnCount === 1,
  );

  assert.equal(calls, 1);
  assert.equal(metrics.snapshot().providerAttempts[0].durationMs >= 40, true);
});

test("real SDK preserves timeout body-consumption duration", async () => {
  const metrics = new RuntimeMetrics();
  let calls = 0;
  const client = createProviderClient(OpenAI, {
    apiKey: "test-key",
    baseURL: "https://provider.example/v1",
    maxRetries: 0,
    timeout: 20,
  }, metrics, async () => {
    calls++;
    return new Response(new ReadableStream({
      start(controller) {
        setTimeout(() => {
          controller.enqueue(new TextEncoder().encode("{}"));
          controller.close();
        }, 40);
      },
    }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  });

  await assert.rejects(
    runAgent({
      client, config: baseConfig, methodologies: [], prompt: "p", sandbox, schema, metrics,
    }),
    (error) => error instanceof AgentFailure && error.category === "provider-timeout" &&
      error.retryable && error.turnCount === 1,
  );

  assert.equal(calls, 1);
  assert.equal(metrics.snapshot().providerAttempts[0].durationMs >= 20, true);
});

test("provider diagnostics expose only bounded status and request IDs", () => {
  assert.deepEqual(providerFailureDiagnostic({
    status: 403,
    requestID: "req_direct-123",
    headers: { get: () => "req_header-456" },
    error: { message: "secret" },
  }), {
    status: 403,
    requestId: "req_direct-123",
  });
  assert.deepEqual(providerFailureDiagnostic({
    status: "429",
    headers: { get: (name) => name === "x-request-id" ? "req_header-456" : null },
  }), {
    status: 429,
    requestId: "req_header-456",
  });
  assert.deepEqual(providerFailureDiagnostic({
    status: 401,
    request_id: "unsafe request id\nsecret",
  }), {
    status: 401,
  });
  assert.equal(providerFailureDiagnostic(new Error("secret")), null);
});

test("executeTool bounds oversized argument strings", () => {
  const result = executeTool(call("large", "read_file", "x".repeat(16 * 1024 + 1)), sandbox);
  assert.match(result, /tool arguments exceed byte limit/);
});

test("repair failures retain the completed output repair count", async () => {
  await assert.rejects(
    runAgent({
      client: clientFrom([
        message('{"wrong":true}'),
        message({ unexpected: true }),
      ]),
      config: baseConfig,
      methodologies: [],
      prompt: "p",
      sandbox,
      schema,
    }),
    (error) => error.reason === "provider response did not contain text" &&
      error.outputRepairCount === 1,
  );
});
