"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const { encodeCheckState, SCHEMA_VERSION } = require("./validate-classifier");
const { encodeReviewOutcome } = require("./review-outcome");
const { lifecycleActor, readLifecycleSnapshot, reconcileLifecycle } = require("./lifecycle");
const { reviewBody, inlineReviewCommentBody, REVIEW_READY_FOOTER } = require("./review-render");
const {
  acceptReviewReady, commandBody, encodeReady, trustedReady, readyRuns,
} = require("./review-ready");

const SHA = "a".repeat(40);
const OTHER = "b".repeat(40);
const marker = `<!-- ironrdp-pr-automation:review:${SHA} -->`;
const issueUrl = "https://api.github.com/repos/Devolutions/IronRDP/issues/42";
const args = { owner: "Devolutions", repo: "IronRDP", prNumber: 42, commentId: 19 };
const check = (count = "ai-reviewed/1", outcome = "findings") => ({
  id: 7, name: "AI automated review", app: { slug: "github-actions" },
  head_sha: SHA, external_id: SHA, status: "completed", conclusion: "success",
  completed_at: "2026-05-01T11:00:01Z",
  output: { summary: encodeReviewOutcome({
    headSha: SHA, outcome, nextReviewCount: count, reviewMarker: marker,
  }) },
});
const ready = (commentId = 19, reviewId = 11, checkId = 7) => ({
  id: 8, name: "AI review-ready", app: { slug: "github-actions" },
  head_sha: SHA, external_id: SHA, status: "completed", conclusion: "success",
  created_at: "2026-05-01T12:00:00Z",
  output: { summary: encodeReady({ headSha: SHA, commentId, reviewId, checkId }) },
});

function mock(options = {}) {
  const writes = [];
  const state = {
    pull: {
      number: 42, state: "open", draft: false, issue_url: issueUrl,
      head: { sha: SHA }, user: { id: 2 },
    },
    comment: {
      id: 19, issue_url: issueUrl, body: "@github-actions review-ready",
      created_at: "2026-05-01T11:00:03Z",
      user: { id: 2, type: "User", login: "author" },
    },
    labels: ["ai-reviewed/1"],
    classificationChecks: [{
      id: 6, app: { slug: "github-actions" }, head_sha: SHA,
      external_id: `${SCHEMA_VERSION}:${SHA}`, conclusion: "success",
      output: { title: "Classification complete", summary: encodeCheckState({
        protocolRelated: false, risk: "low", specialistReviewers: [], automaticReviewEligible: true,
      }) },
    }],
    reviewChecks: [check()],
    readyChecks: [],
    reviews: [{
      id: 11, user: { login: "github-actions[bot]" }, commit_id: SHA,
      submitted_at: "2026-05-01T11:00:02Z",
      body: `${marker}\n\nFindings`,
    }],
    ci: [{ head_sha: SHA, status: "completed", conclusion: "success", id: 4, run_attempt: 1,
      name: "CI", event: "pull_request" }],
    permission: "write",
    ...options,
  };
  const listForRef = () => {};
  const listReviews = () => {};
  const listWorkflowRunsForRepo = () => {};
  const github = {
    paginate: {
      iterator: async function* (method, params) {
        if (method === listForRef) {
          yield { data: params.check_name === "AI automated review" ? state.reviewChecks :
            params.check_name === "AI review-ready" ? state.readyChecks :
              params.check_name === "AI classification" ? state.classificationChecks : [] };
        } else if (method === listReviews) yield { data: state.reviews };
        else if (method === listWorkflowRunsForRepo) yield { data: state.ci };
      },
    },
    rest: {
      pulls: {
        get: async () => ({ data: state.pull }), listReviews,
      },
      issues: {
        get: async () => ({ data: { labels: state.labels.map((name) => ({ name })) } }),
        getComment: async () => ({ data: state.comment }),
        addLabels: async (value) => { writes.push(value); state.labels.push(...value.labels); },
        removeLabel: async (value) => {
          writes.push(value); state.labels = state.labels.filter((name) => name !== value.name);
        },
      },
      checks: {
        listForRef, create: async (payload) => {
          writes.push(payload);
          state.readyChecks = [{ ...ready(), ...payload, id: 8 }];
          return { data: state.readyChecks[0] };
        },
        update: async (payload) => {
          writes.push(payload);
          state.readyChecks[0].output = payload.output;
        },
      },
      repos: {
        getCollaboratorPermissionLevel: async () => {
          if (state.permission === null) throw new Error("permission API unavailable");
          return { data: { permission: state.permission } };
        },
      },
      actions: { listWorkflowRunsForRepo },
    },
  };
  return { state, github, writes };
}

