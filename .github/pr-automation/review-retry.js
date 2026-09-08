"use strict";

// Head checks and the single delayed retry a reviewer stage may take.
//
// Recovery is bounded inside one workflow invocation: a stage that failed transiently waits, proves
// the review is still worth finishing, and runs exactly once more. Anything else is settled.

const { isRetryableFailure } = require("./review-pipeline");

const MAXIMUM_DELAY_SECONDS = 15 * 60;

async function assertCurrentHead({ github, owner, repo, pullNumber, expectedHeadSha }) {
  const { data } = await github.rest.pulls.get({ owner, repo, pull_number: pullNumber });
  if (data.state !== "open" || data.head?.sha !== expectedHeadSha) {
    throw new Error("pull request head is no longer current");
  }
}

// A retry costs a provider request and holds the caller's pipeline slot, so it has to be worth it:
// the failure must be transient, and the review it would finish must still be publishable.
async function delayedRetryGate({
  github, owner, repo, pullNumber, expectedHeadSha, failureCategory, delaySeconds,
  sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
} = {}) {
  if (!isRetryableFailure(failureCategory)) {
    return { retry: false, reason: `${failureCategory || "the failure"} is not retryable` };
  }
  const delay = Number.isFinite(delaySeconds) && delaySeconds >= 0
    ? Math.min(delaySeconds, MAXIMUM_DELAY_SECONDS)
    : 0;
  await sleep(delay * 1000);
  try {
    await assertCurrentHead({ github, owner, repo, pullNumber, expectedHeadSha });
  } catch (error) {
    return { retry: false, reason: error.message };
  }
  return { retry: true, reason: "" };
}

module.exports = { MAXIMUM_DELAY_SECONDS, assertCurrentHead, delayedRetryGate };
