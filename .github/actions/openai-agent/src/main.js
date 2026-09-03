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
  setOutputs(core, { output: "", failureReason: "", turnCount, toolCallCount });

  try {
    apiKey = core.getInput("api-key", { required: true, trimWhitespace: false });
    core.setSecret(apiKey);

    const baseURL = validateBaseUrl(core.getInput("base-url", { required: true }));
    const configFile = core.getInput("config-file", { required: true });
    const workspace = environment.GITHUB_WORKSPACE;
    const loaded = loadConfiguration(workspace, configFile);
    core.info(JSON.stringify({
      event: "openai-agent.start",
      id: loaded.config.id,
      model: loaded.config.model,
      maxTurns: loaded.config.max_turns,
      maxToolCalls: loaded.config.max_tool_calls,
    }));

    const client = new OpenAIClient({
      apiKey,
      baseURL,
      maxRetries: REQUEST_RETRIES,
      timeout: REQUEST_TIMEOUT_MS,
      fetchOptions: { redirect: "error" },
    });
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
    const failureReason = error instanceof AgentFailure
      ? error.reason
      : error instanceof ActionError ? error.code : "action failed";
    if (error instanceof AgentFailure) {
      turnCount = error.turnCount;
      toolCallCount = error.toolCallCount;
      const diagnostic = providerFailureDiagnostic(error.cause);
      if (diagnostic) {
        core.info(JSON.stringify({
          event: "openai-agent.provider-failure",
          ...diagnostic,
        }));
      }
    }
    setOutputs(core, { output: "", failureReason, turnCount, toolCallCount });
    core.setFailed(failureReason);
  } finally {
    apiKey = "";
  }
}

function setOutputs(core, { output, failureReason, turnCount, toolCallCount }) {
  core.setOutput("structured-output", output);
  core.setOutput("failure-reason", failureReason);
  core.setOutput("turn-count", String(turnCount));
  core.setOutput("tool-call-count", String(toolCallCount));
}

module.exports = { main, setOutputs };
