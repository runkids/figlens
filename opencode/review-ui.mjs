import { reviewRpc } from './rpc.mjs';

// Authenticated host clients are trusted; this workflow is not a caller-authentication sandbox.
export async function reviewMappings(ctx, sessionID) {
  const dialog = ctx.ui.dialog;
  const rpc = ctx.client.rpc(reviewRpc);
  const options = { location: ctx.location };
  async function submit(input) {
    try { await rpc.review(input, options); return true; }
    catch (error) {
      if (['permission_denied', 'session_unavailable', 'invalid_input', 'proposal_changed', 'stale_evidence', 'writer_busy', 'result_too_large'].includes(error?.type)) throw error;
      await dialog.alert({ title: 'Review result unconfirmed', message:
        `The server may already have saved this decision. Reloading stored state; no retry will be sent.\n\n${error?.message ?? String(error)}` });
      try {
        const stored = await rpc.pending({ sessionID }, options);
        const pending = stored.candidates.some(candidate => candidate.id === input.candidateId);
        await dialog.alert({ title: 'Stored mapping state', message: pending
          ? 'The proposal is still pending. Review stopped; inspect its evidence before starting another review.'
          : 'This proposal is no longer pending. Use /figlens status to inspect stored mappings before continuing.' });
      } catch (reload) {
        await dialog.alert({ title: 'Stored state unavailable', message:
          `The result remains unconfirmed. Reconnect and reload /figlens status before reviewing again.\n\n${reload?.message ?? String(reload)}` });
      }
      return false;
    }
  }
  const { candidates } = await rpc.pending({ sessionID }, options);
  if (!candidates.length) {
    await dialog.alert({ title: 'Component mappings', message: 'No pending mappings in this project.' });
    return;
  }
  for (const entry of candidates) {
    await dialog.alert({ title: entry.source.name, message: [
      `Design: ${entry.source.file} · frame ${entry.source.frame} · instance ${entry.source.instance}`,
      `Target: ${entry.target.path} · ${entry.target.export}`, entry.target.import, entry.target.example,
      `Rationale: ${entry.rationale}`, `Agent-reported checks (not independently verified): ${entry.checks.join('; ') || 'none'}`,
      ...entry.issues.map(issue => `Needs review: ${issue}`),
    ].join('\n\n') });
    const action = await dialog.select({ title: 'Review component mapping', options: ['Accept', 'Edit', 'Skip', 'Stop'].map(value => ({ title: value, value })) });
    if (!action || action === 'Stop') return;
    if (action === 'Skip') {
      if (!await submit({ sessionID, candidateId: entry.id, action: 'skip' })) return;
      continue;
    }
    let target;
    if (action === 'Edit') {
      target = {};
      for (const key of ['path', 'export', 'import', 'example']) {
        const value = await dialog.prompt({ title: key, value: entry.target[key] });
        if (value === undefined) break;
        target[key] = value;
      }
      if (Object.keys(target).length !== 4) continue;
    }
    const resolution = entry.issues.length ? await dialog.prompt({ title: 'Conflict resolution', description: 'Describe the accepted interpretation of the listed issues.' }) : undefined;
    if (entry.issues.length && !resolution?.trim()) continue;
    const accepted = await dialog.confirm({ title: 'Approve component mapping?', message: [
      JSON.stringify(target ?? entry.target, null, 2), resolution,
      'Approval records your mapping decision; compilation and visual checks remain separate.',
    ].filter(Boolean).join('\n\n') });
    if (!accepted) continue;
    if (!await submit({ sessionID, candidateId: entry.id, action: 'approve', ...(target ? { target } : {}), ...(resolution ? { resolution } : {}) })) return;
  }
}
