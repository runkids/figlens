# Pull requests and AI review

Use this when delivering a feature or optimization, reviewing a change, opening a PR or handling review feedback. An authorized PR workflow includes its in-scope fixes, commits, pushes and review requests. Merge and release still need their own authorization.

## CLI, MCP and Pi coverage

For every feature or optimization, check all three interfaces before calling the work complete. Trace the actual path and decide whether each needs a change, is already covered by shared code, or is not applicable. Keep this assessment scoped to the requested behavior.

| Interface | Check |
|---|---|
| CLI | `src/main.rs` usage and orchestration, `src/command.rs` parsing/execution, defaults, errors and output |
| MCP | `src/contract.rs` discovery, input/output schemas and preflight; `src/mcp.rs` routing, permissions and warm-process behavior |
| Pi | `pi/index.ts` MCP registration and tools/commands, `pi/workflow.mjs` and `pi/viewer.mjs` consumers, shared skill guidance, and the actual launcher/package used |

1. Complete affected behavior, callers, contracts, help, wiki/README locales, shared skill and relevant tests in the same change. Optimizations need the same sweep: they can change output, limits, caching or long-running behavior even when no option is added.
2. Reuse shared paths. A new inspect argument can require CLI parsing/help, MCP schemas and skill guidance while Pi's adapter already discovers it from the server. Verify that path and record why no adapter edit is needed; avoid adding a duplicate Pi command or configuration just to touch every interface.
3. Run checks proportional to the affected paths. Where CLI/MCP share results, verify equivalent inputs preserve the same values, provenance, unknowns and truncation. Check Pi-specific consumers or packaging when affected; edit maintained sources, with generated copies produced by `scripts/prepare-npm.cjs`. A source/schema check does not establish installed Pi runtime acceptance. Report missing runtime or optimization measurements explicitly.
4. In the handoff or PR, give a short outcome for CLI, MCP and Pi: what changed, or the evidence that no change was needed, plus the checks actually run. Do not infer that an installed client uses new behavior without checking its binary/server version and reload path.

## Before opening a PR

1. Refresh the base and pin the diff. Preserve unrelated edits and exclude private designs and exports.
2. Every PR gets this review, including docs- and wiki-only changes and fixes pushed to an open PR. Simplify the change, then review correctness, repository rules, the requested behavior, consumers and cleanup. The `figlens-code-review` skill adapts these passes to the available tools. Verify each finding against actual code or execution before fixing it.
3. Apply the CLI/MCP/Pi coverage check above to every changed concept. Fix related findings together, run the relevant checks and stage only task-owned files.
4. Describe the final behavior, executed checks and remaining limitations in English. Use a draft for unfinished code or an explicit request; documented model-quality limitations alone do not require a draft.

## Follow the PR through review

Opening a PR is not the end of the workflow. Read its current reviews, conversation, inline comments, review threads and CI results without waiting for the user to point them out. Record the current head SHA and the commit each AI review assessed.

```sh
gh pr view <number> --json headRefOid,reviews,comments,reviewDecision,statusCheckRollup
gh api --paginate repos/<owner>/<repo>/pulls/<number>/comments
```

Use the PR's review threads to distinguish unresolved, resolved and outdated comments; an old inline comment alone does not establish current status.

For each finding, trace the cited code, callers and input that produces the claimed failure. Fix confirmed, applicable findings within the authorized scope, including their consumers and a regression test when practical. For a rejected or deferred finding, explain the evidence and remaining scope in its thread. Treat a material change in requirements or scope as a decision for the user.

After all verified fixes pass their checks:

1. Commit and push the fixes together. Check the remote head SHA matches the tested commit.
2. Update stale PR evidence and resolve fixed threads after verification, linking the fix where useful.
3. Post `@codex review` yourself to request review of the updated head. A push alone is not proof that review was requested. Avoid duplicate requests while that same head is already under review.

   ```sh
   gh pr comment <number> --body '@codex review'
   ```

4. Wait for the AI review of that head to complete and inspect its new findings and CI. Repeat the fix/check/push/review cycle for verified findings. A previous commit's clean review, a dispatch acknowledgement, an eyes reaction or green CI does not establish a clean current review.
5. Report the PR link, reviewed head, review outcome and CI outcome separately. If an external review or check cannot complete, state what is still pending instead of reporting it as passed.

Keep private fixtures and local report paths out of public comments. Model planning quality, deterministic tests and visual acceptance are separate evidence; preserve their actual results throughout follow-up.
