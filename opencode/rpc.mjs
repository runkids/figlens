const sessionID = { type: 'string', minLength: 1, maxLength: 200 };
const errors = Object.fromEntries(['permission_denied', 'cancelled', 'session_unavailable',
  'invalid_input', 'proposal_changed', 'stale_evidence', 'writer_busy', 'storage_error', 'result_too_large']
  .map(name => [name, { type: 'null' }]));

export const reviewRpc = {
  id: 'figlens-review', events: {}, methods: {
    pending: {
      input: { type: 'object', properties: { sessionID }, required: ['sessionID'], additionalProperties: false },
      output: { type: 'object', properties: { candidates: { type: 'array', items: { type: 'object' } } }, required: ['candidates'], additionalProperties: false }, errors,
    },
    review: {
      input: { type: 'object', properties: {
        sessionID, candidateId: { type: 'string', format: 'uuid' }, action: { enum: ['approve', 'skip'] },
        target: { type: 'object', properties: Object.fromEntries([['path', 4096], ['export', 200], ['import', 8000], ['example', 8000]]
          .map(([name, maxLength]) => [name, { type: 'string', minLength: 1, maxLength }])),
        required: ['path', 'export', 'import', 'example'], additionalProperties: false },
        resolution: { type: 'string', minLength: 1, maxLength: 2000 },
      }, required: ['sessionID', 'candidateId', 'action'], additionalProperties: false },
      output: { type: 'object', properties: { id: { type: 'string' }, status: { enum: ['approved', 'skipped'] } }, required: ['id', 'status'], additionalProperties: false }, errors,
    },
  },
};

export const operationRpc = {
  id: 'figlens-operation', events: { manual: { schema: {
    type: 'object', properties: { sessionID, action: { enum: ['review', 'frame'] } },
    required: ['sessionID', 'action'], additionalProperties: false,
  } } }, methods: {
    run: {
      input: { type: 'object', properties: { sessionID,
        action: { enum: ['status', 'frames', 'select', 'preview', 'close'] },
        file: { type: 'string', minLength: 1, maxLength: 4096 },
        node: { type: 'string', minLength: 1, maxLength: 200 },
        query: { type: 'string', maxLength: 2000 },
      }, required: ['sessionID', 'action'], additionalProperties: false },
      output: { type: 'object' }, errors,
    },
  },
};


export const HELP = 'Ask OpenCode to build a screen from a local .fig file. Searching a design reads the .fig directly and stores nothing; for repeated searches, ask it to enable an index for a design and scope: it prepares a local index once (no setup command), reports the job until ready and reuses it across restarts and hosts; resetting the index needs your explicit request and the figlens_index permission. Asking where a component, variable, style or image is used lists its exact usages the same way: from the .fig directly, or from that index once enabled. Across every design with an index, it also lists where a library component, style or variable is used, by the library reference that outgoing usages on a layer names. Asking it to follow the changes of a design tracks a scope of that index and lists what changed since the last change it handled; asking it to watch that scope picks up each save for the session. Optional: /figlens open|preview|plan|yolo|map [request]; /figlens status|frame|review|close. Known scope: /figlens open "./design.fig" --frame 0:1. Plan writes a plan only; yolo authorizes scoped implementation. Mappings stay pending until native review. Viewer URLs belong to the server machine; remote clients need their own access.';