test("command is exact, standalone, and single-line", () => {
  for (const body of ["@github-actions review-ready", " \t@github-actions review-ready \t"]) {
    assert.equal(commandBody(body), true);
  }
  for (const body of [
    "> @github-actions review-ready", "`@github-actions review-ready`",
    "please @github-actions review-ready", "@github-actions review-ready thanks",
    ...["\r", "\n", "\u2028", "\u2029", "\v", "\f", "\u00a0"].flatMap((terminator) => [
      `${terminator}@github-actions review-ready`,
      `@github-actions review-ready${terminator}`,
      `@github-actions${terminator} review-ready`,
    ]),
  ]) assert.equal(commandBody(body), false);
});

test("only a live PR author by stable ID or live write-capable human may acknowledge", async () => {
  const { github, state, writes } = mock();
  assert.equal(await acceptReviewReady({ github, ...args }), true);
  assert.equal(writes.filter((entry) => entry.name === "AI review-ready").length, 1);
  assert.equal(await acceptReviewReady({ github, ...args }), false);
  state.comment.user = { id: 3, type: "User", login: "outside-collaborator" };
  for (const permission of ["write", "maintain", "admin"]) {
    state.permission = permission;
    assert.equal(await acceptReviewReady({ github, ...args }), false);
  }
  for (const permission of ["read", "none", null]) {
    state.permission = permission;
    state.readyChecks = [];
    assert.equal(await acceptReviewReady({ github, ...args }), false);
  }
  state.comment.user.type = "Bot";
  state.permission = "admin";
  assert.equal(await acceptReviewReady({ github, ...args }), false);
  state.comment.user = { id: 2, type: "User", login: "renamed-author" };
  state.pull.state = "closed";
  assert.equal(await acceptReviewReady({ github, ...args }), false);
  state.pull.state = "open";
  state.pull.draft = true;
  assert.equal(await acceptReviewReady({ github, ...args }), false);
  state.pull.draft = false;
  state.comment.issue_url = "https://api.github.com/repos/Devolutions/IronRDP/issues/43";
  assert.equal(await acceptReviewReady({ github, ...args }), false);
  state.comment.issue_url = issueUrl;
  state.comment.id = 20;
  assert.equal(await acceptReviewReady({ github, ...args }), false);
});

test("trusted collaborators can accept while CI is unavailable; later comments are idempotent", async () => {
  for (const permission of ["write", "maintain", "admin"]) {
    const { state, github, writes } = mock({
      permission, ci: [{ head_sha: SHA, status: "completed", conclusion: "failure" }],
    });
    state.comment.user = { id: 3, type: "User", login: "outside-collaborator" };
    assert.equal(await acceptReviewReady({ github, ...args }), true);
    state.comment.id = 20;
    assert.equal(await acceptReviewReady({ github, ...args, commentId: 20 }), false);
    assert.equal(writes.length, 1);
    assert.equal(state.readyChecks[0].output.summary, ready().output.summary);
  }
  const { github, state } = mock({ ci: [{ head_sha: SHA, status: "in_progress", conclusion: null }] });
  assert.equal(await acceptReviewReady({ github, ...args }), true);
  assert.deepEqual(lifecycleActor({
    state: "open", draft: false, activeLease: false, labels: new Set(state.labels),
    ci: { status: "in_progress" }, reviewOutcome: "findings", reviewReady: true,
  }), []);
});

