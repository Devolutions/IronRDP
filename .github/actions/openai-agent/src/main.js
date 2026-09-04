"use strict";

const OpenAI = require("openai");

const { AgentFailure, providerFailureDiagnostic, runAgent } = require("./agent");
const { loadConfiguration, validateBaseUrl } = require("./config");
const { ActionError } = require("./errors");
const { REQUEST_RETRIES, REQUEST_TIMEOUT_MS } = require("./limits");

async function main(core, environment = process.env, OpenAIClient = OpenAI) {
  let apiKey = "";
  let turnCount = 0;
  let toolCallCount = 0;
  let phase = "input";
  setOutputs(core, { output: "", failureReason: "", turnCount, toolCallCount });

  try {
    apiKey = requiredInput(core, "api-key", "api key input is missing", false);
    core.setSecret(apiKey);

    const baseUrlInput = requiredInput(core, "base-url", "base URL input is missing");
    const configFile = requiredInput(core, "config-file", "config file input is missing");
    phase = "configuration";
    const baseURL = validateBaseUrl(baseUrlInput);
    const workspace = environment.GITHUB_WORKSPACE;
    if (typeof workspace !== "string" || workspace.length === 0) {
      throw new ActionError("workspace is unavailable");
    }
    const loaded = loadConfiguration(workspace, configFile);
    core.info(JSON.stringify({
      event: "openai-agent.start",
      id: loaded.config.id,
      model: loaded.config.model,
      maxTurns: loaded.config.max_turns,
      maxToolCalls: loaded.config.max_tool_calls,
    }));

    phase = "initialization";
    let client;
    try {
      client = new OpenAIClient({
        apiKey,
        baseURL,
        maxRetries: REQUEST_RETRIES,
        timeout: REQUEST_TIMEOUT_MS,
        fetchOptions: { redirect: "error" },
      });
    } catch {
      throw new ActionError("provider client initialization failed", "initialization");
    }
    phase = "runtime";
    const result = await runAgent({ client, ...loaded });
    turnCount = result.turnCount;
    toolCallCount = result.toolCallCount;
    setOutputs(core, {
      output: result.output,
      failureReason: "",
      turnCount,
      toolCallCount,
    });
    core.info(JSON.stringify({
      event: "openai-agent.complete",
      id: loaded.config.id,
      model: loaded.config.model,
      turnCount,
      toolCallCount,
      outputBytes: Buffer.byteLength(result.output, "utf8"),
    }));
  } catch (error) {
    const failureReason = failure(error, phase);
    if (error instanceof AgentFailure) {
      turnCount = error.turnCount;
      toolCallCount = error.toolCallCount;
      const diagnostic = providerFailureDiagnostic(error.cause);
      if (error.cause) {
        core.info(JSON.stringify({
          event: "openai-agent.provider-failure",
          reason: failureReason,
          ...(diagnostic || {}),
        }));
      } else {
        logActionFailure(core, "runtime", failureReason);
      }
    } else {
      logActionFailure(core, error instanceof ActionError ? error.phase : phase, failureReason);
    }
    setOutputs(core, { output: "", failureReason, turnCount, toolCallCount });
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
  if (error instanceof AgentFailure) return error.reason;
  if (error instanceof ActionError) return error.code;
  switch (phase) {
    case "input": return "action input failed";
    case "configuration": return "action configuration failed";
    case "initialization": return "provider client initialization failed";
    default: return "action runtime failed";
  }
}

function logActionFailure(core, phase, reason) {
  core.info(JSON.stringify({
    event: "openai-agent.failure",
    phase,
    reason,
  }));
}

function setOutputs(core, { output, failureReason, turnCount, toolCallCount }) {
  core.setOutput("structured-output", output);
  core.setOutput("failure-reason", failureReason);
  core.setOutput("turn-count", String(turnCount));
  core.setOutput("tool-call-count", String(toolCallCount));
}

module.exports = { main, setOutputs };
