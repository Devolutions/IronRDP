"use strict";

const Ajv = require("ajv");

const { fail } = require("./errors");
const {
  MAX_CONFIG_BYTES, MAX_METHODOLOGY_BYTES, MAX_METHODOLOGY_TOTAL_BYTES, MAX_PROMPT_BYTES,
  MAX_MODEL_OUTPUT_BYTES, MAX_SCHEMA_BYTES, MAX_TOOL_CALLS, MAX_TURNS,
} = require("./limits");
const { WorkspaceSandbox } = require("./sandbox");

const SAFE_ID = /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/;
const SAFE_MODEL = /^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$/;

const CONFIG_SCHEMA = {
  type: "object",
  additionalProperties: false,
  required: [
    "id", "model", "prompt_file", "schema_file", "allowed_roots", "allowed_files",
    "max_output_bytes", "max_turns", "max_tool_calls",
  ],
  properties: {
    id: { type: "string", pattern: SAFE_ID.source },
    model: { type: "string", pattern: SAFE_MODEL.source },
    prompt_file: { type: "string", minLength: 1 },
    schema_file: { type: "string", minLength: 1 },
    methodology_files: {
      type: "array",
      maxItems: 32,
      items: { type: "string", minLength: 1 },
    },
    allowed_roots: {
      type: "array",
      maxItems: 32,
      uniqueItems: true,
      items: { type: "string", minLength: 1 },
    },
    allowed_files: {
      type: "array",
      maxItems: 256,
      uniqueItems: true,
      items: { type: "string", minLength: 1 },
    },
    max_output_bytes: { type: "integer", minimum: 1024, maximum: MAX_MODEL_OUTPUT_BYTES },
    max_turns: { type: "integer", minimum: 1, maximum: MAX_TURNS },
    max_tool_calls: { type: "integer", minimum: 0, maximum: MAX_TOOL_CALLS },
  },
};

function parseJson(text, code) {
  try {
    return JSON.parse(text);
  } catch {
    fail(code);
  }
}

function loadConfiguration(workspace, configFile) {
  const workspaceReader = new WorkspaceSandbox(workspace);
  const rawConfig = workspaceReader.readWorkflowFile(configFile, MAX_CONFIG_BYTES);
  const config = parseJson(rawConfig, "configuration is not valid JSON");
  const validate = new Ajv({ allErrors: true, strict: true }).compile(CONFIG_SCHEMA);
  if (!validate(config)) fail("configuration does not match its schema");
  if (config.max_tool_calls > 0 &&
      config.allowed_roots.length === 0 && config.allowed_files.length === 0) {
    fail("configuration grants no filesystem capabilities");
  }
  const sandbox = new WorkspaceSandbox(workspace, {
    allowedRoots: config.allowed_roots,
    allowedFiles: config.allowed_files,
  });
  const prompt = workspaceReader.readWorkflowFile(config.prompt_file, MAX_PROMPT_BYTES);
  const schemaText = workspaceReader.readWorkflowFile(config.schema_file, MAX_SCHEMA_BYTES);
  const schema = parseJson(schemaText, "output schema is not valid JSON");
  if (schema === null || typeof schema !== "object" || Array.isArray(schema)) {
    fail("output schema must be an object");
  }

  const methodologies = [];
  let methodologyBytes = 0;
  for (const file of config.methodology_files || []) {
    const content = workspaceReader.readWorkflowFile(file, MAX_METHODOLOGY_BYTES);
    methodologyBytes += Buffer.byteLength(content, "utf8");
    if (methodologyBytes > MAX_METHODOLOGY_TOTAL_BYTES) fail("methodology files exceed byte limit");
    methodologies.push(content);
  }

  return { config, methodologies, prompt, sandbox, schema };
}

function validateBaseUrl(raw) {
  if (typeof raw !== "string" || raw.length === 0 || raw.length > 2048) fail("invalid base URL");
  let url;
  try {
    url = new URL(raw);
  } catch {
    fail("invalid base URL");
  }
  const localHttp = url.protocol === "http:" &&
    ["localhost", "127.0.0.1", "[::1]"].includes(url.hostname);
  if (url.protocol !== "https:" && !localHttp) fail("base URL must use HTTPS");
  if (url.username || url.password || url.search || url.hash) {
    fail("base URL must not contain credentials, a query, or a fragment");
  }
  return url.toString().replace(/\/$/, "");
}

module.exports = { CONFIG_SCHEMA, loadConfiguration, validateBaseUrl };
