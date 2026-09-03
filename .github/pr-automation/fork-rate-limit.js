"use strict";

const GLOBAL_QUOTA = 50;
const QUOTA_EXEMPT_ASSOCIATIONS = new Set(["OWNER", "MEMBER"]);

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
  if (QUOTA_EXEMPT_ASSOCIATIONS.has(pr.author_association ?? author?.association)) {
    return { status: "allowed", scope: "author-association" };
  }

  const currentPrNumber = pr.number;
  const currentCreatedAt = timestamp(pr.created_at);
  const range = utcDayRange(currentCreatedAt);
  if (!Number.isSafeInteger(currentPrNumber) || currentPrNumber <= 0 || !range) {
    return unavailable("invalid pull request data");
  }

  let globalCount = 1; // Include the current non-member fork PR in its creation-day window.

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
          return { status: "allowed", scope: "global", quota: GLOBAL_QUOTA, count: globalCount };
        }
        if (createdAt >= range.end || candidate.number === currentPrNumber) continue;

        const isFork = !isSameRepository(candidate, owner, repo);
        if (isFork && !QUOTA_EXEMPT_ASSOCIATIONS.has(candidate.author_association)) globalCount += 1;
        if (globalCount > GLOBAL_QUOTA) return { status: "limited", scope: "global", quota: GLOBAL_QUOTA, count: globalCount };
      }
    }
  } catch {
    return unavailable("GitHub API unavailable");
  }
  return { status: "allowed", scope: "global", quota: GLOBAL_QUOTA, count: globalCount };
}

module.exports = {
  GLOBAL_QUOTA,
  forkRateLimit, isSameRepository, utcDayRange,
};
