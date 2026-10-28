import { Plugin } from '@opencode/plugin/tui';
import { operationRpc } from './rpc.mjs';
import { reviewMappings } from './review-ui.mjs';

export default Plugin.define({ id: 'figlens', setup(ctx) {
  const rpc = ctx.client.rpc(operationRpc);
  async function run(action, sessionID) {
    const route = ctx.ui.router.current();
    if (route.type !== 'session' || route.sessionID !== sessionID) return;
    try {
      if (action === 'review') { await reviewMappings(ctx, sessionID); return; }
      const operation = request => rpc.run({ sessionID, ...request }, { location: ctx.location });
      const status = await operation({ action: 'status' });
      const file = status.active?.file || await ctx.ui.dialog.prompt({ title: 'Local .fig file' });
      if (!file) return;
      const query = await ctx.ui.dialog.prompt({ title: 'Find a screen by name' });
      if (query === undefined) return;
      const frames = await operation({ action: 'frames', file, query });
      const node = await ctx.ui.dialog.select({ title: frames.truncated ? 'First 200 matches; narrow the search for more' : 'Select a screen',
        options: frames.frames.map(frame => ({ title: frame.name, value: frame.id, description: frame.id })) });
      if (!node) return;
      const selected = await operation({ action: 'select', file, node });
      await ctx.ui.dialog.alert({ title: 'Saved selection', message: `${selected.design.name} (${selected.design.frame})\n${selected.design.file}\nAsk OpenCode to implement it. Mapping is optional.` });
      await ctx.client.session.prompt({ sessionID, text: `figlens reference: ${selected.design.name}\n[Open reference PNG on the server machine](${selected.referenceUrl})`, files: [selected.reference], resume: false });
    } catch (error) {
      await ctx.ui.dialog.alert({ title: 'figlens operation failed', message: error?.message ?? String(error) });
    }
  }
  // Keep slash submission in the host so files, agent/skill mentions and delivery survive.
  ctx.keymap.layer(() => ({ commands: ['review', 'frame'].map(action => ({
    id: `figlens.${action}`, title: `figlens ${action}`, palette: true,
    run: async () => {
      const route = ctx.ui.router.current();
      if (route.type === 'session') await run(action, route.sessionID);
    },
  })) }));
  return rpc.events.on('manual', event => run(event.data.action, event.data.sessionID));
} });