test("newer same-head findings review supersedes old acknowledgement without another check", async () => {
  const { state, github, writes } = mock({ readyChecks: [ready()] });
  const forcedMarker = `<!-- ironrdp-pr-automation:review:${SHA}:force:5 -->`;
  state.comment.created_at = "2026-05-01T11:00:07Z";
  state.reviewChecks.unshift({
    ...check(), id: 9, completed_at: "2026-05-01T11:00:05Z",
    output: { summary: encodeReviewOutcome({
      headSha: SHA, outcome: "findings", nextReviewCount: "ai-reviewed/1",
      reviewMarker: forcedMarker,
    }) },
  });
  state.reviews.push({
    id: 12, user: { login: "github-actions[bot]" }, commit_id: SHA,
    submitted_at: "2026-05-01T11:00:06Z",
    body: `${forcedMarker}\n\nNew findings`,
  });
  assert.equal(await acceptReviewReady({ github, ...args }), true);
  assert.equal(writes.length, 1);
  assert.equal(writes[0].check_run_id, 8);
  assert.equal(trustedReady(state.readyChecks[0], SHA).review_id, 12);
  assert.equal(trustedReady(state.readyChecks[0], SHA).review_check_id, 9);
  assert.equal(await acceptReviewReady({ github, ...args }), false);
});

test("comment creation strictly follows review publication, not check completion", async () => {
  for (const field of ["submitted_at", "created_at"]) {
    const invalid = [undefined, "invalid", "2026-02-30T11:00:00Z"];
    const stale = field === "created_at" ?
      ["2026-05-01T11:00:02Z", "2026-05-01T11:00:02.999Z"] :
      ["2026-05-01T11:00:03Z", "2026-05-01T11:00:03.999Z", "2026-05-01T11:00:04Z"];
    for (const value of [...invalid, ...stale]) {
      const { state, github, writes } = mock();
      const subject = field === "submitted_at" ? state.reviews[0] : state.comment;
      subject[field] = value;
      assert.equal(await acceptReviewReady({ github, ...args }), false, `${field}: ${value}`);
      assert.deepEqual(writes, []);
    }
  }
  const { state, github, writes } = mock();
  state.reviewChecks[0].completed_at = "2026-05-01T11:00:04Z";
  state.comment.updated_at = "2026-05-01T11:00:00Z";
  assert.equal(await acceptReviewReady({ github, ...args }), true);
  assert.equal(writes.length, 1);
});

test("queued commands cannot accept newer same-head or new-head findings", async () => {
  for (const newHead of [false, true]) {
    const { state, github, writes } = mock();
    const head = newHead ? OTHER : SHA;
    const newerMarker = `<!-- ironrdp-pr-automation:review:${head}:force:5 -->`;
    state.pull.head.sha = head;
    state.reviewChecks.unshift({
      ...check(), id: 9, head_sha: head, external_id: head,
      completed_at: "2026-05-01T11:00:05Z",
      output: { summary: encodeReviewOutcome({
        headSha: head, outcome: "findings", nextReviewCount: "ai-reviewed/1",
        reviewMarker: newerMarker,
      }) },
    });
    state.reviews.push({
      id: 12, user: { login: "github-actions[bot]" }, commit_id: head,
      submitted_at: "2026-05-01T11:00:06Z", body: `${newerMarker}\n\nNew findings`,
    });
    assert.equal(await acceptReviewReady({ github, ...args }), false);
    assert.deepEqual(writes, []);
  }
});

test("findings count, review identity, head, and check provenance are authoritative", async () => {
  for (const count of ["ai-reviewed/1", "ai-reviewed/2", "ai-reviewed/3"]) {
    const { state, github } = mock({ labels: [count], reviewChecks: [check(count)] });
    assert.equal(await acceptReviewReady({ github, ...args }), true);
    assert.deepEqual(state.labels, [count]);
  }
  for (const change of [
    (s) => { s.reviewChecks = [check("ai-reviewed/1", "no-findings")]; },
    (s) => { s.reviewChecks = [{ ...check(), output: { summary: "legacy receipt" } }]; },
    (s) => { s.reviewChecks = [{ ...check(), output: { summary: encodeReviewOutcome({
      headSha: SHA, outcome: "findings",
    }) } }]; },
    (s) => { s.reviewChecks = [{ ...check(), app: { slug: "other-app" } }]; },
    (s) => { s.reviewChecks = [{ ...check(), output: { summary: check().output.summary + "\n" + check().output.summary } }]; },
    (s) => { s.reviewChecks = [{ ...check(), head_sha: OTHER }]; },
    (s) => { s.labels = ["ai-reviewed/2"]; },
    (s) => { s.reviews = [{ ...s.reviews[0], commit_id: OTHER }]; },
    (s) => { s.reviews = [{ ...s.reviews[0], body: "wrong marker" }]; },
    (s) => { s.reviews.push({ ...s.reviews[0], id: 12, body: "newer review" }); },
    (s) => { s.reviewChecks.unshift({ ...check(), id: 9, status: "in_progress", conclusion: null }); },
    (s) => { s.reviewChecks = []; },
    (s) => { s.reviewChecks = [{ ...check(), conclusion: "neutral" }]; },
    (s) => { s.readyChecks = [{ ...ready(), app: { slug: "other-app" } }]; },
    (s) => { s.readyChecks = [ready(), ready()]; },
  ]) {
    const { state, github, writes } = mock();
    change(state);
    assert.equal(await acceptReviewReady({ github, ...args }), false);
    assert.equal(writes.length, 0);
  }
});

