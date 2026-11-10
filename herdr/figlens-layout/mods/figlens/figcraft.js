// FigCraft's optional CLI adapter. Keep this module independent of Herdr so its data helpers can
// be tested with node:test; FigCraft ids are integers, unlike Figma's colon-separated ids.

export const isFigCraft = (file) => String(file?.path ?? "").toLowerCase().endsWith(".figcraft");
export const nodeRef = (text) => {
  const value = String(text ?? "").trim();
  return /^\d+$/.test(value) ? value : null;
};

// FigCraft has no max-size option: use fractional scales to keep large exports bounded.
export function renderScale(size, max) {
  const longest = Math.max(1, ...(Array.isArray(size) ? size.map(Number).filter(Number.isFinite) : []));
  const target = Number.isFinite(max) && max > 0 ? max : 1;
  return Math.min(16, Math.max(0.01, target / longest));
}

// FigCraft reports the top-level frames grouped by page.
export function groups(info) {
  return (info?.pages ?? [])
    .map((page) => ({
      id: Number(page.id),
      name: typeof page.name === "string" ? page.name : "",
      frames: (page.frames ?? []).filter((frame) => frame.type !== "SECTION").map((frame) => ({
        id: Number(frame.id),
        name: typeof frame.name === "string" ? frame.name : "",
        type: frame.type,
        size: [frame.width, frame.height],
      })).filter((frame) => Number.isSafeInteger(frame.id) && frame.id >= 0),
    }))
    .filter((page) => Number.isSafeInteger(page.id) && page.frames.length);
}

// Index the CLI's converted node tree once per saved revision.
export function buildIndex(doc) {
  const index = new Map();
  for (const [key, node] of Object.entries(doc?.nodes ?? {})) {
    const id = Number(node.id ?? key);
    if (!Number.isSafeInteger(id) || id < 0) continue;
    index.set(id, {
      id,
      name: typeof node.name === "string" ? node.name : "",
      type: String(node.type ?? "").toUpperCase(),
      text: typeof node.characters === "string" ? node.characters : "",
      width: node.width,
      height: node.height,
      parent: node.parent == null ? null : Number(node.parent),
      children: Array.isArray(node.children) ? node.children.map(Number).filter((child) => Number.isSafeInteger(child) && child >= 0) : [],
    });
  }
  return index;
}

export function firstTextOf(id, index, maxLen = 120) {
  const seen = new Set(), queue = [...(index.get(Number(id))?.children ?? [])];
  let cursor = 0;
  while (cursor < queue.length) {
    const node = index.get(queue[cursor++]);
    if (!node || seen.has(node.id)) continue;
    seen.add(node.id);
    if (node.text.trim()) return node.text.trim().slice(0, maxLen);
    queue.push(...node.children);
  }
  return "";
}

function topHas(pages, id) {
  for (const page of pages) {
    for (const frame of page.frames) if (frame.id === id) return { frame, page };
  }
  return null;
}

export function placement(id, index, pages) {
  let node = index.get(Number(id));
  if (!node || node.type === "PAGE") return null;
  const ownFrame = topHas(pages, node.id);
  if (ownFrame) return { ...ownFrame, isScreen: true };

  const seen = new Set();
  while (node?.parent != null && !seen.has(node.id)) {
    seen.add(node.id);
    node = index.get(node.parent);
    if (!node) break;
    const ancestorFrame = topHas(pages, node.id);
    if (ancestorFrame) return ancestorFrame;
  }
  return { page: node?.type === "PAGE" ? pages.find((page) => page.id === node.id) ?? null : null, frame: null };
}

export function searchIndex(index, query, pages, { limit = 30 } = {}) {
  const needle = String(query ?? "").toLowerCase().trim();
  if (!needle || limit < 1) return { rows: [], truncated: false };
  const topFrames = new Set(pages.flatMap((page) => page.frames.map((frame) => frame.id)));
  const rows = [];
  for (const node of index.values()) {
    if (node.type === "PAGE" || topFrames.has(node.id)) continue;
    if (!node.name.toLowerCase().includes(needle) && !node.text.toLowerCase().includes(needle)) continue;
    const at = placement(node.id, index, pages) ?? {};
    rows.push({ id: node.id, name: node.name || String(node.id), type: node.type, text: node.text,
      size: Number.isFinite(node.width) && Number.isFinite(node.height) ? [node.width, node.height] : null,
      frame: at.frame ?? null, page: at.page ?? null });
    if (rows.length > limit) return { rows: rows.slice(0, limit), truncated: true };
  }
  return { rows, truncated: false };
}
