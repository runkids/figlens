// OMP commands are trusted extension code; model tools cannot call this review path.
export async function reviewMappings(flow, ctx, show, signal) {
  const pending = await flow.pending();
  if (!pending.length) { show('No pending component mappings in this project.'); return; }
  if (!ctx.hasUI) { show(`${pending.length} pending mapping(s). Review requires interactive OMP. Proposals remain pending.`); return; }
  for (const entry of pending) {
    signal?.throwIfAborted();
    show([
      `${entry.source.name} (${entry.source.instance}) → ${entry.target.path}`,
      `Design: ${entry.source.file} · frame ${entry.source.frame}`,
      `Export: ${entry.target.export}`, entry.target.import, entry.target.example,
      `Rationale: ${entry.rationale}`,
      `Agent-reported checks (not independently verified): ${entry.checks.join('; ') || 'none'}`,
      ...entry.issues.map(issue => `Needs review: ${issue}`),
    ].join('\n\n'));
    const action = await ctx.ui.select('Review this component mapping', ['Accept', 'Edit', 'Skip', 'Stop'], { signal });
    if (!action || action === 'Stop') return;
    if (action === 'Skip') { await flow.review(entry.id, 'skip', {}, signal); continue; }
    let target;
    if (action === 'Edit') {
      target = {};
      for (const key of ['path', 'export', 'import', 'example']) {
        const value = await ctx.ui.input(`${key} (leave empty to keep): ${entry.target[key]}`, undefined, { signal });
        if (value === undefined) break;
        target[key] = value || entry.target[key];
      }
      if (Object.keys(target).length !== 4) continue;
    }
    const resolution = entry.issues.length
      ? await ctx.ui.input('Resolve the listed issues: describe the accepted interpretation', undefined, { signal }) : undefined;
    if (entry.issues.length && !resolution?.trim()) continue;
    if (target) show(`Edited target:\n\n${JSON.stringify(target, null, 2)}`);
    if (resolution) show(`Conflict resolution:\n\n${resolution}`);
    if (!await ctx.ui.confirm('Approve the mapping shown above?',
      `${entry.source.name} → ${(target ?? entry.target).path}\n\nApproval records your mapping decision; compilation and visual checks remain separate.`, { signal })) continue;
    signal?.throwIfAborted();
    await flow.review(entry.id, 'approve', { ...(target ? { target } : {}), resolution }, signal);
  }
  show(`Review finished. ${(await flow.pending()).length} proposal(s) remain pending.`);
}
