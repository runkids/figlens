#!/usr/bin/env python3
"""Bounded, private one-frame pilot; not a general benchmark runner."""
import base64
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import random
import shutil
import signal
import subprocess
import sys
import time

ROOT = Path(sys.argv[2]).resolve()
SESSION = 'figlens-e2e-pilot-setup'


def browser(script):
    result = subprocess.run(['agent-browser', '--session', SESSION, 'eval', '--stdin'],
                            input=script, text=True, capture_output=True, check=True, timeout=40)
    return json.loads(result.stdout)


def save_json(path, value):
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + '\n')


def missing_text(expected, observed):
    # shortcut: compare whole text-bearing elements; split/merged labels need a richer DOM audit.
    available = Counter(' '.join(text.split()) for text in observed)
    missing = []
    for text in expected:
        normalized = ' '.join(text.split())
        if available[normalized]:
            available[normalized] -= 1
        else:
            missing.append(text)
    return missing


def prepare():
    common = ROOT / 'common'
    (common / 'fonts').mkdir(parents=True, exist_ok=True)
    (common / 'icons').mkdir(exist_ok=True)
    for name in ['Arial.ttf', 'Arial Bold.ttf', 'Georgia.ttf']:
        shutil.copyfile(Path('/System/Library/Fonts/Supplemental') / name,
                        common / 'fonts' / name.replace(' ', '-'))
    shutil.copyfile(ROOT / 'official/before-reference/get_screenshot-0.png', ROOT / 'ground-truth.png')
    sources = sorted((ROOT / 'figlens/assets').glob('*.svg'))
    rasterized = browser('''(async () => {
      const files = FILES;
      const out = [];
      for (const url of files) {
        const img = new Image(); img.src = url; await img.decode();
        const canvas = document.createElement('canvas');
        canvas.width = img.width * 2; canvas.height = img.height * 2;
        canvas.getContext('2d').drawImage(img, 0, 0, canvas.width, canvas.height);
        out.push(canvas.toDataURL('image/png').split(',')[1]);
      }
      return out;
    })()'''.replace('FILES', json.dumps(['/figlens/assets/' + p.name for p in sources])))
    assert len(rasterized) == len(sources) and sources
    mapping = {}
    for i, (source, data) in enumerate(zip(sources, rasterized)):
        name = f'icon-{i:02d}.png'
        (common / 'icons' / name).write_bytes(base64.b64decode(data))
        mapping[source.name] = name
    css = '''@font-face{font-family:PilotSans;src:url("fonts/Arial.ttf");font-weight:400}
@font-face{font-family:PilotSans;src:url("fonts/Arial-Bold.ttf");font-weight:600 900}
@font-face{font-family:PilotSerif;src:url("fonts/Georgia.ttf");font-weight:400}
*{box-sizing:border-box}html,body{margin:0;width:1441px;height:1025px;overflow:hidden}
body{font-family:PilotSans,Arial,sans-serif}#frame{position:relative;margin:.5px;width:1440px;height:1024px;overflow:hidden}
'''
    starter = '<!doctype html>\n<html lang="en"><meta charset="utf-8"><title>Pilot</title><link rel="stylesheet" href="base.css"><link rel="stylesheet" href="styles.css"><body><main id="frame"></main></body></html>\n'
    for arm in ['screenshot', 'official', 'figlens']:
        run = ROOT / 'runs' / arm
        project = run / 'project'
        project.mkdir(parents=True)
        shutil.copytree(common / 'fonts', project / 'fonts')
        shutil.copytree(common / 'icons', project / 'icons')
        (project / 'base.css').write_text(css)
        (project / 'styles.css').write_text('/* Add the screen styles here. */\n')
        (project / 'index.html').write_text(starter)
        design = run / 'design'
        design.mkdir()
        if arm == 'screenshot':
            shutil.copyfile(ROOT / 'ground-truth.png', design / 'reference.png')
        if arm == 'figlens':
            shutil.copytree(ROOT / 'figlens', design / 'bundle')
            save_json(design / 'common-icon-map.json', mapping)
        instructions = '''Implement the supplied design as a static, editable HTML/CSS screen in project/index.html and project/styles.css. Reproduce the visible text, layout, colors, borders, icons, and typography as closely as possible. This pilot requires only the desktop view, not a backend or responsive behavior.

Work only inside this run directory. Do not inspect parent directories, other runs, repository files, global agent configuration, credentials, or other design sources. Do not use network resources except the permitted design tool and the local project URL below. Do not install packages or tools, modify the Figma document, or publish anything. Use real DOM text and layout: no whole-frame screenshot, SVG, canvas rendering, or generated layer-tree-to-HTML converter as the implementation. Small supplied icon PNGs are permitted.

Keep project/base.css unchanged. The image reference may be downsampled: implement a 1440x1024 frame inside the existing 1441x1025 canvas at offset (0.5,0.5). The frame name is List views. Use PilotSans (Arial) for sans-serif and PilotSerif (Georgia) for serif text. Original Anthropic fonts are unavailable; this identical fallback is mandatory in every arm. Common icons are at project/icons/ as 2x PNGs; inspect these when helpful. Do not use bundled SVGs in the implementation; use the common PNG icons or simple CSS shapes.

Use read/write/edit/bash tools. Browser inspection is available through agent-browser CLI with your isolated session from AGENT_BROWSER_SESSION. Your project URL is URL. After editing, inspect it at viewport 1441x1025, devicePixelRatio 1, and wait for document.fonts.ready before screenshots. Use read to view screenshots. All iterations must fit the one 10-minute attempt. Do not ask questions. Finish with a short list of files changed, checks run, and known deviations.

DESIGN_INPUT
'''.replace('URL', f'http://127.0.0.1:8876/runs/{arm}/project/index.html')
        arm_input = {
            'screenshot': 'Your only design input is design/reference.png. Read this image; no node/layout/style metadata is supplied.',
            'official': 'Your design input is the official Figma desktop MCP, node 467:24651 in the currently open original pilot document. Use figma_get_design_context and figma_get_screenshot, with metadata/variables or smaller subnodes if needed. Setup observed root get_design_context timeouts; preserve/report tool failures and use available official tools rather than guessing that context was obtained. Do not use figlens or external Figma endpoints. Treat generated React/Tailwind as design evidence, adapt it to this static project and its existing font/style setup. Use supplied assets, not new icon packages. Validate against the official screenshot.',
            'figlens': 'Your design input is the frozen figlens 0.7.3 handoff in design/bundle/. Read context.md, design/nodes.json, variables, and the reference image as needed. This is output from the saved .fig; its image is figlens rendering, not Figma. design/common-icon-map.json maps bundle SVG filenames to the common icon PNG filenames. Do not contact Figma or read the independent Figma reference.',
        }[arm]
        (run / 'prompt.md').write_text(instructions.replace('DESIGN_INPUT', arm_input))
    order = ['screenshot', 'official', 'figlens']
    random.Random(20261027).shuffle(order)
    save_json(ROOT / 'run-order.json', {'seed': 20261027, 'order': order})
    save_json(ROOT / 'pilot-controls.json', {
        'model': 'openai-codex/gpt-6.1-sol', 'thinking': 'medium',
        'referencePixels': [1024, 729], 'browserPixels': [1441, 1025], 'dpr': 1,
        'pixelComparison': 'Browser Canvas drawImage explicitly resamples each browser PNG to 1024x729; reference is unchanged.',
        'fonts': 'Arial 400/700 and Georgia 400, common local files; independent reference uses original design fonts.',
        'commonIcons': len(mapping), 'source': 'Rasterized figlens leaf SVGs at 2x; no IDs or absolute geometry supplied to screenshot arm.',
        'attemptsPerArm': 1, 'wallLimitSeconds': 600, 'assetRedistributionCleared': False,
        'officialSetupFailures': 'Root get_design_context timed out twice at 60s; not counted as agent runs.',
        'scope': 'One-frame exploratory pilot with precomputed figlens handoff and warm tools, not a full cold-start timing benchmark.',
    })
    print(json.dumps({'order': order, 'commonIcons': len(mapping), 'prepared': str(ROOT)}))


