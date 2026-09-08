"use strict";

const OpenAI = require("openai");

const { AgentFailure, providerFailureDiagnostic, runAgent } = require("./agent");
const { loadConfiguration, validateBaseUrl } = require("./config");
const { ActionError } = require("./errors");
const {
  DEFAULT_OUTPUT_REPAIRS, DEFAULT_REQUEST_RETRIES, DEFAULT_REQUEST_TIMEOUT_MS,
  MAX_MODEL_OUTPUT_BYTES, MAX_OUTPUT_REPAIRS, MAX_REQUEST_RETRIES, MAX_REQUEST_TIMEOUT_MS,
  MAX_TOOL_CALLS, MAX_TURNS,
} = require("./limits");
const { RuntimeMetrics, createProviderClient } = require("./provider");
const { ValidatorFailure, loadValidator, parseMetadata } = require("./validator");

async function main(core, environment = process.env, OpenAIClient = OpenAI) {
  let apiKey = "";
  let turnCount = 0;
  let toolCallCount = 0;
  let outputRepairCount = 0;
  let phase = "input";
  const metrics = new RuntimeMetrics();
  setOutputs(core, {
    output: "", failureReason: "", failureCategory: "", retryable: false,
    turnCount, toolCallCount, outputRepairCount, metrics,
  });

  try {
    apiKey = requiredInput(core, "api-key", "api key input is missing", false);
    core.setSecret(apiKey);

    const baseUrlInput = requiredInput(core, "base-url", "base URL input is missing");
    const configFile = requiredInput(core, "config-file", "config file input is missing");
    const inputLimits = readInputLimits(core);
    const validatorSelector = core.getInput("validator");
    const validatorMetadata = parseMetadata(core.getInput("validator-metadata"));
    if (validatorSelector === "" && Object.keys(validatorMetadata).length !== 0) {
      throw new ActionError("validator metadata requires a validator", "input");
    }
    phase = "configuration";
    const baseURL = validateBaseUrl(baseUrlInput);
    const workspace = environment.GITHUB_WORKSPACE;
    if (typeof workspace !== "string" || workspace.length === 0) {
      throw new ActionError("workspace is unavailable");
    }
    const loaded = loadConfiguration(workspace, configFile);
    const config = resolveConfiguration(loaded.config, inputLimits);
    const validator = loadValidator(workspace, validatorSelector, validatorMetadata);
    core.info(JSON.stringify({
      event: "openai-agent.start",
      id: config.id,
      model: config.model,
      maxTurns: config.max_turns,
      maxToolCalls: config.max_tool_calls,
    }));

    phase = "initialization";
    let client;
    try {
      client = createProviderClient(OpenAIClient, {
        apiKey,
        baseURL,
        maxRetries: config.max_request_retries,
        timeout: config.request_timeout_ms,
        fetchOptions: { redirect: "error" },
      }, metrics);
    } catch {
      throw new ActionError("provider client initialization failed", "initialization");
    }
    phase = "runtime";
    const result = await runAgent({ client, ...loaded, config, validator, metrics });
    turnCount = result.turnCount;
    toolCallCount = result.toolCallCount;
    outputRepairCount = result.outputRepairCount;
    setOutputs(core, {
      output: result.output,
      failureReason: "",
      failureCategory: "",
      retryable: false,
      turnCount, toolCallCount, outputRepairCount, metrics,
    });
    core.info(JSON.stringify({
      event: "openai-agent.complete",
      id: config.id,
      model: config.model,
      turnCount,
      toolCallCount,
      outputRepairCount,
      outputBytes: Buffer.byteLength(result.output, "utf8"),
    }));
  } catch (error) {
    const outcome = failure(error, phase);
    const failureReason = outcome.reason;
    if (error instanceof AgentFailure) {
      turnCount = error.turnCount;
      toolCallCount = error.toolCallCount;
      outputRepairCount = error.outputRepairCount;
      const diagnostic = providerFailureDiagnostic(error.cause);
      if (error.cause) {
        core.info(JSON.stringify({
          event: "openai-agent.provider-failure",
          reason: failureReason,
          category: outcome.category,
          retryable: outcome.retryable,
          ...(diagnostic || {}),
        }));
      } else {
        logActionFailure(core, "runtime", outcome);
      }
    } else {
      logActionFailure(core, error instanceof ActionError ? error.phase : phase, outcome);
    }
    setOutputs(core, {
      output: "", failureReason, failureCategory: outcome.category, retryable: outcome.retryable,
      turnCount, toolCallCount, outputRepairCount, metrics,
    });
    core.setFailed(failureReason);
  } finally {
    apiKey = "";
  }
}

