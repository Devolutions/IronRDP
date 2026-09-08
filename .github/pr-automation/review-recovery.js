"use strict";

const { SHA, exactKeys, isBoundedArray, isPlainObject, normalizeText, parseJson } = require("./validation");

const MAX_STAGE_COUNT = 32;
const METRIC_KEYS = ["tokens", "elapsed_ms", "request_retries", "output_repairs", "stage_recoveries"];

function validMetric(value) {
  return value === null || (typeof value === "number" && Number.isFinite(value) && value >= 0);
}

function normalizeMetrics(value) {
  if (!exactKeys(value, METRIC_KEYS) || METRIC_KEYS.some((key) => !validMetric(value[key]))) return null;
  return Object.fromEntries(METRIC_KEYS.map((key) => [key, value[key]]));
}

function normalizeStage(value) {
  const keys = [
    "id", "status", "required", "reason", "failure_category", "retryable", "reused",
    "reused_from_run_id", "metrics",
  ];
  if (!exactKeys(value, keys) || typeof value.id !== "string" || !/^[a-z][a-z0-9-]{0,63}$/.test(value.id) ||
      !["success", "failed", "skipped"].includes(value.status) ||
      typeof value.required !== "boolean" || typeof value.retryable !== "boolean" ||
      typeof value.reused !== "boolean" || typeof value.failure_category !== "string" ||
      !/^[a-z][a-z0-9-]{0,63}$/.test(value.failure_category) ||
      !(value.reused_from_run_id === null ||
      (Number.isSafeInteger(value.reused_from_run_id) && value.reused_from_run_id > 0))) return null;
  const reason = normalizeText(value.reason, 300);
  const metrics = normalizeMetrics(value.metrics);
  if (reason === null || !metrics || (value.status === "success" && (reason !== "" || value.retryable)) ||
      (!value.reused && value.reused_from_run_id !== null) ||
      (value.reused && value.reused_from_run_id === null)) return null;
  return {
    id: value.id,
    status: value.status,
    required: value.required,
    reason,
    failureCategory: value.failure_category,
    retryable: value.retryable,
    reused: value.reused,
    reusedFromRunId: value.reused_from_run_id,
    metrics,
  };
}

function normalizeProvenance(value, {
  expectedHeadSha, expectedBaseSha, expectedRecoveryAttempt,
} = {}) {
  const keys = [
    "v", "run_id", "run_attempt", "attempt_id", "base_sha", "head_sha", "evidence_digest",
    "policy_digest", "corpus_sha", "artifacts",
  ];
  const artifacts = value?.artifacts;
  if (!exactKeys(value, keys) || value.v !== 1 || !/^[1-9]\d*$/.test(value.run_id) ||
      !/^[1-9]\d*$/.test(value.run_attempt) ||
      !SHA.test(value.head_sha) ||
      value.attempt_id !== `${value.head_sha}-r${expectedRecoveryAttempt}-a${value.run_attempt}-${value.run_id}` ||
      !SHA.test(value.base_sha) || !SHA.test(value.head_sha) ||
      !/^[0-9a-f]{64}$/.test(value.evidence_digest) || !/^[0-9a-f]{64}$/.test(value.policy_digest) ||
      !(value.corpus_sha === null || SHA.test(value.corpus_sha)) ||
      !exactKeys(artifacts, ["evidence", "validation", "corpus", "aggregate", "general", "specialists"]) ||
      !["evidence", "validation", "aggregate"].every((key) =>
        typeof artifacts[key] === "string" && /^[a-z0-9][a-z0-9-]{0,127}$/.test(artifacts[key])) ||
      !["corpus", "general"].every((key) =>
        artifacts[key] === null || (typeof artifacts[key] === "string" &&
        /^[a-z0-9][a-z0-9-]{0,127}$/.test(artifacts[key]))) ||
      !isPlainObject(artifacts.specialists) ||
      Object.entries(artifacts.specialists).some(([id, name]) =>
        !/^[a-z][a-z0-9-]{0,63}$/.test(id) ||
        typeof name !== "string" || !/^[a-z0-9][a-z0-9-]{0,127}$/.test(name)) ||
      (expectedHeadSha && value.head_sha !== expectedHeadSha) ||
      (expectedBaseSha && value.base_sha !== expectedBaseSha) ||
      (!Number.isSafeInteger(expectedRecoveryAttempt) || expectedRecoveryAttempt < 0 ||
      expectedRecoveryAttempt > 1)) return null;
  return {
    value,
    runId: value.run_id,
    runAttempt: value.run_attempt,
    baseSha: value.base_sha,
    headSha: value.head_sha,
    evidenceDigest: value.evidence_digest,
    policyDigest: value.policy_digest,
    corpusSha: value.corpus_sha,
  };
}