def run():
    order = json.loads((ROOT / 'run-order.json').read_text())['order']
    for arm in order:
        directory = ROOT / 'runs' / arm
        assert not (directory / 'events.jsonl').exists(), 'Never overwrite or reroll an attempt'
        tools = 'read,write,edit,bash'
        command = ['pi', '--offline', '--print', '--mode', 'json', '--no-session',
                   '--no-extensions', '--no-mcp', '--no-skills', '--no-prompt-templates', '--no-context-files',
                   '--model', 'openai-codex/gpt-6.1-sol', '--thinking', 'medium']
        if arm == 'official' or arm.startswith('official-'):
            command += ['--extension', str(Path.home() / '.pi/agent/npm/node_modules/pi-figma-mcp/extensions/figma-mcp/index.ts'),
                        '--extension', str(Path(__file__).with_name('e2e-pilot-images.mjs').resolve())]
            tools += ',figma_get_design_context,figma_get_screenshot,figma_get_metadata,figma_get_variable_defs'
        command += ['--tools', tools, (directory / 'prompt.md').read_text()]
        save_json(directory / 'command.json', command)
        env = dict(os.environ, AGENT_BROWSER_SESSION=f'figlens-e2e-pilot-{arm}')
        started = time.time()
        with (directory / 'events.jsonl').open('x') as out, (directory / 'stderr.log').open('x') as err:
            process = subprocess.Popen(command, cwd=directory, env=env, stdout=out, stderr=err, start_new_session=True)
            timed_out = False
            try:
                code = process.wait(timeout=600)
            except subprocess.TimeoutExpired:
                timed_out = True
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    code = process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    code = process.wait()
        files = {str(p.relative_to(directory)): hashlib.sha256(p.read_bytes()).hexdigest()
                 for p in sorted((directory / 'project').rglob('*')) if p.is_file()}
        result = {'arm': arm, 'startedEpoch': started, 'endedEpoch': time.time(),
                  'wallSeconds': time.time() - started, 'exitCode': code, 'timedOut': timed_out,
                  'fileHashes': files}
        save_json(directory / 'result.json', result)
        print(json.dumps({k: v for k, v in result.items() if k != 'fileHashes'}), flush=True)
        subprocess.run(['agent-browser', '--session', env['AGENT_BROWSER_SESSION'], 'close'],
                       capture_output=True, timeout=20)