function requiredInput(core, name, failureReason, trimWhitespace = true) {
  const value = core.getInput(name, { trimWhitespace });
  if (value.length === 0) throw new ActionError(failureReason, "input");
  return value;
}

function failure(error, phase) {
  if (error instanceof AgentFailure) {
    return { reason: error.reason, category: error.category, retryable: error.retryable };
  }
  if (error instanceof ValidatorFailure) {
    return { reason: error.reason, category: error.category, retryable: false };
  }
  if (error instanceof ActionError) {
    return {
      reason: error.code,
      category: error.phase === "input" ? "input" : "configuration",
      retryable: false,
    };
  }
  switch (phase) {
    case "input": return { reason: "action input failed", category: "input", retryable: false };
    case "configuration": return { reason: "action configuration failed", category: "configuration", retryable: false };
    case "initialization": return { reason: "provider client initialization failed", category: "initialization", retryable: false };
    default: return { reason: "action runtime failed", category: "runtime", retryable: false };
  }
}

function logActionFailure(core, phase, outcome) {
  core.info(JSON.stringify({
    event: "openai-agent.failure",
    phase,
    reason: outcome.reason,
    category: outcome.category,
    retryable: outcome.retryable,
  }));
}

function readInputLimits(core) {
  return {
    requestTimeout: optionalPositiveInput(
      core, "request-timeout-ms", 1, MAX_REQUEST_TIMEOUT_MS,
    ),
    requestRetries: optionalPositiveInput(
      core, "max-request-retries", 0, MAX_REQUEST_RETRIES,
    ),
    modelTurns: optionalPositiveInput(core, "max-model-turns", 1, MAX_TURNS),
    toolCalls: optionalPositiveInput(core, "max-tool-calls", 0, MAX_TOOL_CALLS),
    outputBytes: optionalPositiveInput(core, "max-output-bytes", 1024, MAX_MODEL_OUTPUT_BYTES),
    outputRepairs: optionalPositiveInput(core, "max-output-repairs", 0, MAX_OUTPUT_REPAIRS),
  };
}

function optionalPositiveInput(core, name, minimum, maximum) {
  const value = core.getInput(name);
  if (value === "") return undefined;
  if (!/^\d+$/.test(value)) throw new ActionError(`invalid ${name} input`, "input");
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed < minimum || parsed > maximum) {
    throw new ActionError(`invalid ${name} input`, "input");
  }
  return parsed;
}

function resolveConfiguration(config, inputLimits) {
  return {
    ...config,
    request_timeout_ms: inputLimits.requestTimeout ??
      config.request_timeout_ms ?? DEFAULT_REQUEST_TIMEOUT_MS,
    max_request_retries: inputLimits.requestRetries ??
      config.max_request_retries ?? DEFAULT_REQUEST_RETRIES,
    max_turns: inputLimits.modelTurns ?? config.max_turns,
    max_tool_calls: inputLimits.toolCalls ?? config.max_tool_calls,
    max_output_bytes: inputLimits.outputBytes ?? config.max_output_bytes,
    max_output_repair_attempts: inputLimits.outputRepairs ??
      config.max_output_repair_attempts ?? DEFAULT_OUTPUT_REPAIRS,
    output_format: config.output_format || "json_object",
  };
}

function setOutputs(core, {
  output, failureReason, failureCategory, retryable, turnCount, toolCallCount, outputRepairCount, metrics,
}) {
  const runtime = metrics.snapshot();
  const diagnostics = {
    activity: runtime.activity,
    durationMs: runtime.durationMs,
    requestRetryCount: runtime.requestRetryCount,
    outputRepairCount,
    providerFinishReason: runtime.providerFinishReason || null,
    tokenUsage: runtime.tokenUsage,
    turnCount,
    toolCallCount,
    failureCategory: failureCategory || null,
    retryable,
    providerAttempts: runtime.diagnostics.providerAttempts,
  };
  core.setOutput("structured-output", output);
  core.setOutput("failure-reason", failureReason);
  core.setOutput("turn-count", String(turnCount));
  core.setOutput("tool-call-count", String(toolCallCount));
  core.setOutput("activity", runtime.activity);
  core.setOutput("duration-ms", String(runtime.durationMs));
  core.setOutput("request-retry-count", String(runtime.requestRetryCount));
  core.setOutput("output-repair-count", String(outputRepairCount));
  core.setOutput("provider-finish-reason", runtime.providerFinishReason);
  core.setOutput("token-usage", JSON.stringify(runtime.tokenUsage));
  core.setOutput("diagnostics", JSON.stringify(diagnostics));
  core.setOutput("failure-category", failureCategory);
  core.setOutput("retryable", String(retryable));
}

module.exports = {
  main, optionalPositiveInput, readInputLimits, resolveConfiguration, setOutputs,
};