test("second authority read prevents head or review changes before check creation", async () => {
  const { github, state, writes } = mock();
  const read = github.rest.pulls.get;
  let count = 0;
  github.rest.pulls.get = async () => {
    const result = await read();
    if (++count === 2) result.data.head.sha = OTHER;
    return result;
  };
  assert.equal(await acceptReviewReady({ github, ...args }), false);
  assert.deepEqual(writes, []);
  state.pull.head.sha = SHA;
  count = 0;
  github.rest.pulls.get = read;
  const listing = state.reviews;
  const iterator = github.paginate.iterator;
  github.paginate.iterator = async function* (method, params) {
    if (method === github.rest.pulls.listReviews && ++count === 2) {
      listing.push({ ...listing[0], id: 12 });
    }
    yield* iterator(method, params);
  };
  assert.equal(await acceptReviewReady({ github, ...args }), false);
  assert.deepEqual(writes, []);
  // A new canonical findings check before the second read supersedes the command.
  const { github: secondGithub, state: secondState, writes: secondWrites } = mock();
  const getComment = secondGithub.rest.issues.getComment;
  let reads = 0;
  secondGithub.rest.issues.getComment = async (...parameters) => {
    const result = await getComment(...parameters);
    if (++reads === 2) secondState.reviewChecks[0].id = 9;
    return result;
  };
  assert.equal(await acceptReviewReady({ github: secondGithub, ...args }), false);
  assert.deepEqual(secondWrites, []);
});

test("receipt rejects foreign, malformed, stale, and non-server-created checks", () => {
  assert.equal(trustedReady(ready(), SHA).review_id, 11);
  for (const run of [
    { ...ready(), app: { slug: "foreign" } },
    { ...ready(), name: "AI automated review" },
    { ...ready(), head_sha: OTHER },
    { ...ready(), external_id: OTHER },
    { ...ready(), conclusion: "neutral" },
    { ...ready(), created_at: undefined },
    { ...ready(), created_at: "not-a-date" },
    { ...ready(), created_at: "2026-02-30T11:00:00Z" },
    { ...ready(), output: { summary: encodeReady({
      headSha: SHA, commentId: 19, reviewId: 11, checkId: 7,
    }).replace('"review_id":11', '"review_id":11,"extra":1') } },
    { ...ready(), output: { summary: `${ready().output.summary}\n${ready().output.summary}` } },
  ]) assert.equal(trustedReady(run, SHA), null);
});

test("ready check reads distinguish absence from invalid or duplicate receipts", async () => {
  const { state, github } = mock();
  const read = () => readyRuns(github, args.owner, args.repo, SHA);
  assert.deepEqual(await read(), { existing: null, receipt: null });
  state.readyChecks = [ready()];
  assert.deepEqual(await read(), { existing: state.readyChecks[0], receipt: trustedReady(ready(), SHA) });
  state.readyChecks[0] = { ...ready(), output: { summary: "invalid" } };
  assert.equal(await read(), null);
  state.readyChecks = [ready(), { ...ready(), id: 9 }];
  assert.equal(await read(), null);
});

