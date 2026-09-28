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

async function assertCurrentHead({ github, owner, repo, pullNumber, expectedHeadSha }) {
  const data = await readPullRequest({ github, owner, repo, pullNumber });
  if (data.state !== "open" || data.head?.sha !== expectedHeadSha) {
    throw new StaleHeadError();
  }
}

module.exports = { StaleHeadError, assertCurrentHead };
