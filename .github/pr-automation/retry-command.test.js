"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const { encodeCheckState, SCHEMA_VERSION } = require("./validate-classifier");
const { acceptRetry, commandBody, marker, retryBinding, validRetryBinding } = require("./retry-command");
const { resolvePr } = require("./resolve-pr");
const { claimAutomaticLease } = require("./automation-lease");

const SHA = "a".repeat(40);
const NEXT = "b".repeat(40);
const ISSUE_URL = "https://api.github.com/repos/Devolutions/IronRDP/issues/42";
const AT = Date.parse("2026-05-01T12:00:00Z");
const args = { owner: "Devolutions", repo: "IronRDP", prNumber: 42, commentId: 19 };
const classification = (conclusion = "neutral", id = 5) => ({
  id, name: "AI classification", head_sha: SHA, app: { slug: "github-actions" },
  external_id: `${SCHEMA_VERSION}:${SHA}`, status: "completed",
  completed_at: "2026-05-01T11:00:00Z", conclusion,
  output: conclusion === "success" ? {
    title: "Classification complete", summary: encodeCheckState({
      protocolRelated: false, risk: "low", specialistReviewers: [],
      automaticReviewEligible: true,
    }),
  } : { title: "Classification unavailable", summary: "Automated classification was unavailable" },
});
const review = (id = 8) => ({
  id, name: "AI automated review", head_sha: SHA, app: { slug: "github-actions" },
  external_id: SHA, status: "completed", completed_at: "2026-05-01T11:00:00Z", conclusion: "neutral",
  output: { title: "Automated review unavailable", summary: "Automated review was unavailable" },
});

function workflowScript() {
  const yaml = fs.readFileSync(path.join(__dirname, "../workflows/pr-automation-retry.yml"), "utf8")
    .replace(/\r\n/g, "\n");
  const script = yaml.split("          script: |\n")[1];
  return script.split("\n").map((line) => line.startsWith("            ") ? line.slice(12) : line)
    .join("\n");
}

function mock(options = {}) {
  const state = {
    pull: { number: 42, state: "open", draft: false, issue_url: ISSUE_URL,
      created_at: "2026-04-01T00:00:00Z", author_association: "MEMBER",
      head: { sha: SHA, repo: { full_name: "Devolutions/IronRDP" } },
      base: { sha: NEXT },
      user: { id: 2, type: "User", login: "author" } },
    comment: { id: 19, issue_url: ISSUE_URL, body: "@github-actions retry",
      created_at: "2026-05-01T12:00:00Z",
      user: { id: 2, type: "User", login: "author" } },
    labels: ["automation-failed"],
    classification: [classification()],
    review: [],
    admissions: [],
    published: [],
    ci: [{ id: 10, run_attempt: 1, name: "CI", head_sha: SHA,
      status: "completed", conclusion: "success" }],
    permission: "write", dispatchFailure: null, dispatches: [], writes: [], readNames: [],
    ...options,
  };
  const listForRef = () => {};
  const listReviews = () => {};
  const listWorkflowRunsForRepo = () => {};
  let nextCheckId = 100;
  const github = {
    paginate: { iterator: async function* (method, params) {
      if (method === listForRef) {
        assert.equal(params.filter, "all");
        assert.equal(params.ref, state.pull.head.sha);
        const key = {
          "AI classification": "classification", "AI automated review": "review",
          "AI retry admission": "admissions",
        }[params.check_name];
        state.readNames.push(key);
        yield { data: state[key].slice(0, 1) };
        yield { data: state[key].slice(1) };
      } else if (method === listReviews) yield { data: state.published };
      else if (method === listWorkflowRunsForRepo) yield { data: state.ci };
      else throw new Error("unexpected pagination");
    } },
    rest: {
      pulls: {
        get: async () => ({ data: state.pull }), listReviews, list: () => {},
      },
      issues: {
        getComment: async () => ({ data: state.comment }),
        get: async () => ({ data: { labels: state.labels.map((name) => ({ name })) } }),
      },
      repos: { getCollaboratorPermissionLevel: async () => ({
        data: { permission: state.permission },
      }) },
      checks: {
        listForRef,
        create: async (params) => {
          state.writes.push(params);
          const data = { ...params, id: nextCheckId++, created_at: new Date(AT).toISOString(),
            app: { slug: "github-actions" } };
          if (params.name === "AI classification") state.classification.unshift(data);
          else state.admissions.push(data);
          state.afterCreate?.(state);
          return { data };
        },
        update: async (params) => {
          state.writes.push(params);
          Object.assign([...state.admissions, ...state.classification]
            .find((run) => run.id === params.check_run_id), params);
          return { data: {} };
        },
      },
      actions: {
        listWorkflowRunsForRepo,
        createWorkflowDispatch: async (params) => {
          if (!state.dispatchFailure || state.dispatchFailure.accepted) state.dispatches.push(params);
          if (state.dispatchFailure) throw state.dispatchFailure.error;
          return { data: {} };
        },
      },
    },
  };
  return { github, state, execute: () => acceptRetry({
    github, ...args, eventHadFailureLabel: true,
  }) };
}