test("lifecycle persists acknowledgement but preserves all existing gates", async () => {
  const { state, github } = mock({ readyChecks: [ready()] });
  const actor = (changes = {}) => lifecycleActor({
    state: "open", draft: false, activeLease: false,
    labels: new Set(state.labels), ci: { status: "completed", conclusion: "success" },
    classificationValid: true, reviewOutcome: "findings", reviewReady: true, ...changes,
  });
  assert.deepEqual(actor(), ["needs-review"]);
  assert.deepEqual(actor({ ci: { status: "completed", conclusion: "failure" } }),
    ["needs-author-action"]);
  for (const changes of [
    { draft: true }, { state: "closed" }, { activeLease: true },
    { ci: { status: "in_progress" } }, { labels: new Set(["automation-failed"]) },
    { canonicalAmbiguous: true }, { reviewReady: false }, { classificationValid: false },
  ]) assert.notDeepEqual(actor(changes), ["needs-review"]);
  let snapshot = await readLifecycleSnapshot({ github, ...args, ciRetry: { retries: 0 } });
  assert.equal(snapshot.reviewReady, true);
  assert.deepEqual(lifecycleActor(snapshot), ["needs-review"]);
  state.classificationChecks.unshift({
    ...state.classificationChecks[0], id: 10, conclusion: null, status: "in_progress",
  });
  snapshot = await readLifecycleSnapshot({ github, ...args, ciRetry: { retries: 0 } });
  assert.equal(snapshot.reviewReady, true);
  assert.equal(snapshot.classificationValid, false);
  assert.deepEqual(lifecycleActor(snapshot), ["needs-author-action"]);
  state.classificationChecks[0] = {
    ...state.classificationChecks[0], conclusion: "success", status: "completed",
    output: { title: "Classification complete", summary: "untrusted" },
  };
  snapshot = await readLifecycleSnapshot({ github, ...args, ciRetry: { retries: 0 } });
  assert.equal(snapshot.classificationValid, false);
  assert.deepEqual(lifecycleActor(snapshot), ["needs-author-action"]);
  state.classificationChecks.shift();
  snapshot = await readLifecycleSnapshot({ github, ...args, ciRetry: { retries: 0 } });
  assert.deepEqual(lifecycleActor(snapshot), ["needs-review"]);
  await reconcileLifecycle({ github, ...args, ciRetry: { retries: 0 } });
  snapshot = await readLifecycleSnapshot({ github, ...args, ciRetry: { retries: 0 } });
  assert.deepEqual(lifecycleActor(snapshot), ["needs-review"]);
  state.reviews.push({ ...state.reviews[0], id: 12 });
  snapshot = await readLifecycleSnapshot({ github, ...args, ciRetry: { retries: 0 } });
  assert.deepEqual(lifecycleActor(snapshot), ["needs-author-action"]);
  state.reviews.pop();
  state.readyChecks = [{ ...ready(), output: { summary: "invalid" } }];
  snapshot = await readLifecycleSnapshot({ github, ...args, ciRetry: { retries: 0 } });
  assert.deepEqual(lifecycleActor(snapshot), ["needs-author-action"]);
  state.readyChecks = [];
  snapshot = await readLifecycleSnapshot({ github, ...args, ciRetry: { retries: 0 } });
  assert.deepEqual(lifecycleActor(snapshot), ["needs-author-action"]);
});

test("prior terminal review trusts only the newest published bot review", async () => {
  const priorMarker = `<!-- ironrdp-pr-automation:review:${OTHER} -->`;
  const { state, github } = mock({
    labels: ["ai-reviewed/3"],
    reviewChecks: [{
      ...check("ai-reviewed/3"), head_sha: OTHER, external_id: OTHER,
      output: { summary: encodeReviewOutcome({
        headSha: OTHER, outcome: "findings", nextReviewCount: "ai-reviewed/3",
        reviewMarker: priorMarker,
      }) },
    }],
    reviews: [{
      id: 11, user: { login: "github-actions[bot]" }, commit_id: OTHER,
      body: `${priorMarker}\n\nFindings`,
    }],
  });
  const snapshot = () => readLifecycleSnapshot({ github, ...args, ciRetry: { retries: 0 } });
  assert.equal((await snapshot()).priorReviewTrusted, true);
  for (const latest of [
    { body: "invalid marker", commit_id: OTHER },
    { body: `${marker}\n\nFindings`, commit_id: SHA },
    { body: `${priorMarker}\n\nFindings`, commit_id: "invalid-sha" },
    { id: "malformed", body: `${priorMarker}\n\nFindings`, commit_id: OTHER },
  ]) {
    state.reviews.push({ id: 12, user: { login: "github-actions[bot]" }, ...latest });
    assert.equal((await snapshot()).priorReviewTrusted, false);
    state.reviews.pop();
  }
});

