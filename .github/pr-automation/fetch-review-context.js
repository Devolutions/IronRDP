"use strict";

const MAX_BODY_LENGTH = 32_000;
const MAX_COMMENT_LENGTH = 4_000;
const MAX_COMMENTS = 25;

function boundedText(value, maximum) {
  const text = typeof value === "string" ? value : "";
  return {
    text: text.slice(0, maximum),
    truncated: text.length > maximum,
  };
}

async function fetchReviewContext({ github, owner, repo, pullNumber, expectedHeadSha }) {
  const { data: pullRequest } = await github.rest.pulls.get({
    owner, repo, pull_number: pullNumber,
  });
  if (pullRequest.head?.sha !== expectedHeadSha) {
    throw new Error("pull request head changed while collecting review context");
  }

  const [conversationComments, reviewComments, submittedReviews] = await Promise.all([
    github.paginate(github.rest.issues.listComments, {
      owner, repo, issue_number: pullNumber, per_page: 100,
    }),
    github.paginate(github.rest.pulls.listReviewComments, {
      owner, repo, pull_number: pullNumber, per_page: 100,
    }),
    github.paginate(github.rest.pulls.listReviews, {
      owner, repo, pull_number: pullNumber, per_page: 100,
    }),
  ]);
  const { data: currentPullRequest } = await github.rest.pulls.get({
    owner, repo, pull_number: pullNumber,
  });
  if (currentPullRequest.head?.sha !== expectedHeadSha) {
    throw new Error("pull request head changed while collecting review context");
  }

  const comments = [
    ...conversationComments.map((comment) => ({ kind: "conversation", comment })),
    ...reviewComments.map((comment) => ({ kind: "review", comment })),
    ...submittedReviews.map((comment) => ({
      kind: "review-body",
      comment: { ...comment, created_at: comment.submitted_at || comment.created_at },
    })),
  ]
    .filter(({ comment }) => comment.user?.type !== "Bot" && typeof comment.body === "string")
    .sort((left, right) => left.comment.created_at.localeCompare(right.comment.created_at));
  const selectedComments = comments.slice(-MAX_COMMENTS);
  const body = boundedText(pullRequest.body, MAX_BODY_LENGTH);

  return {
    head_sha: expectedHeadSha,
    pull_request: {
      number: pullRequest.number,
      title: pullRequest.title,
      author: pullRequest.user?.login || null,
      body: body.text,
      body_truncated: body.truncated,
    },
    comments: selectedComments.map(({ kind, comment }) => {
      const body = boundedText(comment.body, MAX_COMMENT_LENGTH);
      return {
        kind,
        author: comment.user?.login || null,
        author_association: comment.author_association || null,
        created_at: comment.created_at,
        path: kind === "review" ? comment.path || null : null,
        body: body.text,
        body_truncated: body.truncated,
      };
    }),
    comments_omitted: comments.length - selectedComments.length,
  };
}

module.exports = { MAX_BODY_LENGTH, MAX_COMMENT_LENGTH, MAX_COMMENTS, fetchReviewContext };
