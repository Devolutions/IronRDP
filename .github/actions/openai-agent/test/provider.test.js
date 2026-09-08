"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const OpenAI = require("openai");

const {
  RuntimeMetrics, createProviderClient, hasKnownQuotaCode, retryAfterMilliseconds,
} = require("../src/provider");

class BaseClient {
  constructor(options) {
    this.options = options;
  }

  async shouldRetry() {
    return true;
  }

  async retryRequest(options, retriesRemaining, requestLogID) {
    return this.makeRequest(options, retriesRemaining - 1, requestLogID);
  }
}

test("SDK adapter suppresses known quota retries without exposing bodies", async () => {
  const metrics = new RuntimeMetrics();
  const client = createProviderClient(OpenAI, {
    apiKey: "test-key",
    baseURL: "https://provider.example/v1",
  }, metrics);
  assert.equal(await client.shouldRetry(new Response(JSON.stringify({
    error: { code: "insufficient_quota", message: "secret" },
  }), { status: 429 })), false);
  assert.equal(await client.shouldRetry(new Response(JSON.stringify({
    error: { code: "temporarily_limited", message: "secret" },
  }), { status: 429 })), true);
  assert.equal(await hasKnownQuotaCode(new Response(JSON.stringify({
    error: { type: "quota_exhausted", message: "secret" },
  }), { status: 429 })), true);
  const started = Date.now();
  assert.equal(await hasKnownQuotaCode(new Response("x".repeat(8 * 1024 + 1), {
    status: 429,
  }), 100), false);
  assert.equal(Date.now() - started < 100, true);
});

test("SDK adapter honors valid Retry-After without another retry budget", async () => {
  const delays = [];
  const client = createProviderClient(
    BaseClient,
    { timeout: 120_000 },
    new RuntimeMetrics(),
    globalThis.fetch,
    async (milliseconds) => { delays.push(milliseconds); },
  );
  client.makeRequest = async (options, retriesRemaining, requestLogID) => ({
    options, retriesRemaining, requestLogID,
  });
  const result = await client.retryRequest({}, 4, "request", new Headers({ "retry-after": "600" }));
  assert.deepEqual(delays, [600_000]);
  assert.deepEqual(result, { options: {}, retriesRemaining: 3, requestLogID: "request" });
  assert.equal(retryAfterMilliseconds(new Headers({ "retry-after-ms": "250" })), 250);
  assert.equal(
    retryAfterMilliseconds(new Headers({ "retry-after-ms": "3000000000" })),
    3_000_000_000,
  );
  assert.equal(
    retryAfterMilliseconds(
      new Headers({ "retry-after": "Thu, 01 Jan 1970 00:10:00 GMT" }),
      0,
    ),
    600_000,
  );
  assert.equal(retryAfterMilliseconds(new Headers({ "retry-after": "invalid" })), undefined);
});

test("SDK adapter prohibits policy-terminal retries despite provider headers", async () => {
  const client = createProviderClient(OpenAI, {
    apiKey: "test-key",
    baseURL: "https://provider.example/v1",
  }, new RuntimeMetrics());
  for (const [status, body] of [
    [401, {}],
    [403, {}],
    [400, {}],
    [429, { error: { code: "insufficient_quota" } }],
  ]) {
    assert.equal(await client.shouldRetry(new Response(JSON.stringify(body), {
      status,
      headers: { "x-should-retry": "true" },
    })), false);
  }
  for (const status of [408, 409, 429, 503]) {
    assert.equal(await client.shouldRetry(new Response("", {
      status,
      headers: { "x-should-retry": "false" },
    })), true);
  }
});

test("runtime metrics retain activity and mark partial usage incomplete", async () => {
  const metrics = new RuntimeMetrics(() => 0);
  const request = metrics.beginRequest("repairing");
  const first = metrics.beginAttempt();
  metrics.observeResponse(first, new Response("", { status: 429 }));
  metrics.finishAttempt(first);
  const second = metrics.beginAttempt();
  metrics.observeResponse(second, new Response("", {
    status: 200,
    headers: { "x-request-id": "req_safe-123" },
  }));
  metrics.finishAttempt(second);
  metrics.recordCompletion(request, {
    choices: [{ finish_reason: "stop" }],
    usage: { prompt_tokens: 4, completion_tokens: 3, total_tokens: 7 },
  });

  const snapshot = metrics.snapshot();
  assert.equal(snapshot.requestRetryCount, 1);
  assert.equal(snapshot.providerFinishReason, "stop");
  assert.deepEqual(snapshot.tokenUsage, {
    complete: false,
    knownAttemptCount: 1,
    unknownAttemptCount: 1,
    inputTokens: 4,
    outputTokens: 3,
    totalTokens: 7,
  });
  assert.deepEqual(snapshot.providerAttempts, [
    { activity: "repairing", durationMs: 0, status: 429 },
    {
      activity: "repairing",
      durationMs: 0,
      status: 200,
      requestId: "req_safe-123",
      finishReason: "stop",
      usage: { inputTokens: 4, outputTokens: 3, totalTokens: 7 },
    },
  ]);
});

test("runtime metrics mark individual missing usage fields incomplete", () => {
  const metrics = new RuntimeMetrics(() => 0);
  const first = metrics.beginRequest("investigating");
  const firstAttempt = metrics.beginAttempt();
  metrics.finishAttempt(firstAttempt);
  metrics.recordCompletion(first, {
    choices: [{ finish_reason: "stop" }],
    usage: { prompt_tokens: 4, completion_tokens: 3, total_tokens: 7 },
  });
  const second = metrics.beginRequest("investigating");
  const secondAttempt = metrics.beginAttempt();
  metrics.finishAttempt(secondAttempt);
  metrics.recordCompletion(second, {
    choices: [{ finish_reason: "stop" }],
    usage: { prompt_tokens: 6 },
  });

  assert.deepEqual(metrics.snapshot().tokenUsage, {
    complete: false,
    knownAttemptCount: 2,
    unknownAttemptCount: 0,
    inputTokens: 10,
    outputTokens: 3,
    totalTokens: 7,
  });
});

test("provider attempts include full response-body consumption time", async () => {
  const metrics = new RuntimeMetrics();
  const client = createProviderClient(
    BaseClient,
    { timeout: 1_000 },
    metrics,
    async () => new Response(new ReadableStream({
      start(controller) {
        setTimeout(() => {
          controller.enqueue(new TextEncoder().encode("{}"));
          controller.close();
        }, 20);
      },
    }), { status: 200 }),
  );
  const request = metrics.beginRequest("investigating");
  const response = await client.options.fetch("https://provider.example/v1");
  assert.equal(metrics.snapshot().providerAttempts[0].durationMs < 20, true);
  await response.text();
  metrics.recordCompletion(request, { choices: [{ finish_reason: "stop" }] });
  assert.equal(metrics.snapshot().providerAttempts[0].durationMs >= 20, true);
});
