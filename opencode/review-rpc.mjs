import { realpath } from 'node:fs/promises';
import { WorkflowAuthorizationError } from '../pi/workflow.mjs';

import { reviewRpc } from './rpc.mjs';

class ReviewError extends Error {
  constructor(code, message) { super(message); this.code = code; }
}

export async function registerReview(ctx, { scope }) {
  async function handle(input, call, operation) {
    let signal = call.signal;
    try {
      if (Buffer.byteLength(JSON.stringify(input)) > 64 * 1024) throw new ReviewError('invalid_input', 'Review request exceeds 64 KiB.');
      signal.throwIfAborted();
      let session;
      try { session = await ctx.session.get({ sessionID: input.sessionID }); }
      catch { throw new ReviewError('session_unavailable', 'Session is unavailable; reopen review in the current session.'); }
      const root = await realpath(ctx.location.directory);
      if (!session.location?.directory || await realpath(session.location.directory) !== root) {
        throw new ReviewError('session_unavailable', 'Review session belongs to a different project directory.');
      }
      const current = await scope(session.id, call.signal);
      signal = current.signal;
      signal.throwIfAborted();
      const output = await operation(current.flow, signal);
      // Reserve envelope space; never truncate the evidence presented for approval.
      if (Buffer.byteLength(JSON.stringify(output)) > 4 * 1024 * 1024 - 1024) throw new ReviewError('result_too_large', 'Review list exceeds the reply budget. Review a smaller batch in Pi, then reload.');
      return output;
    } catch (failure) {
      const error = failure instanceof WorkflowAuthorizationError ? failure.cause : failure;
      if (signal.aborted || error?.name === 'AbortError' || error?._tag === 'Permission.DeclinedError') return call.error('cancelled', 'Review cancelled; reload the stored state before retrying.', null);
      if (error?._tag === 'Tool.Error') return call.error('permission_denied', error.message, null);
      if (error instanceof ReviewError) return call.error(error.code, error.message, null);
      const message = error?.message ?? String(error);
      const code = /Proposal changed or was already reviewed/.test(message) ? 'proposal_changed'
        : /(?:Design|Code) evidence changed/.test(message) ? 'stale_evidence'
          : /Another mapping writer holds/.test(message) ? 'writer_busy'
            : /must be non-empty text|Code evidence (?:paths|must)|Review resolution/.test(message) ? 'invalid_input' : 'storage_error';
      return call.error(code, message, null);
    }
  }
  return ctx.rpc.register(reviewRpc, {
    pending: (input, call) => handle(input, call, async flow => ({ candidates: await flow.pending() })),
    review: (input, call) => handle(input, call, async (flow, signal) => {
      if (input.action === 'skip' && (input.target || input.resolution)) throw new ReviewError('invalid_input', 'Skip does not accept target edits or a resolution.');
      const result = await flow.review(input.candidateId, input.action, { target: input.target, resolution: input.resolution }, signal);
      return { id: result.id, status: result.status };
    }),
  });
}
