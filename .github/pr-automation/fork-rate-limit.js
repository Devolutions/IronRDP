"use strict";

const { qualifyingMergedPrs } = require("./resolve-state");

const AUTHOR_QUOTA = 5;
const ESTABLISHED_AUTHOR_QUOTA = 10;
const ESTABLISHED_MERGED_PRS = 15;
const GLOBAL_QUOTA = 30;

function unavailable(reason) {
  return { status: "unavailable", scope: "unknown", reason };
}

function utcDayRange(now) {
  const date = now instanceof Date ? now : new Date(now);
  if (Number.isNaN(date.getTime())) return null;
  const start = Date.UTC(date.getUTCFullYear(), date.getUTCMonth(), date.getUTCDate());
  return { start, end: start + 24 * 60 * 60 * 1000 };
}

function timestamp(value) {
  if (typeof value !== "string") return null;
  const parsed = Date.parse(value);
  return Number.isNaN(parsed) ? null : parsed;
}

function isSameRepository(pr, owner, repo) {
  const headRepo = pr?.head?.repo;
  if (!headRepo || typeof headRepo !== "object") return false;
  if (typeof headRepo.full_name === "string") return headRepo.full_name.toLowerCase() === `${owner}/${repo}`.toLowerCase();
  return typeof headRepo.name === "string" && typeof headRepo.owner?.login === "string" &&
    headRepo.name.toLowerCase() === repo.toLowerCase() && headRepo.owner.login.toLowerCase() === owner.toLowerCase();
}

async function forkRateLimit({ github, owner, repo, pr, author } = {}) {
  if (!github?.paginate?.iterator || !github?.rest?.pulls?.list || typeof owner !== "string" || typeof repo !== "string" ||
      !pr || typeof pr !== "object") return unavailable("invalid rate-limit input");
  if (isSameRepository(pr, owner, repo)) return { status: "allowed", scope: "same-repository" };

  const authorNodeId = author?.nodeId ?? pr.user?.node_id;
  const currentPrNumber = pr.number;
  const currentCreatedAt = timestamp(pr.created_at);
  const range = utcDayRange(currentCreatedAt);
  if (typeof authorNodeId !== "string" || !authorNodeId || !Number.isSafeInteger(currentPrNumber) ||
      currentPrNumber <= 0 || !range) {
    return unavailable("invalid pull request data");
  }

  let merged;
  try {
    merged = await qualifyingMergedPrs({
      github, owner, repo, authorNodeId, currentPrNumber, stopAt: ESTABLISHED_MERGED_PRS,
    });
  } catch {
    return unavailable("GitHub API unavailable");
  }
  const quota = merged >= ESTABLISHED_MERGED_PRS ? ESTABLISHED_AUTHOR_QUOTA : AUTHOR_QUOTA;
  let authorCount = 1; // Include the current fork PR, which is necessarily in its creation-day window.
  let globalCount = 1;

  try {
    for await (const response of github.paginate.iterator(github.rest.pulls.list, {
      owner, repo, state: "all", sort: "created", direction: "desc", per_page: 100,
    })) {
      if (!Array.isArray(response?.data)) throw new Error("invalid pull request data");
      for (const candidate of response.data) {
        if (!candidate || typeof candidate !== "object" || !Number.isSafeInteger(candidate.number) || candidate.number <= 0) {
          throw new Error("invalid pull request data");
        }
        const createdAt = timestamp(candidate.created_at);
        if (createdAt === null) throw new Error("invalid pull request timestamp");
        if (createdAt < range.start) {
          return { status: "allowed", scope: "author", quota, count: authorCount };
        }
        if (createdAt >= range.end || candidate.number === currentPrNumber) continue;

        const isFork = !isSameRepository(candidate, owner, repo);
        if (isFork && candidate.user?.node_id === authorNodeId) authorCount += 1;
        if (isFork) globalCount += 1;
        if (authorCount > quota) return { status: "limited", scope: "author", quota, count: authorCount };
        if (globalCount > GLOBAL_QUOTA) return { status: "limited", scope: "global", quota: GLOBAL_QUOTA, count: globalCount };
      }
    }
  } catch {
    return unavailable("GitHub API unavailable");
  }
  return { status: "allowed", scope: "author", quota, count: authorCount };
}

module.exports = {
  AUTHOR_QUOTA, ESTABLISHED_AUTHOR_QUOTA, ESTABLISHED_MERGED_PRS, GLOBAL_QUOTA,
  forkRateLimit, isSameRepository, utcDayRange,
};
