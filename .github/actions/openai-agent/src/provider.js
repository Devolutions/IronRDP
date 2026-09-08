"use strict";

const { MAX_PROVIDER_ERROR_BYTES } = require("./limits");

const KNOWN_QUOTA_CODES = new Set([
  "billing_hard_limit_reached",
  "insufficient_quota",
  "quota_exceeded",
  "quota_exhausted",
]);
const SAFE_DIAGNOSTIC_VALUE = /^[A-Za-z0-9._:-]{1,128}$/;
const MAX_TIMEOUT = 2_147_483_647;

class RuntimeMetrics {
  constructor(now = Date.now) {
    this.now = now;
    this.startedAt = now();
    this.activity = "input";
    this.requests = [];
    this.activeRequest = null;
  }

  beginRequest(activity) {
    this.activity = activity;
    const request = { activity, attempts: [] };
    this.requests.push(request);
    this.activeRequest = request;
    return request;
  }

  beginAttempt() {
    const attempt = {
      activity: this.activeRequest?.activity || this.activity,
      startedAt: this.now(),
    };
    this.activeRequest?.attempts.push(attempt);
    return attempt;
  }

  finishAttempt(attempt, response) {
    attempt.durationMs = Math.max(0, this.now() - attempt.startedAt);
    if (response && Number.isInteger(response.status) &&
        response.status >= 100 && response.status <= 599) {
      attempt.status = response.status;
    }
    const requestId = response?.headers?.get?.("x-request-id") ||
      response?.headers?.get?.("request-id");
    if (typeof requestId === "string" && SAFE_DIAGNOSTIC_VALUE.test(requestId)) {
      attempt.requestId = requestId;
    }
  }

  recordCompletion(request, response) {
    const attempt = request.attempts.at(-1);
    if (!attempt) return;
    const finishReason = response?.choices?.[0]?.finish_reason;
    if (typeof finishReason === "string" && SAFE_DIAGNOSTIC_VALUE.test(finishReason)) {
      attempt.finishReason = finishReason;
    }
    const usage = normalizeUsage(response?.usage);
    if (usage) attempt.usage = usage;
  }

  snapshot() {
    const providerAttempts = this.requests.flatMap((request) => request.attempts.map((attempt) => ({
      activity: attempt.activity,
      durationMs: attempt.durationMs ?? Math.max(0, this.now() - attempt.startedAt),
      ...(attempt.status === undefined ? {} : { status: attempt.status }),
      ...(attempt.requestId === undefined ? {} : { requestId: attempt.requestId }),
      ...(attempt.finishReason === undefined ? {} : { finishReason: attempt.finishReason }),
      ...(attempt.usage === undefined ? {} : { usage: attempt.usage }),
    })));
    const usage = summarizeUsage(providerAttempts);
    const finishReason = [...providerAttempts].reverse()
      .find((attempt) => attempt.finishReason !== undefined)?.finishReason || "";
    return {
      activity: this.activity,
      durationMs: Math.max(0, this.now() - this.startedAt),
      requestRetryCount: this.requests.reduce(
        (count, request) => count + Math.max(0, request.attempts.length - 1), 0,
      ),
      providerFinishReason: finishReason,
      tokenUsage: usage,
      diagnostics: { providerAttempts, tokenUsage: usage },
    };
  }
}

function createProviderClient(OpenAIClient, options, metrics, fetch = globalThis.fetch, sleep = delay) {
  const instrumentedFetch = async (...args) => {
    const attempt = metrics.beginAttempt();
    try {
      const response = await fetch(...args);
      metrics.finishAttempt(attempt, response);
      return response;
    } catch (error) {
      metrics.finishAttempt(attempt);
      throw error;
    }
  };
  const client = new OpenAIClient({ ...options, fetch: instrumentedFetch });
  if (typeof client.shouldRetry === "function") {
    const shouldRetry = client.shouldRetry.bind(client);
    client.shouldRetry = async (response) => {
      if (response?.status === 429 && await hasKnownQuotaCode(response)) return false;
      return shouldRetry(response);
    };
  }
  if (typeof client.retryRequest === "function") {
    const retryRequest = client.retryRequest.bind(client);
    client.retryRequest = async (requestOptions, retriesRemaining, requestLogID, responseHeaders) => {
      const retryAfter = retryAfterMilliseconds(responseHeaders);
      if (retryAfter === undefined) {
        return retryRequest(requestOptions, retriesRemaining, requestLogID, responseHeaders);
      }
      await sleep(retryAfter);
      return client.makeRequest(requestOptions, retriesRemaining - 1, requestLogID);
    };
  }
  return client;
}

