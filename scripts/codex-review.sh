#!/bin/sh
# Request a Codex review of a PR's current head, wait for it, and print its findings on that head:
#   scripts/codex-review.sh <pr> [--wait-only]
# --wait-only skips the request when that head is already under review.
# Exit 0 = review completed without findings, 2 = findings printed, 1 = failed or timed out
# (CODEX_REVIEW_TIMEOUT seconds, default 1800). Findings are matched on original_commit_id:
# GitHub moves a still-valid older comment's commit_id to the new head, so commit_id alone
# shows earlier findings as new.
set -eu

pr=${1:?usage: scripts/codex-review.sh <pr> [--wait-only]}
bot='chatgpt-codex-connector[bot]'
repo=$(gh repo view --json nameWithOwner --jq .nameWithOwner)
head=$(gh pr view "$pr" --json headRefOid --jq .headRefOid)
[ "${2:-}" = --wait-only ] || gh pr comment "$pr" --body '@codex review' >/dev/null
echo "waiting for Codex review of $repo#$pr at $head"

timeout=${CODEX_REVIEW_TIMEOUT:-1800}
start=$(date +%s)
while :; do
  # The summary comment carries one hidden state line per review kind; take the newest.
  state=$(gh api "repos/$repo/issues/$pr/comments" --paginate --jq \
    ".[] | select(.user.login == \"$bot\") | .body | capture(\"codex-review-state:v1 (?<s>\\\\{.*\\\\}) -->\").s | fromjson | .code_review | select(.commit_sha == \"$head\") | \"\\(.status) \\(.has_any_findings)\"" |
    tail -n 1)
  case $state in
    completed\ *) break ;;
    failed* | error* | cancelled*) echo "Codex review $state" >&2; exit 1 ;;
  esac
  if [ $(($(date +%s) - start)) -ge "$timeout" ]; then
    echo "timed out after ${timeout}s; last state: ${state:-none for this head}" >&2
    exit 1
  fi
  sleep 30
done

findings=$(gh api "repos/$repo/pulls/$pr/comments" --paginate --jq \
  ".[] | select(.user.login == \"$bot\" and .original_commit_id == \"$head\") | \"\\(.path):\\(.original_line // .line // \"?\") \\(.body | capture(\"(?<p>P[0-9]) Badge\").p // \"P?\") \\(.body | split(\"\\n\")[0] | gsub(\"<[^>]*>|!\\\\[[^]]*\\\\]\\\\([^)]*\\\\)|\\\\*\\\\*\"; \"\") | sub(\"^ +\"; \"\"))\\n  \\(.html_url)\"")
if [ -z "$findings" ]; then
  echo "completed: no findings on $head"
  exit 0
fi
echo "$findings"
exit 2
