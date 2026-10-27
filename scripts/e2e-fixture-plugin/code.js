// Experiment-only, self-owned artwork; run in a new blank Figma Design file.
(async () => {
  const page = figma.currentPage;
  let frame;
  try {
    if (figma.root.children.length !== 1 || page.children.length !== 0) {
      throw new Error('Open a new blank Figma Design file before running this plugin.');
    }
    const fontName = { family: 'Source Code Pro', style: 'Regular' };
    await figma.loadFontAsync(fontName);
    const paint = hex => [{
      type: 'SOLID',
      color: {
        r: parseInt(hex.slice(0, 2), 16) / 255,
        g: parseInt(hex.slice(2, 4), 16) / 255,
        b: parseInt(hex.slice(4, 6), 16) / 255,
      },
    }];
    frame = figma.createFrame();
    frame.name = 'Owned fixture / Task list';
    frame.resize(960, 640);
    frame.fills = paint('F7F8FC');
    frame.strokes = paint('C7CBD8');
    frame.strokeWeight = 1;
    frame.strokeAlign = 'OUTSIDE';
    frame.cornerRadius = 20;
    frame.clipsContent = true;
    const box = (name, x, y, width, height, fill, radius) => {
      const node = figma.createFrame();
      frame.appendChild(node);
      node.name = name;
      node.x = x; node.y = y;
      node.resize(width, height);
      node.fills = paint(fill);
      node.strokes = [];
      node.cornerRadius = radius;
      return node;
    };
    const text = (name, value, x, y, size, color) => {
      const node = figma.createText();
      frame.appendChild(node);
      node.name = name;
      node.fontName = fontName;
      node.fontSize = size;
      node.lineHeight = { unit: 'PIXELS', value: size + 8 };
      node.textAutoResize = 'WIDTH_AND_HEIGHT';
      node.characters = value;
      node.fills = paint(color);
      node.x = x; node.y = y;
      return node;
    };
    box('Brand mark', 40, 32, 24, 24, '6056D6', 7);
    text('Product name', 'FIELDNOTES', 76, 30, 16, '242738');
    text('Workspace label', 'PERSONAL WORKSPACE', 726, 32, 12, '74798C');
    text('Page title', 'A quieter task list', 40, 86, 28, '242738');
    text('Page description', 'A small plan for a focused afternoon.', 40, 134, 14, '74798C');
    box('New task button', 776, 96, 144, 40, '6056D6', 10);
    text('New task label', '+ New task', 788, 102, 14, 'FFFFFF');
    const card = box('Task list background', 40, 192, 880, 352, 'FFFFFF', 16);
    card.strokes = paint('E1E4ED');
    card.strokeWeight = 1;
    card.strokeAlign = 'INSIDE';
    const tasks = [
      ['Review design notes', 'Check spacing and typography', 'Ready', 'EDEBFF', '6056D6'],
      ['Build the settings page', 'Use editable text and supplied icons', 'In progress', 'E7F1FF', '35659B'],
      ['Check keyboard navigation', 'Confirm focus order and labels', 'Blocked', 'FFF0DE', '9B6426'],
      ['Share the first draft', 'Keep the scope small', 'Ready', 'EDEBFF', '6056D6'],
    ];
    for (let i = 0; i < tasks.length; i++) {
      const [title, description, status, fill, color] = tasks[i];
      const y = 208 + i * 80;
      box('Task row / ' + (i + 1), 56, y, 848, 64, i === 0 ? 'F7F6FF' : 'FFFFFF', 10);
      const icon = figma.createEllipse();
      frame.appendChild(icon);
      icon.name = 'Task icon / ' + (i + 1);
      icon.x = 72; icon.y = y + 23;
      icon.resize(18, 18);
      icon.fills = i === 0 ? paint('6056D6') : [];
      icon.strokes = paint(i === 0 ? '6056D6' : 'B3B8C9');
      icon.strokeWeight = 1.5;
      icon.strokeAlign = 'INSIDE';
      text('Task title / ' + (i + 1), title, 106, y + 6, 15, '242738');
      text('Task description / ' + (i + 1), description, 106, y + 32, 11, '74798C');
      box('Task status pill / ' + (i + 1), 768, y + 18, 120, 28, fill, 14);
      text('Task status / ' + (i + 1), status, 778, y + 21, 11, color);
    }
    text('List count', '4 tasks / 1 selected', 40, 578, 12, '74798C');
    text('Footer note', 'Less noise. More focus.', 736, 578, 12, '74798C');
    frame.exportSettings = [{ format: 'PNG', constraint: { type: 'SCALE', value: 1 }, suffix: '' }];
    page.name = 'Owned comparison';
    page.selection = [frame];
    figma.viewport.scrollAndZoomIntoView([frame]);
    figma.closePlugin('Fixture created. Save a local .fig copy and export this frame at 1x PNG.');
  } catch (error) {
    if (frame) frame.remove();
    figma.closePlugin('No fixture kept: ' + error.message);
  }
})()