test("workflow github-script accepts once, reconciles, and skips ineligible events", async () => {
  const workflow = fs.readFileSync(".github/workflows/pr-automation-review-ready.yml", "utf8")
    .replace(/\r\n/g, "\n");
  assert.match(workflow, /if: github\.event\.issue\.pull_request/);
  const body = workflow.slice(workflow.indexOf("          script: |\n") + "          script: |\n".length);
  const lines = [];
  for (const line of body.split("\n")) {
    if (line.trim() !== "" && !line.startsWith("            ")) break;
    lines.push(line.slice(12));
  }
  assert.ok(lines.some((line) => line.includes("acceptReviewReady")));
  const script = lines.join("\n");
  const { state, github, writes } = mock();
  const context = {
    repo: { owner: "Devolutions", repo: "IronRDP" },
    payload: { issue: { number: 42, pull_request: { url: issueUrl } }, comment: state.comment },
  };
  const modules = [];
  const execute = async () => vm.runInNewContext(`(async () => {\n${script}\n})()`, {
    github, context, require: (id) => {
      modules.push(id);
      assert.ok(["./.github/pr-automation/review-ready",
        "./.github/pr-automation/lifecycle"].includes(id), `unexpected model dispatch: ${id}`);
      return require(path.resolve(__dirname, "..", "..", id));
    },
  });
  await execute();
  assert.equal(writes.filter((entry) => entry.name === "AI review-ready").length, 1);
  assert.deepEqual(state.labels, ["ai-reviewed/1", "needs-review"]);
  assert.deepEqual(trustedReady(state.readyChecks[0], SHA), {
    schema_version: "review-ready-v1", head_sha: SHA,
    comment_id: 19, review_id: 11, review_check_id: 7,
  });
  const count = writes.length;
  await execute();
  assert.equal(writes.length, count);
  const get = github.rest.pulls.get;
  github.rest.pulls.get = () => { throw new Error("ineligible event made REST request"); };
  for (const payload of [
    { issue: { number: 42 }, comment: state.comment },
    { issue: context.payload.issue, comment: { ...state.comment, body: "not a command" } },
    { issue: context.payload.issue, comment: { ...state.comment, user: { type: "Bot" } } },
    { issue: { ...context.payload.issue, number: "42" }, comment: state.comment },
    { issue: context.payload.issue, comment: { ...state.comment, id: "19" } },
  ]) {
    context.payload = payload;
    const before = modules.length;
    await execute();
    assert.equal(writes.length, count);
    assert.ok(modules.length === before + 2);
  }
  github.rest.pulls.get = get;
  assert.doesNotMatch(workflow, /openai-agent|review-pipeline/);
});

test("main review body alone gains one footer for body or inline findings and enforces length", () => {
  const finding = {
    severity: "medium", title: "Problem", path: "src/file.rs", rationale: "Fix this",
    start_line: null, end_line: null, sources: ["general"], question: false,
  };
  const provenance = () => "General";
  for (const startLine of [null, 1]) {
    const review = { summary: "Summary", findings: [{ ...finding, start_line: startLine, end_line: startLine }] };
    const body = reviewBody(marker, review, [], provenance);
    assert.equal(body.split(REVIEW_READY_FOOTER).length, 2);
    assert.ok(body.endsWith(REVIEW_READY_FOOTER));
    assert.ok(body.length <= 65_536);
    assert.doesNotMatch(inlineReviewCommentBody(review.findings[0], provenance), /review-ready/);
  }
  assert.doesNotMatch(reviewBody(marker, { summary: "Clean", findings: [] }, [], provenance),
    /review-ready/);
  const workflow = fs.readFileSync(".github/workflows/pr-automation-review-ready.yml", "utf8");
  assert.match(workflow, /issue_comment:\s*\n\s*types: \[created\]/);
  assert.match(workflow, /pr-automation-mutation-\$\{\{ github.event.issue.number \}\}/);
  assert.match(workflow, /cancel-in-progress: false/);
  assert.match(workflow, /ref: master\s*\n\s*persist-credentials: false/);
  assert.doesNotMatch(workflow, /pull_request_target|review-pipeline|openai-agent|head\.ref/);
});
