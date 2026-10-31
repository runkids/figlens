# Plan, implement, verify and clean up

Use only when the user explicitly authorizes autonomous design implementation and cleanup of what the run created, including Pi's `/figlens yolo`. Keep the work in the same agent context. This workflow adds no planner agent, permission bypass or task scheduler.

## Plan before editing the application

Resolve the current request's `.fig`, recognizable screen/region and target project. Read its actual architecture, neighboring implementations, applicable instructions and image pipeline. Use [the planning workflow](planning.md) to inspect only the authorized design scope and dependencies and make the next slice actionable.

Create a small working plan before application edits. Use the project's plan conventions and the [saved-plan namespaces](planning-details.md#save-and-resume-proportionately). Choose a new, unoccupied path for this run and record exactly which plan files you create; keep an existing plan as read-only context. Likewise choose a new, unoccupied export directory for this run, following the plan's naming (`.figlens/exports/<scope>/` or `<design>/<scope>/`) unless the user names one, and write every bundle, extracted file, reference image, screenshot, scratch script and log for your own checking there, never loose in `.figlens/exports/`. Record that directory in the plan. The working plan identifies its source/scope, implementation steps and observable acceptance checks. Do not create a whole-site inventory for a local change.

Resolve routine implementation choices from project evidence and state material assumptions in the plan. Optional metadata, such as an online Figma file key, can remain unknown when the verified local file and frame establish the scope. Continue directly into the authorized implementation without a plan-approval question. If the source, scope or another indispensable prerequisite cannot be established, report the blocker and preserve the plan rather than guessing another design or widening the task. Host permissions still apply.

## Build, check and correct

Implement coherent slices using the project's components, tokens, layout and asset conventions. Follow the main skill's image delivery and reference-comparison procedure. Update the working plan with observed progress rather than repeating broad discovery.

After the first pass, compare the running screen at the requested viewport with the design evidence, verify typography, layout, crops and image delivery, and run the project's relevant build/runtime/interaction checks. Correct confirmed discrepancies and rerun the affected checks. When a check leads you to depart from the design, such as darkening text for contrast, record each departure with the design value, the delivered value and the reason; it is a known difference, not a match. Stop when acceptance is met; if progress needs unavailable evidence, permissions or a decision that cannot be inferred, report the unresolved condition instead of retrying without a new hypothesis.

## Clean up after verified completion

Delete only the temporary plan files created by this run, after implementation and the applicable acceptance checks succeed. Record their contents after your own updates; immediately before deletion, re-read each exact path and verify it still belongs to this run and matches that last known content. Preserve any plan with unexpected edits, user-added notes or uncertain ownership. Remove an empty plan directory only if this run created it. Avoid recursive cleanup of `.figlens`, shared namespaces or wildcard-matched plans.

In the run's own export directory, keep the verification summary and the final acceptance screenshot, and delete the rest: bundle ZIPs and their extracted files, intermediate screenshots, scratch scripts and logs can all be regenerated. Delete nothing outside that directory: `.figlens/component-mappings.md`, `.figlens/previews/`, other exports and shipped assets stay. If the directory was not created by this run, delete only the files this run wrote there.

Stop the dev servers, browser sessions and other processes this run started, and only those; leave any that were running before. Report a process you could not stop with its PID.

Keep delivered code/assets, existing plans and source designs. Keep figlens workspaces too: their databases, registry and any index are durable state, not task-owned temporaries; never run `figlens index reset` or `untrack` as cleanup (see [Search and local index](../SKILL.md#search-and-local-index)). Start no watch lease, subscription or tracking unless the user asked for one; the only watch state a run removes is a subscription it created with an `owner`, through `watch unsubscribe` with that consumer and owner, never `forget`, another owner's or an ownerless one. Failure, interruption, blocked or unavailable required verification leaves the working plan and the run's export directory available for continuation. If the user asks to retain them, follow that instruction.

Return the implemented scope, checks actually run, remaining known differences including recorded design departures, and the actual cleanup outcome: plan, export files kept and removed, and processes stopped. Cleanup failure is a reported remaining task, not permission to claim it was done. A successful build alone does not establish visual acceptance.