async function embedded(fixture, event = {}) {
  const context = { repo: { owner: args.owner, repo: args.repo },
    payload: { issue: { number: 42, pull_request: {},
      labels: fixture.state.labels.map((name) => ({ name })) },
      comment: fixture.state.comment, ...event } };
  const rootRequire = (name) => name === "./.github/pr-automation/retry-command"
    ? require("./retry-command") : require(name);
  return new vm.Script(`(async function(github, context, require) {\n${workflowScript()}\n})`)
    .runInThisContext()(fixture.github, context, rootRequire);
}

test("standalone command dispatches a bound retry and CI runs every automation suite", async () => {
  const caller = fs.readFileSync(path.join(__dirname, "../workflows/pr-automation.yml"), "utf8");
  const ciWorkflow = fs.readFileSync(path.join(__dirname, "../workflows/ci.yml"), "utf8");
  assert.match(ciWorkflow, /node --test \.github\/pr-automation\/automation\.test\.js \.github\/pr-automation\/review-ready\.test\.js \.github\/pr-automation\/retry-command\.test\.js/);
  assert.match(caller, /workflow_dispatch:\s*\n\s*inputs:/);
  assert.match(caller, /review:\s*\n(?:[^\n]*\n)*?\s*type: boolean/);
  assert.match(caller, /force:\s*\n(?:[^\n]*\n)*?\s*type: boolean/);
  assert.match(caller, /retry-binding: \$\{\{ steps\.resolve\.outputs\.retry-binding \}\}/);
  assert.equal((caller.match(/retryBinding: process\.env\.RETRY_BINDING \? JSON\.parse/g) || []).length, 2);
  assert.match(caller, /\(github\.event_name == 'workflow_dispatch' && inputs\.review\)/);
  const resolver = fs.readFileSync(path.join(__dirname, "resolve-pr.js"), "utf8");
  assert.match(resolver, /const dispatchReview = route === "dispatch" &&\s*\n\s*inputFlag\(/);
  for (const body of ["@github-actions retry\n", "\n@github-actions retry",
    "@github-actions retry\u2028", "@github-actions retry\u0085",
    "> @github-actions retry", "hello @github-actions retry",
    "@github-actions retry now", "@github-actions review"]) assert.equal(commandBody(body), false);
  assert.equal(commandBody("\t @github-actions retry \t"), true);
  const fixture = mock();
  await embedded(fixture);
  assert.equal(fixture.state.dispatches.length, 1);
  assert.deepEqual(fixture.state.dispatches[0], {
    owner: args.owner, repo: args.repo, workflow_id: "pr-automation.yml", ref: "master",
    inputs: {
      "pr-number": "42", review: "false", force: "false",
      "retry-head-sha": SHA, "retry-failure-check-id": "5", "retry-admission-check-id": "100",
    },
  });
  assert.equal(fixture.state.admissions[0].name, "AI retry admission");
  assert.equal(fixture.state.readNames.filter((name) => name === "classification").length, 3);
  assert.equal(await fixture.execute(), false);
  assert.equal(fixture.state.dispatches.length, 1);
  assert.equal(fixture.state.writes.length, 1);
});

test("authentication, event shape, and live mutable policy fail closed", async () => {
  for (const changes of [
    { comment: { body: "@github-actions retry\n" } },
    { comment: { created_at: "invalid" } },
    { comment: { created_at: "2026-02-30T12:00:00Z" } },
    { comment: { user: { id: 2, type: "Bot", login: "bot" } } },
    { comment: { user: { id: 7, type: "User", login: "stranger" } }, permission: "read" },
    { comment: { issue_url: "other" } }, { pull: { state: "closed" } },
    { pull: { draft: true } }, { labels: [] }, { classification: [] },
    { classification: [{ ...classification(), app: { slug: "other" } }] },
    { classification: [{ ...classification(), external_id: "foreign" }] },
    { classification: [{ ...classification(), status: "in_progress" }] },
    { classification: [{ ...classification(), output: { title: "Automation superseded" } }] },
    { classification: [classification(), classification("success", 4)] },
  ]) {
    const fixture = mock();
    for (const [key, value] of Object.entries(changes)) {
      if (key === "comment" || key === "pull") Object.assign(fixture.state[key], value);
      else fixture.state[key] = value;
    }
    await embedded(fixture);
    assert.equal(fixture.state.dispatches.length, 0, JSON.stringify(changes));
  }
  for (const permission of ["write", "maintain", "admin"]) {
    const fixture = mock({ permission });
    fixture.state.comment.user = { id: 7, type: "User", login: "outside" };
    assert.equal(await fixture.execute(), true);
  }
  const issue = mock();
  await embedded(issue, { issue: { number: 42 } });
  assert.equal(issue.state.writes.length, 0);
  const beforeLabel = mock();
  await embedded(beforeLabel, { issue: { number: 42, pull_request: {}, labels: [] } });
  assert.equal(beforeLabel.state.writes.length, 0);
  assert.equal(await acceptRetry({ github: beforeLabel.github, ...args }), false);
  assert.equal(await acceptRetry({
    github: beforeLabel.github, ...args, eventHadFailureLabel: false,
  }), false);
  const redCi = mock({ ci: [{ id: 10, run_attempt: 1, name: "CI", head_sha: SHA,
    status: "completed", conclusion: "failure" }],
    labels: ["automation-failed", "needs-author-action"] });
  assert.equal(await redCi.execute(), true);
});

test("review retry requires valid classification, current green CI, no published review and quota", async () => {
  const base = { classification: [classification("success")], review: [review()] };
  const fixture = mock(base);
  fixture.state.labels.push("needs-author-action", "ai-reviewed/1");
  await embedded(fixture);
  assert.equal(fixture.state.dispatches[0].inputs.review, "true");
  assert.equal(fixture.state.dispatches[0].inputs.force, "false");
  for (const change of [
    { ci: [{ ...fixture.state.ci[0], conclusion: "failure" }] },
    { ci: [{ ...fixture.state.ci[0], status: "in_progress", conclusion: null }] },
    { ci: [{ ...fixture.state.ci[0], id: 11, conclusion: "failure" }, fixture.state.ci[0]] },
    { labels: ["automation-failed", "ai-reviewed/3"] },
    { labels: ["automation-failed", "ai-reviewed/1", "ai-reviewed/2"] },
    { labels: ["automation-failed", "triage/legitimacy"] },
    { published: [{ user: { login: "github-actions[bot]" }, commit_id: SHA,
      body: `<!-- ironrdp-pr-automation:review:${SHA} -->\n\nPartial review` }] },
    { classification: [classification()] },
    { review: [{ ...review(), status: "in_progress" }] },
    { review: [{ ...review(), app: { slug: "foreign" } }] },
    { review: [{ ...review(), conclusion: "success" }, review(7)] },
    { pull: { head: { sha: NEXT, repo: { full_name: "Devolutions/IronRDP" } } } },
  ]) {
    const current = mock({ ...base, labels: ["automation-failed", "ai-reviewed/1",
      "needs-author-action"] });
    Object.assign(current.state, change);
    assert.equal(await current.execute(), false, JSON.stringify(change));
    assert.equal(current.state.dispatches.length, 0);
  }
  const quota = mock({ ...base, labels: ["automation-failed", "ai-reviewed/1"] });
  quota.state.pull.head.repo = { full_name: "outside/fork" };
  quota.state.pull.author_association = "CONTRIBUTOR";
  // An unavailable quota is never interpreted as an exemption.
  assert.equal(await quota.execute(), false);
});

test("receipts enforce comment deduplication, server-time cooldown, stage cap and malformed history", async () => {
  const fixture = mock();
  const receipt = (id, commentId, at, stage = "classification") => ({
    id, name: "AI retry admission", app: { slug: "github-actions" },
    head_sha: SHA, external_id: `retry-admission-v1:${SHA}:${stage}`,
    status: "completed", conclusion: "success", created_at: at,
    output: { summary: marker({ headSha: SHA, stage, commentId, failureCheckId: 5 }) },
  });
  fixture.state.admissions = [receipt(40, 12, "2026-05-01T11:01:00Z")];
  assert.equal(await fixture.execute(), false);
  fixture.state.admissions[0].created_at = "2026-05-01T11:00:00Z";
  const runnerClock = Date.now;
  try {
    Date.now = () => Date.parse("2000-01-01T00:00:00Z");
    assert.equal(await fixture.execute(), true);
    Date.now = () => Date.parse("2100-01-01T00:00:00Z");
    const tooSoon = mock({ admissions: [receipt(40, 12, "2026-05-01T11:01:00Z")] });
    assert.equal(await tooSoon.execute(), false);
  } finally {
    Date.now = runnerClock;
  }
  assert.equal(fixture.state.admissions[1].conclusion, "success");
  assert.equal(fixture.state.admissions[1].output.summary.includes('"comment_id":19'), true);
  assert.equal(await fixture.execute(), false);
  const full = mock({ admissions: [receipt(40, 12, "2026-04-30T00:00:00Z"),
    receipt(41, 13, "2026-04-30T01:00:00Z"),
    receipt(42, 14, "2026-04-30T02:00:00Z")] });
  assert.equal(await full.execute(), false);
  for (const bad of [
    { ...receipt(40, 12, "invalid") },
    { ...receipt(40, 12, "2026-04-30T00:00:00Z"), app: { slug: "other" } },
    { ...receipt(40, 12, "2026-04-30T00:00:00Z"), output: { summary: "malformed" } },
  ]) {
    const malformed = mock({ admissions: [bad] });
    assert.equal(await malformed.execute(), false);
  }
  const sameComment = mock({ admissions: [receipt(40, 19, "2026-04-30T00:00:00Z")] });
  assert.equal(await sameComment.execute(), false);
  const distinct = mock({ admissions: [receipt(40, 12, "2026-04-30T00:00:00Z")] });
  distinct.state.comment.user = { id: 7, type: "User", login: "maintainer" };
  assert.equal(await distinct.execute(), true);
  const changedHistory = mock({ admissions: [receipt(40, 12, "2026-04-30T00:00:00Z")] });
  changedHistory.state.afterCreate = (state) => {
    state.admissions.push(receipt(41, 19, "2026-05-01T11:30:00Z"));
  };
  assert.equal(await changedHistory.execute(), false);
  assert.equal(changedHistory.state.dispatches.length, 0);
  assert.equal(changedHistory.state.admissions.find((run) => run.id === 100).conclusion, "neutral");
  const stage = mock({
    classification: [classification("success")], review: [review()],
    labels: ["automation-failed", "ai-reviewed/1"],
    admissions: [receipt(40, 12, "2026-05-01T11:59:00Z")],
  });
  assert.equal(await stage.execute(), true);
  assert.equal(stage.state.dispatches[0].inputs.review, "true");
});

test("queued commands must follow the selected failure and have a failure label at creation", async () => {
  for (const created_at of [undefined, "garbage", "2026-05-01T10:59:59Z",
    "2026-05-01T11:00:00Z"]) {
    const fixture = mock();
    fixture.state.comment.created_at = created_at;
    assert.equal(await fixture.execute(), false);
    assert.equal(fixture.state.admissions.length, 0);
  }
  for (const completed_at of [undefined, "garbage", "2026-05-01T12:00:00Z",
    "2026-05-01T12:00:01Z"]) {
    const fixture = mock();
    fixture.state.classification[0].completed_at = completed_at;
    assert.equal(await fixture.execute(), false);
  }
  const queued = mock();
  queued.state.comment.created_at = "2026-05-01T11:30:00Z";
  queued.state.classification[0].completed_at = "2026-05-01T12:00:00Z";
  assert.equal(await queued.execute(), false);
  const reviewQueued = mock({ classification: [classification("success")], review: [review()],
    labels: ["automation-failed", "ai-reviewed/1"] });
  reviewQueued.state.review[0].completed_at = "2026-05-01T12:00:00Z";
  assert.equal(await reviewQueued.execute(), false);
  const valid = mock();
  assert.equal(await valid.execute(), true);
  const preLabel = mock();
  assert.equal(await acceptRetry({ github: preLabel.github, ...args,
    eventHadFailureLabel: false }), false);
  assert.equal(preLabel.state.writes.length, 0);
});

test("recheck head, permission, CI generation and stage before dispatch; retain ambiguous dispatch receipts", async () => {
  for (const mutate of [
    { update: (s) => { s.pull.head.sha = NEXT; } },
    { update: (s) => { s.permission = "read"; } },
    { update: (s) => { s.review.push(review()); s.classification = [classification("success")]; } },
    { review: true, update: (s) => { s.ci.push({ ...s.ci[0], id: 11, conclusion: "failure" }); } },
    { review: true, update: (s) => { s.labels.push("ai-reviewed/2"); } },
  ]) {
    const fixture = mutate.review
      ? mock({ classification: [classification("success")], review: [review()],
        labels: ["automation-failed", "ai-reviewed/1"] }) : mock();
    fixture.state.comment.user = { id: 7, type: "User", login: "outside" };
    fixture.state.afterCreate = mutate.update;
    assert.equal(await fixture.execute(), false);
    assert.equal(fixture.state.dispatches.length, 0);
  }
  for (const dispatchFailure of [
    { accepted: false, error: Object.assign(new Error("unprocessable"), { status: 422 }) },
    { accepted: true, error: new Error("timeout after acceptance") },
    { accepted: true, error: Object.assign(new Error("server error"), { status: 500 }) },
  ]) {
    const failed = mock({ dispatchFailure });
    await assert.rejects(failed.execute(), (error) => error === dispatchFailure.error);
    assert.equal(failed.state.admissions.length, 1);
    assert.equal(failed.state.admissions[0].conclusion, "success");
    failed.state.dispatchFailure = null;
    assert.equal(await failed.execute(), false);
    assert.equal(failed.state.dispatches.length, dispatchFailure.accepted ? 1 : 0);
  }
});

test("bound dispatch rejects stale state before routing and queued changes before lease claim", async () => {
  const variants = [
    ["head moved", (s) => { s.pull.head.sha = NEXT; }],
    ["new same-stage failure", (s) => { s.classification.unshift(classification("neutral", 9)); }],
    ["superseding success", (s) => { s.classification.unshift(classification("success", 9)); }],
    ["foreign admission", (s) => { s.admissions[0].app.slug = "other"; }],
    ["malformed admission", (s) => { s.admissions[0].output.summary = "not a receipt"; }],
    ["receipt failure changed", (s) => { s.admissions[0].output.summary =
      marker({ headSha: SHA, stage: "classification", commentId: 19, failureCheckId: 9 }); }],
    ["missing failure label", (s) => { s.labels = []; }],
  ];
  const context = (inputs) => ({
    eventName: "workflow_dispatch", repo: { owner: args.owner, repo: args.repo },
    payload: { inputs },
  });
  for (const [name, change] of variants) {
    const fixture = mock();
    assert.equal(await fixture.execute(), true);
    const inputs = fixture.state.dispatches[0].inputs;
    change(fixture.state);
    const decision = await resolvePr({ github: fixture.github, context: context(inputs) });
    assert.equal(decision.ok, false, name);
    assert.equal(fixture.state.writes.length, 1, name);
  }
  for (const inputsChange of [
    (i) => { delete i["retry-head-sha"]; },
    (i) => { delete i["retry-failure-check-id"]; },
    (i) => { delete i["retry-admission-check-id"]; },
    (i) => { i["retry-admission-check-id"] = "0"; },
    (i) => { i.review = "true"; },
  ]) {
    const fixture = mock();
    assert.equal(await fixture.execute(), true);
    const inputs = { ...fixture.state.dispatches[0].inputs };
    inputsChange(inputs);
    assert.equal((await resolvePr({ github: fixture.github, context: context(inputs) })).ok, false);
  }
  const fixture = mock();
  assert.equal(await fixture.execute(), true);
  const inputs = fixture.state.dispatches[0].inputs;
  const initial = await resolvePr({ github: fixture.github, context: context(inputs) });
  assert.equal(initial.ok, true);
  assert.equal(await validRetryBinding({
    github: fixture.github, ...args, stage: "classification", binding: retryBinding(inputs),
  }), true);
  fixture.state.classification.unshift(classification("neutral", 9));
  const claim = await claimAutomaticLease({
    github: fixture.github, owner: args.owner, repo: args.repo,
    kind: "classification", headSha: initial.headSha, prNumber: args.prNumber,
    runId: 123, attempt: 1, retryBinding: initial.retryBinding,
  });
  assert.equal(claim.owner, false);
  assert.equal(fixture.state.writes.length, 1, "no canonical lease claimed after queued change");

  const manual = await resolvePr({ github: fixture.github, context: context({
    "pr-number": "42", review: "false", force: "true",
  }) });
  assert.equal(manual.ok, true);
  assert.equal(manual.retryBinding, null);

  const valid = mock();
  assert.equal(await valid.execute(), true);
  const admitted = await resolvePr({
    github: valid.github, context: context(valid.state.dispatches[0].inputs),
  });
  assert.equal(admitted.ok, true);
  const owned = await claimAutomaticLease({
    github: valid.github, owner: args.owner, repo: args.repo,
    kind: "classification", headSha: admitted.headSha, prNumber: args.prNumber,
    runId: 124, attempt: 1, retryBinding: admitted.retryBinding,
  });
  assert.equal(owned.owner, true, "a valid retry must be able to claim its lease");

  const reviewRetry = mock({
    classification: [classification("success")], review: [review()],
    labels: ["automation-failed", "ai-reviewed/1"],
  });
  assert.equal(await reviewRetry.execute(), true);
  const reviewInputs = reviewRetry.state.dispatches[0].inputs;
  assert.equal(reviewInputs.review, "true");
  assert.equal(reviewInputs["retry-failure-check-id"], "8");
  assert.equal((await resolvePr({
    github: reviewRetry.github, context: context(reviewInputs),
  })).ok, true);
  reviewRetry.state.review.unshift(review(9));
  assert.equal((await resolvePr({
    github: reviewRetry.github, context: context(reviewInputs),
  })).ok, false);
});
