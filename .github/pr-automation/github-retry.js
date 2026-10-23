"use strict";

const RETRY_DELAYS_MS = [1000, 2000, 4000];

// Repeating these requests cannot duplicate state: reads, updates by identity, deletions, and label
// additions. Creating a comment, review, check run, or dispatch is never retried because GitHub may
// have applied the request before returning the server error.
function isIdempotentRequest({ method, url } = {}) {
  const verb = String(method || "GET").toUpperCase();
  if (["GET", "HEAD", "PATCH", "PUT", "DELETE"].includes(verb)) return true;
  return verb === "POST" && url === "/repos/{owner}/{repo}/issues/{issue_number}/labels";
}

function isTransientError(error) {
  return Number.isInteger(error?.status) && error.status >= 500 && error.status <= 599;
}

// Bounded retry of transient GitHub server errors for idempotent requests. Installed as an Octokit
// request hook so every read and write in the state-publication path shares one policy.
function retryTransientRequests(github, {
  sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
  onRetry = () => {},
} = {}) {
  github.hook.wrap("request", async (request, options) => {
    for (let attempt = 0; ; attempt += 1) {
      try {
        return await request(options);
      } catch (error) {
        if (attempt >= RETRY_DELAYS_MS.length || !isTransientError(error) || !isIdempotentRequest(options)) {
          throw error;
        }
        onRetry({
          method: options.method, url: options.url, status: error.status,
          attempt: attempt + 1, delayMs: RETRY_DELAYS_MS[attempt],
        });
        await sleep(RETRY_DELAYS_MS[attempt]);
      }
    }
  });
}

module.exports = { RETRY_DELAYS_MS, retryTransientRequests };