def evaluate():
    subprocess.run(['agent-browser', '--session', SESSION, 'open', 'http://127.0.0.1:8876/'], check=True, timeout=30)
    expected = []
    def text_nodes(node):
        if node.get('visible') is False:
            return
        if node.get('type') == 'TEXT' and node.get('characters'):
            expected.append(node['characters'])
        for child in node.get('children', []):
            text_nodes(child)
    text_nodes(json.loads((ROOT / 'figlens/design/nodes.json').read_text())['document'])
    metrics = {}
    for arm in json.loads((ROOT / 'run-order.json').read_text())['order']:
        directory = ROOT / 'runs' / arm
        assert (directory / 'result.json').is_file(), 'Evaluate only completed or bounded-out attempts'
        subprocess.run(['agent-browser', '--session', SESSION, 'set', 'viewport', '1441', '1025'], check=True, timeout=20)
        url = f'http://127.0.0.1:8876/runs/{arm}/project/index.html'
        subprocess.run(['agent-browser', '--session', SESSION, 'open', url], check=True, timeout=30)
        state = browser('''(async () => {
          await document.fonts.ready;
          await Promise.all(Array.from(document.images).map(i => i.decode().catch(() => null)));
          return {
            viewport:[innerWidth,innerHeight], dpr:devicePixelRatio, userAgent:navigator.userAgent, text:document.body.innerText,
            fonts:Array.from(document.fonts).map(f=>({family:f.family,status:f.status,weight:f.weight})),
            resources:performance.getEntriesByType('resource').map(r=>r.name),
            images:Array.from(document.images).map(i=>({src:i.getAttribute('src'),width:i.width,height:i.height,naturalWidth:i.naturalWidth})),
            canvasCount:document.querySelectorAll('canvas').length,
            textElements:Array.from(document.querySelectorAll('body *')).filter(e=>Array.from(e.childNodes).some(n=>n.nodeType===3 && n.textContent.trim())).map(e=>({tag:e.tagName,text:e.innerText,box:JSON.parse(JSON.stringify(e.getBoundingClientRect())),font:getComputedStyle(e).fontFamily}))
          };
        })()''')
        assert state['viewport'] == [1441, 1025] and state['dpr'] == 1
        state['expectedTextNodes'] = len(expected)
        state['missingText'] = missing_text(expected, [e['text'] for e in state['textElements']])
        state['matchedTextNodes'] = len(expected) - len(state['missingText'])
        state['commonAssetMismatches'] = [str(p.relative_to(ROOT / 'common')) for p in (ROOT / 'common').rglob('*')
                                         if p.is_file() and p.read_bytes() != (directory / 'project' / p.relative_to(ROOT / 'common')).read_bytes()]
        save_json(directory / 'browser-state.json', state)
        subprocess.run(['agent-browser', '--session', SESSION, 'screenshot', str(directory / 'browser.png')], check=True, timeout=30)
        result = browser('''(async () => {
          const urls = ['/ground-truth.png', '/runs/ARM/browser.png'];
          const pixels = [];
          for (const url of urls) {
            const img = new Image(); img.src = url; await img.decode();
            const c = document.createElement('canvas'); c.width=1024; c.height=729;
            const ctx=c.getContext('2d'); ctx.fillStyle='#fff'; ctx.fillRect(0,0,1024,729);
            ctx.drawImage(img,0,0,1024,729); pixels.push({canvas:c,data:ctx.getImageData(0,0,1024,729)});
          }
          const a=pixels[0].data.data, b=pixels[1].data.data;
          let sum=0,changed=0;
          const diff=document.createElement('canvas'); diff.width=1024; diff.height=729;
          const dx=diff.getContext('2d'), id=dx.createImageData(1024,729);
          for(let i=0;i<a.length;i+=4){
            let max=0;
            for(let c=0;c<3;c++){const delta=Math.abs(a[i+c]-b[i+c]);sum+=delta;max=Math.max(max,delta);id.data[i+c]=Math.min(255,delta*3);}
            if(max>48)changed++; id.data[i+3]=255;
          }
          dx.putImageData(id,0,0);
          const overlay=document.createElement('canvas'); overlay.width=1024;overlay.height=729;
          const ox=overlay.getContext('2d');ox.drawImage(pixels[0].canvas,0,0);ox.globalAlpha=.5;ox.drawImage(pixels[1].canvas,0,0);
          return {mae255:sum/(1024*729*3),fractionMaxChannelOver48:changed/(1024*729),
            normalized:pixels[1].canvas.toDataURL('image/png').split(',')[1],
            difference:diff.toDataURL('image/png').split(',')[1],overlay:overlay.toDataURL('image/png').split(',')[1]};
        })()'''.replace('ARM', arm))
        for name in ['normalized', 'difference', 'overlay']:
            (directory / (name + '.png')).write_bytes(base64.b64decode(result.pop(name)))
        result.update({'matchedTextNodes':state['matchedTextNodes'], 'expectedTextNodes':len(expected)})
        metrics[arm] = result
        save_json(directory / 'pixels.json', result)
    save_json(ROOT / 'pixel-summary.json', metrics)
    audit = {}
    for arm in metrics:
        directory = ROOT / 'runs' / arm
        events = [json.loads(line) for line in (directory / 'events.jsonl').read_text().splitlines() if line.strip()]
        messages = [e['message'] for e in events if e['type'] == 'message_end' and e['message'].get('role') == 'assistant']
        ends = [e for e in events if e['type'] == 'tool_execution_end']
        starts = [e for e in events if e['type'] == 'tool_execution_start']
        design_ends = [e for e in ends if e['toolName'].startswith('figma_')]
        screenshots = [e for e in design_ends if e['toolName'] == 'figma_get_screenshot']
        usage = {key:sum(m.get('usage', {}).get(key, 0) for m in messages)
                 for key in ['input', 'output', 'cacheRead', 'cacheWrite', 'reasoning', 'totalTokens']}
        audit[arm] = {
            **json.loads((directory / 'result.json').read_text()),
            'usage':usage, 'assistantTurns':len(messages), 'toolCalls':len(ends),
            'toolErrors':sum(bool(e.get('isError')) for e in ends),
            'officialDesignCalls':len(design_ends),
            'officialDesignDurationSumSeconds':sum(e.get('durationMs', 0) for e in design_ends)/1000,
            'screenshotCallsDeliveringModelImageBlocks':sum(any(c.get('type')=='image' for c in e.get('result', {}).get('content', [])) for e in screenshots),
            'toolCallsForManualAudit':[{'name':e['toolName'],'args':e.get('args')} for e in starts],
            'finalText':'\n'.join(c.get('text','') for c in messages[-1].get('content',[]) if c.get('type')=='text') if messages else '',
        }
    save_json(ROOT / 'run-summary.json', audit)
    subprocess.run(['agent-browser', '--session', SESSION, 'close'], capture_output=True, timeout=20)
    print(json.dumps(metrics, indent=2))