async function hasKnownQuotaCode(response) {
  let text;
  try {
    text = await readBoundedBody(response.clone());
  } catch {
    return false;
  }
  if (text === null) return false;
  try {
    const body = JSON.parse(text);
    const error = body?.error && typeof body.error === "object" ? body.error : body;
    return [error?.code, error?.type].some((value) =>
      typeof value === "string" && KNOWN_QUOTA_CODES.has(value));
  } catch {
    return false;
  }
}

async function readBoundedBody(response) {
  const reader = response.body?.getReader?.();
  if (!reader) return null;
  const chunks = [];
  let length = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      length += value.byteLength;
      if (length > MAX_PROVIDER_ERROR_BYTES) return null;
      chunks.push(value);
    }
    return new TextDecoder("utf-8", { fatal: true }).decode(concatenate(chunks, length));
  } finally {
    await reader.cancel().catch(() => undefined);
  }
}

function concatenate(chunks, length) {
  const combined = new Uint8Array(length);
  let offset = 0;
  for (const chunk of chunks) {
    combined.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return combined;
}

function retryAfterMilliseconds(headers) {
  const milliseconds = parseDelay(headers?.get?.("retry-after-ms"), 1);
  if (milliseconds !== undefined) return milliseconds;
  const retryAfter = headers?.get?.("retry-after");
  if (typeof retryAfter !== "string") return undefined;
  const seconds = parseDelay(retryAfter, 1000);
  if (seconds !== undefined) return seconds;
  const date = Date.parse(retryAfter);
  return Number.isFinite(date) && date >= Date.now() ? date - Date.now() : undefined;
}

function parseDelay(value, multiplier) {
  if (typeof value !== "string" || !/^\d+(?:\.\d+)?$/.test(value)) return undefined;
  const delay = Number(value) * multiplier;
  return Number.isFinite(delay) && delay >= 0 ? delay : undefined;
}

async function delay(milliseconds) {
  let remaining = milliseconds;
  while (remaining > 0) {
    const chunk = Math.min(remaining, MAX_TIMEOUT);
    await new Promise((resolve) => setTimeout(resolve, chunk));
    remaining -= chunk;
  }
}

function normalizeUsage(usage) {
  if (usage === null || typeof usage !== "object" || Array.isArray(usage)) return null;
  const inputTokens = safeTokenCount(usage.prompt_tokens ?? usage.input_tokens);
  const outputTokens = safeTokenCount(usage.completion_tokens ?? usage.output_tokens);
  const totalTokens = safeTokenCount(usage.total_tokens);
  if (inputTokens === undefined && outputTokens === undefined && totalTokens === undefined) return null;
  return {
    ...(inputTokens === undefined ? {} : { inputTokens }),
    ...(outputTokens === undefined ? {} : { outputTokens }),
    ...(totalTokens === undefined ? {} : { totalTokens }),
  };
}

function safeTokenCount(value) {
  return Number.isSafeInteger(value) && value >= 0 ? value : undefined;
}

function summarizeUsage(attempts) {
  const known = attempts.filter((attempt) => attempt.usage !== undefined);
  const summary = {
    complete: attempts.length > 0 && known.length === attempts.length,
    knownAttemptCount: known.length,
    unknownAttemptCount: attempts.length - known.length,
  };
  for (const field of ["inputTokens", "outputTokens", "totalTokens"]) {
    const values = known.map((attempt) => attempt.usage[field]).filter((value) => value !== undefined);
    if (values.length !== 0) summary[field] = values.reduce((total, value) => total + value, 0);
  }
  return summary;
}

module.exports = {
  RuntimeMetrics, createProviderClient, hasKnownQuotaCode, retryAfterMilliseconds,
};
