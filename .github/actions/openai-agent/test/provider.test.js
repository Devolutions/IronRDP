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
});

test("SDK adapter honors valid Retry-After without another retry budget", async () => {
  const delays = [];
  const client = createProviderClient(
    BaseClient,
    {},
    new RuntimeMetrics(),
    globalThis.fetch,
    async (milliseconds) => { delays.push(milliseconds); },
  );
  client.makeRequest = async (options, retriesRemaining, requestLogID) => ({
    options, retriesRemaining, requestLogID,
  });
  const result = await client.retryRequest({}, 4, "request", new Headers({ "retry-after": "61" }));
  assert.deepEqual(delays, [61_000]);
  assert.deepEqual(result, { options: {}, retriesRemaining: 3, requestLogID: "request" });
  assert.equal(retryAfterMilliseconds(new Headers({ "retry-after-ms": "250" })), 250);
  assert.equal(
    retryAfterMilliseconds(new Headers({ "retry-after": "600" }), 120_000),
    120_000,
  );
  assert.equal(retryAfterMilliseconds(new Headers({ "retry-after": "invalid" })), undefined);
});

test("runtime metrics retain activity and mark partial usage incomplete", async () => {
  const metrics = new RuntimeMetrics(() => 0);
  const request = metrics.beginRequest("repairing");
  const first = metrics.beginAttempt();
  metrics.finishAttempt(first, new Response("", { status: 429 }));
  const second = metrics.beginAttempt();
  metrics.finishAttempt(second, new Response("", {
    status: 200,
    headers: { "x-request-id": "req_safe-123" },
  }));
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
  assert.deepEqual(snapshot.diagnostics.providerAttempts, [
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
