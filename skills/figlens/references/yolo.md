# Plan, implement, verify and clean up

Use only when the user explicitly authorizes autonomous design implementation and temporary-plan cleanup, including Pi's `/figlens yolo`. Keep the work in the same agent context. This workflow adds no planner agent, permission bypass or task scheduler.

## Plan before editing the application

Resolve the current request's `.fig`, recognizable screen/region and target project. Read its actual architecture, neighboring implementations, applicable instructions and image pipeline. Use [the planning workflow](planning.md) to inspect only the authorized design scope and dependencies and make the next slice actionable.

Create a small working plan before application edits. Use the project's plan conventions and the [saved-plan namespaces](planning-details.md#save-and-resume-proportionately). Choose a new, unoccupied path for this run and record exactly which plan files you create; keep an existing plan as read-only context. The working plan identifies its source/scope, implementation steps and observable acceptance checks. Do not create a whole-site inventory for a local change.

Resolve routine implementation choices from project evidence and state material assumptions in the plan. Optional metadata, such as an online Figma file key, can remain unknown when the verified local file and frame establish the scope. Continue directly into the authorized implementation without a plan-approval question. If the source, scope or another indispensable prerequisite cannot be established, report the blocker and preserve the plan rather than guessing another design or widening the task. Host permissions still apply.

## Build, check and correct

Implement coherent slices using the project's components, tokens, layout and asset conventions. Follow the main skill's image delivery and reference-comparison procedure. Update the working plan with observed progress rather than repeating broad discovery.

After the first pass, compare the running screen at the requested viewport with the design evidence, verify typography, layout, crops and image delivery, and run the project's relevant build/runtime/interaction checks. Correct confirmed discrepancies and rerun the affected checks. Stop when acceptance is met; if progress needs unavailable evidence, permissions or a decision that cannot be inferred, report the unresolved condition instead of retrying without a new hypothesis.

## Clean up after verified completion

Delete only the temporary plan files created by this run, after implementation and the applicable acceptance checks succeed. Record their contents after your own updates; immediately before deletion, re-read each exact path and verify it still belongs to this run and matches that last known content. Preserve any plan with unexpected edits, user-added notes or uncertain ownership. Remove an empty plan directory only if this run created it. Avoid recursive cleanup of `.figlens`, shared namespaces or wildcard-matched plans.

Keep delivered code/assets, existing plans, source designs and useful verification evidence. Keep figlens workspaces too: their databases, registry and any index are durable state, not task-owned temporaries; never run `figlens index reset` or `untrack` as cleanup (see [Search and local index](../SKILL.md#search-and-local-index)). Failure, interruption, blocked or unavailable required verification leaves the working plan available for continuation. If the user asks to retain it, follow that instruction.

Return the implemented scope, checks actually run, remaining known differences and the actual plan-cleanup outcome. Cleanup failure is a reported remaining task, not permission to claim the plan was removed. A successful build alone does not establish visual acceptance.
