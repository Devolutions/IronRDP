"use strict";

class StaleHeadError extends Error {
  constructor() {
    super("pull request head is no longer current");
    this.name = "StaleHeadError";
  }
}

function isTruncatedGithubResponse(error) {
  return error?.status === 500 && /unexpected end of JSON input/i.test(String(error.message));
}

async function readPullRequest({ github, owner, repo, pullNumber }) {
  const read = () => github.rest.pulls.get({ owner, repo, pull_number: pullNumber });
  try {
    return (await read()).data;
  } catch (error) {
    if (!isTruncatedGithubResponse(error)) throw error;
    return (await read()).data;
  }
}

async function assertCurrentHead({
  github, owner, repo, pullNumber, expectedHeadSha, allowClosedUnmerged = false,
}) {
  const data = await readPullRequest({ github, owner, repo, pullNumber });
  const closedUnmerged = data.state === "closed" && !data.merged && !data.merged_at;
  if ((data.state !== "open" && !(allowClosedUnmerged && closedUnmerged)) ||
      data.head?.sha !== expectedHeadSha) {
    throw new StaleHeadError();
  }
  return data;
}

function isOpenNonDraftAtHead(pull, expectedHeadSha) {
  return pull?.state === "open" && pull.draft !== true && pull.head?.sha === expectedHeadSha;
}

async function isOpenNonDraftAtHeadNow({ github, owner, repo, pullNumber, expectedHeadSha }) {
  const data = await readPullRequest({ github, owner, repo, pullNumber });
  return isOpenNonDraftAtHead(data, expectedHeadSha);
}

module.exports = {
  StaleHeadError, assertCurrentHead, isOpenNonDraftAtHead, isOpenNonDraftAtHeadNow,
};