function parsePipelineRecovery({
  stages, metrics, provenance, recoverable, expectedHeadSha, expectedBaseSha,
  expectedRecoveryAttempt,
} = {}) {
  const parsedStages = parseJson(stages, 128 * 1024);
  const parsedMetrics = parseJson(metrics, 32 * 1024);
  const parsedProvenance = parseJson(provenance, 4096);
  if (!isBoundedArray(parsedStages, MAX_STAGE_COUNT) || !isPlainObject(parsedMetrics) ||
      !expectedHeadSha || !expectedBaseSha) {
    return { ok: false, reason: "review pipeline recovery data is unavailable" };
  }
  const normalizedStages = parsedStages.map(normalizeStage);
  if (normalizedStages.some((stage) => stage === null) ||
      new Set(normalizedStages.map((stage) => stage.id)).size !== normalizedStages.length) {
    return { ok: false, reason: "review pipeline stage data is invalid" };
  }
  const normalizedProvenance = normalizeProvenance(parsedProvenance, {
    expectedHeadSha, expectedBaseSha, expectedRecoveryAttempt,
  });
  if (!normalizedProvenance) return { ok: false, reason: "review pipeline provenance is invalid" };
  if (!["true", "false"].includes(recoverable)) {
    return { ok: false, reason: "review pipeline recoverability is invalid" };
  }
  return {
    ok: true,
    value: {
      stages: normalizedStages,
      metrics: parsedMetrics,
      provenance: normalizedProvenance,
      recoverable: recoverable === "true",
    },
  };
}

function recoveryDecision({ pipeline, attempt } = {}) {
  if (!pipeline?.ok || !Number.isSafeInteger(attempt) || attempt < 1 || attempt > 2) {
    return { status: "exhausted", reason: pipeline?.reason || "review pipeline recovery data is unavailable" };
  }
  const failedRequired = pipeline.value.stages.filter((stage) => stage.required && stage.status === "failed");
  const terminalRequired = failedRequired.find((stage) => !stage.retryable);
  if (terminalRequired) {
    return { status: "exhausted", reason: terminalRequired.reason || "required review stage failed" };
  }
  const temporaryRequired = failedRequired.find((stage) => stage.retryable);
  if (!temporaryRequired) {
    return { status: "complete", reason: "" };
  }
  if (!pipeline.value.recoverable || attempt >= 2) {
    return { status: "exhausted", reason: temporaryRequired.reason || "required review stage recovery exhausted" };
  }
  return { status: "pending", reason: temporaryRequired.reason || "required review stage retry is pending" };
}

function aggregateStageMetrics(pipelines) {
  const totals = Object.fromEntries(METRIC_KEYS.map((key) => [key, 0]));
  const unavailable = new Set();
  const stages = new Map();
  let attempts = 0;
  let reused = 0;
  for (const pipeline of pipelines || []) {
    if (!pipeline?.ok) {
      METRIC_KEYS.forEach((key) => unavailable.add(key));
      continue;
    }
    for (const stage of pipeline.value.stages) {
      attempts += 1;
      reused += Number(stage.reused);
      const current = stages.get(stage.id) || {
        attempts: 0,
        reused: 0,
        failures: [],
        metrics: Object.fromEntries(METRIC_KEYS.map((key) => [key, 0])),
        unavailable: [],
      };
      current.attempts += 1;
      current.reused += Number(stage.reused);
      if (stage.status === "failed") current.failures.push(stage.reason || stage.failureCategory);
      if (!stage.reused) {
        for (const key of METRIC_KEYS) {
          if (stage.metrics[key] === null) {
            unavailable.add(key);
            if (!current.unavailable.includes(key)) current.unavailable.push(key);
          } else {
            totals[key] += stage.metrics[key];
            current.metrics[key] += stage.metrics[key];
          }
        }
      }
      stages.set(stage.id, current);
    }
  }
  for (const key of unavailable) totals[key] = null;
  return {
    attempts,
    reused,
    totals,
    stages: Object.fromEntries([...stages.entries()].sort(([left], [right]) => left.localeCompare(right))),
  };
}

module.exports = {
  METRIC_KEYS, aggregateStageMetrics, parsePipelineRecovery, recoveryDecision,
};