def semantic():
    # Preserve v1/v2 results; keep pseudo text diagnostic-only, separate from DOM coverage.
    subprocess.run(['agent-browser', '--session', SESSION, 'open', 'http://127.0.0.1:8876/'], check=True, timeout=30)
    observed = browser('''(async () => {
      const out={};
      for(const arm of ['official','figlens','screenshot']){
        const iframe=document.createElement('iframe'); iframe.width=1441;iframe.height=1025;
        const loaded=new Promise(resolve=>iframe.onload=resolve);
        iframe.src='/runs/'+arm+'/project/index.html';document.body.append(iframe);await loaded;
        const d=iframe.contentDocument,w=iframe.contentWindow;await d.fonts.ready;
        const pseudo=Array.from(d.querySelectorAll('body *')).flatMap(e=>['::before','::after'].map(p=>w.getComputedStyle(e,p).content)).filter(t=>t && t!=='none' && t!=='normal' && t!=='""');
        const elements=Array.from(d.querySelectorAll('body *')).filter(e=>Array.from(e.childNodes).some(n=>n.nodeType===3&&n.textContent.trim())).map(e=>e.innerText);
        out[arm]={textContent:d.body.textContent,innerText:d.body.innerText,pseudoContent:pseudo,textElements:elements};iframe.remove();
      }
      return out;
    })()''')
    expected = []
    def walk(node):
        if node.get('visible') is False:
            return
        if node.get('type') == 'TEXT' and node.get('characters'):
            expected.append(node['characters'])
        for child in node.get('children', []):
            walk(child)
    walk(json.loads((ROOT / 'figlens/design/nodes.json').read_text())['document'])
    for arm, state in observed.items():
        state['missingText'] = missing_text(expected, state['textElements'])
        state['matchedTextNodes'] = len(expected) - len(state['missingText'])
        state['expectedTextNodes'] = len(expected)
    save_json(ROOT / 'text-check-v3.json', observed)
    subprocess.run(['agent-browser', '--session', SESSION, 'close'], capture_output=True, timeout=20)
    print(json.dumps({k:{key:v[key] for key in ['matchedTextNodes','expectedTextNodes','missingText','pseudoContent']} for k,v in observed.items()},indent=2))


if __name__ == '__main__':
    assert len(sys.argv) == 3 and sys.argv[1] in ['prepare', 'run', 'evaluate', 'semantic']
    assert ROOT.is_dir()
    {'prepare': prepare, 'run': run, 'evaluate': evaluate, 'semantic': semantic}[sys.argv[1]]()
