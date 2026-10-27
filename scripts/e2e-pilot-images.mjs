// Loaded only by the comparison harness, never installed globally.
export default function register(pi) {
  pi.on('tool_result', event => {
    if (!event.toolName.startsWith('figma_') || event.content.some(block => block.type === 'image')) return;
    const images = event.details?.raw?.content?.filter(block => block.type === 'image') ?? [];
    if (images.length === 0) return;
    return { content: [...event.content, ...images], structuredContent: event.structuredContent };
  });
}
