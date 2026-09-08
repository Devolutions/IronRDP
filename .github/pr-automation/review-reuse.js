"use strict";

// Trust rules for consuming a previous attempt's artifacts.
//
// A matching content digest proves an artifact is self-consistent, not that a trusted job produced
// it. Any workflow run in this repository can publish an artifact under any name, so the producing
// run is authenticated first: same repository, a trusted triggering event, the same caller workflow
// file, and — when the server reports it — this reusable workflow referenced from a trusted ref.
//
// An in-progress run is accepted, because a recovery round may happen inside the run that produced
// the results it is recovering from.

const { normalizeText } = require("./validation");

const SHA = /^[0-9a-f]{40}$/;
const DIGEST = /^[0-9a-f]{64}$/;
const RUN_ID = /^[0-9]{1,20}$/;
const ARTIFACT_NAME = /^[A-Za-z0-9][A-Za-z0-9._-]{0,190}$/;
const PROVENANCE_VERSION = 1;
const REUSABLE_WORKFLOW_PATH = ".github/workflows/review-pipeline.yml";

// Events whose jobs run trusted base-branch code. `pull_request` is absent on purpose: it executes
// contributor-controlled workflow content and could otherwise forge a cached result.
const TRUSTED_EVENTS = Object.freeze([
  "pull_request_target", "workflow_run", "workflow_dispatch", "repository_dispatch",
  "issue_comment", "schedule", "push",
]);

function refuse(reason) {
  return { ok: false, reason };
}

function workflowPathOf(workflowRef) {
  if (typeof workflowRef !== "string") return "";
  const [pathPart] = workflowRef.split("@");
  const segments = pathPart.split("/");
  return segments.length > 2 ? segments.slice(2).join("/") : "";
}

function workflowRefOf(workflowRef) {
  if (typeof workflowRef !== "string") return "";
  const separator = workflowRef.lastIndexOf("@");
  return separator === -1 ? "" : workflowRef.slice(separator + 1);
}

function parseProvenance(raw) {
  let value;
  try {
    value = JSON.parse(typeof raw === "string" ? raw : "");
  } catch {
    return refuse("prior results are not valid JSON");
  }
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    return refuse("prior results are not an object");
  }
  if (value.v !== PROVENANCE_VERSION) return refuse("prior results use an unsupported version");
  if (!RUN_ID.test(String(value.run_id ?? ""))) return refuse("prior results name no run");
  if (!SHA.test(value.head_sha ?? "") || !SHA.test(value.base_sha ?? "")) {
    return refuse("prior results name no reviewed commits");
  }
  if (!DIGEST.test(value.evidence_digest ?? "") || !DIGEST.test(value.policy_digest ?? "")) {
    return refuse("prior results carry no identity digests");
  }
  if (value.corpus_sha !== null && !SHA.test(value.corpus_sha ?? "")) {
    return refuse("prior results carry an invalid corpus commit");
  }
  const artifacts = value.artifacts;
  if (artifacts === null || typeof artifacts !== "object" || Array.isArray(artifacts)) {
    return refuse("prior results name no artifacts");
  }
  const named = [
    artifacts.evidence, artifacts.validation, artifacts.corpus, artifacts.aggregate,
    artifacts.general, ...Object.values(artifacts.specialists ?? {}),
  ].filter((name) => name !== null && name !== undefined);
  if (named.some((name) => typeof name !== "string" || !ARTIFACT_NAME.test(name))) {
    return refuse("prior results name an invalid artifact");
  }
  return { ok: true, value };
}

async function authenticatePriorRun({
  github, owner, repo, runId, currentRunId, repository, workflowRef,
} = {}) {
  let run;
  try {
    ({ data: run } = await github.rest.actions.getWorkflowRun({
      owner, repo, run_id: Number(runId),
    }));
  } catch (error) {
    return refuse(`prior run is unavailable: ${normalizeText(error.message, 120) || "unknown error"}`);
  }
  if (run.repository?.full_name !== repository) {
    return refuse("prior run belongs to another repository");
  }
  if (!TRUSTED_EVENTS.includes(run.event)) {
    return refuse(`prior run was triggered by the untrusted event ${run.event}`);
  }
  const expectedPath = workflowPathOf(workflowRef);
  if (expectedPath === "" || run.path !== expectedPath) {
    return refuse("prior run used a different workflow file");
  }
  const referenced = Array.isArray(run.referenced_workflows) ? run.referenced_workflows : [];
  const trustedRef = workflowRefOf(workflowRef);
  if (referenced.length > 0) {
    const pipeline = referenced.find((entry) =>
      typeof entry?.path === "string" && entry.path.split("@")[0]
        .endsWith(`/${REUSABLE_WORKFLOW_PATH}`));
    if (!pipeline) return refuse("prior run did not reference the reusable review pipeline");
    const pipelineRef = pipeline.ref || workflowRefOf(pipeline.path);
    if (pipelineRef !== trustedRef) {
      return refuse("prior run referenced the review pipeline from an untrusted ref");
    }
  } else if (String(runId) !== String(currentRunId)) {
    return refuse("prior run reports no referenced workflows");
  }
  return { ok: true, value: { runId: String(runId), event: run.event, status: run.status } };
}

async function resolveTrustedArtifact({ github, owner, repo, runId, name } = {}) {
  if (typeof name !== "string" || !ARTIFACT_NAME.test(name)) {
    return refuse("requested artifact name is invalid");
  }
  let artifacts;
  try {
    artifacts = await github.paginate(github.rest.actions.listWorkflowRunArtifacts, {
      owner, repo, run_id: Number(runId), per_page: 100,
    });
  } catch (error) {
    return refuse(`prior artifacts are unavailable: ${normalizeText(error.message, 120) || "unknown error"}`);
  }
  const matches = artifacts.filter((artifact) => artifact.name === name);
  if (matches.length === 0) return refuse(`prior artifact ${name} is missing`);
  if (matches.length > 1) return refuse(`prior artifact ${name} is ambiguous`);
  const [artifact] = matches;
  if (artifact.expired === true) return refuse(`prior artifact ${name} has expired`);
  if (artifact.workflow_run && String(artifact.workflow_run.id) !== String(runId)) {
    return refuse(`prior artifact ${name} belongs to another run`);
  }
  return { ok: true, value: { id: artifact.id, name: artifact.name } };
}

module.exports = {
  PROVENANCE_VERSION, REUSABLE_WORKFLOW_PATH, TRUSTED_EVENTS,
  authenticatePriorRun, parseProvenance, resolveTrustedArtifact, workflowPathOf, workflowRefOf,
};
