#!/usr/bin/env python3
"""One owned fixture, twelve bounded attempts; private experiment, not a benchmark framework."""
import base64
import hashlib
import importlib.util
import json
from pathlib import Path
import random
import shutil
import struct
import subprocess
import sys
import xml.etree.ElementTree as ET

CHECKLIST = '''Before finishing, check your implementation in the browser against the supplied design data:
- Compare the outer dimensions and origin, each important container's size and position, and every corner radius; do not round values by preference.
- Check stroke width and alignment (inside, center, outside), including the effect on content and export bounds; CSS borders must not silently shift the layout.
- Check every visible text string, font family, weight, size, line-height, color, and placement using loaded fonts and real DOM text.
- Inspect each supplied icon at its intended slot and rendered dimensions. Inspect the browser screenshot for remaining layout or clipping differences and correct in-scope discrepancies within this attempt.
'''


def make_order():
    order = [f'{arm}-{i}' for arm in ['screenshot', 'official', 'figlens', 'checklist'] for i in range(1, 4)]
    random.Random(20261027).shuffle(order)
    return order


def sha(file):
    return hashlib.sha256(file.read_bytes()).hexdigest()


def walk(node):
    yield node
    for child in node.get('children', []):
        yield from walk(child)


def png_size(file):
    data = file.read_bytes()
    assert data[:8] == b'\x89PNG\r\n\x1a\n'
    return list(struct.unpack('>II', data[16:24]))


def prepare(root, base):
    assert not (root / 'runs').exists(), 'Do not overwrite or reroll attempts'
    nodes = json.loads((root / 'figlens/design/nodes.json').read_text())['document']
    all_nodes = list(walk(nodes))
    metadata = json.loads((root / 'official/before-reference/get_metadata.json').read_text())
    xml = next(b['text'] for b in metadata['content'] if b.get('text', '').startswith('<frame'))
    live = {n.attrib['id']: n.attrib for n in ET.fromstring(xml).iter()}
    assert set(live) == {n['id'] for n in all_nodes}
    for node in all_nodes:
        other = live[node['id']]
        assert node['name'] == other['name']
        for key in ['x', 'y', 'width', 'height']:
            assert node['absoluteBoundingBox'][key] == float(other[key]), (node['id'], key)
    assert nodes['absoluteBoundingBox'] == {'x': 0.0, 'y': 0.0, 'width': 960.0, 'height': 640.0}
    assert nodes['absoluteRenderBounds'] == {'x': -1.0, 'y': -1.0, 'width': 962.0, 'height': 642.0}
    reference = root / 'official/before-reference/get_screenshot-0.png'
    assert png_size(reference) == [962, 642], 'No resampling or fallback reference'
    assert json.loads((root / 'figlens/missing-assets.json').read_text()) == []
    for entry in json.loads((root / 'figlens/manifest.json').read_text())['files']:
        file = root / 'figlens' / entry['path']
        assert file.stat().st_size == entry['size'] and sha(file) == entry['sha256']
    text_nodes = [n for n in all_nodes if n['type'] == 'TEXT']
    context = json.loads((root / 'official/before-reference/get_design_context.json').read_text())
    code = '\n'.join(b.get('text', '') for b in context['content'])
    for node in text_nodes:
        assert node['style']['fontFamily'] == 'Source Code Pro' and node['style']['fontWeight'] == 400
        assert node['characters'] in code
    assert "Source_Code_Pro:Regular" in code
    font = root / 'fonts/SourceCodePro-variable.ttf'
    assert sha(font) == 'b400fc584e10aff25d0e775ce181b4fc1c5ea1b5dc37b81aeb2084375b945790'
    shutil.copyfile(reference, root / 'ground-truth.png')
    common = root / 'common'
    shutil.copytree(root / 'fonts', common / 'fonts')
    (common / 'icons').mkdir()
    (root / 'font-probe.html').write_text('<style>@font-face{font-family:"Source Code Pro";src:url("fonts/SourceCodePro-variable.ttf");font-weight:100 900}</style>font probe')
    subprocess.run(['agent-browser', '--session', base.SESSION, 'open', 'http://127.0.0.1:8877/font-probe.html'], check=True, timeout=30)
    font_probe = base.browser('''(async()=>{
      await document.fonts.load('400 16px "Source Code Pro"'); await document.fonts.ready;
      const c=document.createElement('canvas').getContext('2d');
      return {loaded:document.fonts.check('400 16px "Source Code Pro"'),widths:TEXT.map(n=>{
        c.font='400 '+n.style.fontSize+'px "Source Code Pro"';return {id:n.id,width:c.measureText(n.characters).width};
      })};
    })()'''.replace('TEXT', json.dumps(text_nodes)))
    assert font_probe['loaded']
    # Advance widths and Figma text boxes are different metrics; verify glyph shapes instead.
    font_probe['shapeControl'] = base.browser('''(async()=>{
      await document.fonts.load('400 16px "Source Code Pro"');
      const img=new Image();img.src='/ground-truth.png';await img.decode();
      const ref=document.createElement('canvas');ref.width=962;ref.height=642;ref.getContext('2d').drawImage(img,0,0);
      const errors=[0,0],details=[];let count=0;
      for(const n of TEXT){
        const b=n.absoluteBoundingBox,w=b.width+4,h=b.height;
        const rgba=ref.getContext('2d').getImageData(b.x-1,b.y+1,w,h).data;
        const fg=['r','g','b'].map(k=>n.fills[0].color[k]*255),bg=Array.from(rgba.slice(0,3));
        let channel=0;for(let c=1;c<3;c++)if(Math.abs(fg[c]-bg[c])>Math.abs(fg[channel]-bg[channel]))channel=c;
        const target=[];for(let p=0;p<rgba.length;p+=4)target.push(Math.max(0,Math.min(1,(rgba[p+channel]-bg[channel])/(fg[channel]-bg[channel]))));
        const canvas=document.createElement('canvas');canvas.width=w;canvas.height=h;
        const ctx=canvas.getContext('2d',{willReadFrequently:true}),best=[];
        for(const [f,family] of ['"Source Code Pro"','monospace'].entries()){
          let min=Infinity,baseline=null;
          for(let y=0;y<=h;y+=.5){
            ctx.fillStyle='white';ctx.fillRect(0,0,w,h);ctx.fillStyle='black';ctx.font='400 '+n.style.fontSize+'px '+family;ctx.fillText(n.characters,2,y);
            const data=ctx.getImageData(0,0,w,h).data;let error=0;
            for(let p=0;p<target.length;p++)error+=Math.abs(target[p]-(255-data[p*4])/255);
            if(error<min){min=error;baseline=y;}
          }
          errors[f]+=min;best.push({error:min/(w*h),baseline});
        }
        count+=w*h;details.push({id:n.id,sourceCodePro:best[0],negativeMonospace:best[1]});
      }
      return {sourceCodeProError:errors[0]/count,negativeMonospaceError:errors[1]/count,details};
    })()'''.replace('TEXT', json.dumps(text_nodes)))
    base.save_json(root / 'font-probe.json', font_probe)
    shape = font_probe['shapeControl']
    assert shape['sourceCodeProError'] < shape['negativeMonospaceError'] * .75, 'Matching font must beat the known-wrong font glyph control by 25%'

    # Use just two shared leaf icons; anonymous filenames reveal no absolute geometry or node IDs.
    assets = json.loads((root / 'figlens/assets.json').read_text())
    selected = [next(a['path'] for a in assets if a['nodeId'] == id) for id in ['2:12', '2:18']]
    rasters = base.browser('''(async()=>{const out=[];for(const url of URLS){
      const i=new Image();i.src=url;await i.decode();const c=document.createElement('canvas');
      c.width=i.width*2;c.height=i.height*2;c.getContext('2d').drawImage(i,0,0,c.width,c.height);
      out.push(c.toDataURL('image/png').split(',')[1]);}return out;})()'''.replace('URLS', json.dumps(['/figlens/' + p for p in selected])))
    for name, image in zip(['selected.png', 'unselected.png'], rasters):
        (common / 'icons' / name).write_bytes(base64.b64decode(image))
    css = '''@font-face{font-family:"Source Code Pro";src:url("fonts/SourceCodePro-variable.ttf");font-weight:100 900}
*{box-sizing:border-box}html,body{margin:0;width:962px;height:642px;overflow:hidden}
body{font-family:"Source Code Pro",monospace;font-synthesis:none;background:white}
#frame{position:relative;margin:1px;width:960px;height:640px}
'''
    intro = '''Implement the supplied design as a static, editable HTML/CSS screen in project/index.html and project/styles.css. Reproduce all visible text, layout, colors, borders, icons, and typography. Only this desktop view is required; no backend or responsive claim.
Work only inside this run directory. Do not inspect parent directories, other runs, repository files, global agent configuration, credentials, or other design sources. No network except your permitted design tool and local project URL. Do not install packages, modify Figma, or publish anything. Use real DOM text and layout, not a whole-frame image, SVG, canvas, or layer-tree-to-HTML converter.
Keep project/base.css, fonts, and common icons unchanged. Frame: Owned fixture / Task list, 960x640 inside a 962x642 canvas at (1,1); the outside stroke extends into that margin. Use the supplied local Source Code Pro variable font at weight 400; no fallback substitution. Use project/icons/selected.png and unselected.png for the task icons. For this controlled comparison these identical PNGs replace all tools' SVG icon assets; do not download or use other design assets.
Use read/write/edit/bash tools and agent-browser CLI with your isolated AGENT_BROWSER_SESSION. Inspect your local URL at viewport 962x642, DPR 1, await document.fonts.ready, and read the screenshot before finishing. Close your browser session when done. One attempt, 10 minutes total, no questions. Finish with changed files, actual checks, and known deviations.
'''
    design_inputs = {
        'screenshot': 'Your only design input is design/reference.png. Read it. No layout/node/style metadata is supplied.',
        'official': 'Your only design source is the official live Figma desktop MCP: node 2:2 in the currently open Untitled Design document. Read design/figma-guidance.md BEFORE calling figma_get_design_context. Request its screenshot and call figma_get_screenshot if none is returned. Adapt high-fidelity context to this existing static HTML/CSS project. Use metadata/variables/subnodes if needed; retain failures. Do not use figlens or other Figma endpoints. The supplied common PNG icon rule overrides the guidance asset rule for this experiment.',
        'figlens': 'Your only design source is the frozen figlens 0.7.3 bundle at design/bundle/. Read context.md, design/nodes.json, variables, and references/screen.png as needed. Its image is figlens rendering, NOT Figma reference. Use the common PNG icons, not bundle SVGs. Do not contact Figma or obtain an independent Figma screenshot.',
    }
    for name in make_order():
        arm = name.split('-')[0]
        directory = root / 'runs' / name
        project = directory / 'project'
        shutil.copytree(common, project)
        (project / 'base.css').write_text(css)
        (project / 'styles.css').write_text('/* Implement this screen here. */\n')
        (project / 'index.html').write_text('<!doctype html>\n<html lang="en"><meta charset="utf-8"><title>Owned fixture</title><link rel="stylesheet" href="base.css"><link rel="stylesheet" href="styles.css"><body><main id="frame"></main></body></html>\n')
        design = directory / 'design'
        design.mkdir()
        source_arm = 'figlens' if arm == 'checklist' else arm
        if arm == 'screenshot':
            shutil.copyfile(reference, design / 'reference.png')
        elif arm == 'official':
            shutil.copyfile(root / 'figma-guidance.md', design / 'figma-guidance.md')
        else:
            shutil.copytree(root / 'figlens', design / 'bundle')
        prompt = intro + f'Local URL: http://127.0.0.1:8877/runs/{name}/project/index.html\n\n' + design_inputs[source_arm] + '\n'
        if arm == 'checklist':
            prompt += '\n' + CHECKLIST
        (directory / 'prompt.md').write_text(prompt)
    base.save_json(root / 'run-order.json', {'seed': 20261027, 'order': make_order()})
    base.save_json(root / 'source-gates.json', {
        'passed': True, 'source': 'Untitled.fig', 'sourceSha256': sha(root / 'Untitled.fig'),
        'node': '2:2', 'matchedNodes': len(all_nodes), 'matchedTextNodes': len(text_nodes),
        'referencePixels': [962, 642], 'referenceSha256': sha(reference), 'resampling': False,
        'fontFamily': 'Source Code Pro', 'fontWeight': 400, 'fontSha256': sha(font),
        'fontLicense': 'SIL OFL 1.1, common/fonts/OFL.txt',
        'fontVerification': 'Family/weight match official context and the licensed binary loads. Browser glyph masks must match the independent Figma PNG at least 25% better than a known-wrong monospace control. Advance widths are diagnostic, not text-box widths. Figma internal font bytes are not exposed.',
        'missingAssets': 0, 'manifestHashesVerified': True,
        'scope': 'One deliberately simple synthetic fixed-layout fixture; no general design-quality claim.',
    })
    base.save_json(root / 'pilot-controls.json', {
        'model': 'openai-codex/gpt-6.1-sol', 'thinking': 'medium',
        'referencePixels': [962, 642], 'browserPixels': [962, 642], 'dpr': 1,
        'attemptsPerArm': 3, 'maxImplementations': 12, 'wallLimitSeconds': 600,
        'commonIcons': 2, 'iconSource': 'Frozen leaf SVGs rasterized at 2x for all arms.',
        'pixelComparison': 'Native pixels, no scaling/cropping; both images composited over white; channel MAE and max-channel>48 fraction.',
        'scope': 'Prepared inputs and warm tools, not cold-start end-to-end timing; generic checklist is the sole prompt treatment.',
    })
    print(json.dumps({'gates': 'passed', 'nodes': len(all_nodes), 'textNodes': len(text_nodes), 'order': make_order()}), flush=True)


def evaluate(root, base):
    expected = [n['characters'] for n in walk(json.loads((root / 'figlens/design/nodes.json').read_text())['document']) if n['type'] == 'TEXT']
    metrics = {}
    for name in make_order():
        directory = root / 'runs' / name
        subprocess.run(['agent-browser', '--session', base.SESSION, 'set', 'viewport', '962', '642'], check=True, timeout=20)
        subprocess.run(['agent-browser', '--session', base.SESSION, 'open', f'http://127.0.0.1:8877/runs/{name}/project/index.html'], check=True, timeout=30)
        state = base.browser('''(async()=>{await document.fonts.ready;await Promise.all(Array.from(document.images).map(i=>i.decode().catch(()=>null)));
          return {viewport:[innerWidth,innerHeight],dpr:devicePixelRatio,text:document.body.innerText,
            fonts:Array.from(document.fonts).map(f=>({family:f.family,status:f.status})),
            resources:performance.getEntriesByType('resource').map(r=>r.name),
            images:Array.from(document.images).map(i=>({src:i.getAttribute('src'),width:i.width,height:i.height,naturalWidth:i.naturalWidth})),
            canvasCount:document.querySelectorAll('canvas').length,svgCount:document.querySelectorAll('svg').length,
            textElements:Array.from(document.querySelectorAll('body *')).filter(e=>Array.from(e.childNodes).some(n=>n.nodeType===3&&n.textContent.trim())).map(e=>({tag:e.tagName,text:e.innerText,box:JSON.parse(JSON.stringify(e.getBoundingClientRect())),font:getComputedStyle(e).fontFamily,weight:getComputedStyle(e).fontWeight}))};})()''')
        assert state['viewport'] == [962, 642] and state['dpr'] == 1
        state['missingText'] = base.missing_text(expected, state['text'])
        state['matchedTextNodes'] = len(expected) - len(state['missingText'])
        state['commonAssetMismatches'] = [str(p.relative_to(root / 'common')) for p in (root / 'common').rglob('*') if p.is_file() and (not (directory / 'project' / p.relative_to(root / 'common')).exists() or sha(p) != sha(directory / 'project' / p.relative_to(root / 'common'))) ]
        base.save_json(directory / 'browser-state.json', state)
        subprocess.run(['agent-browser', '--session', base.SESSION, 'screenshot', str(directory / 'browser.png')], check=True, timeout=30)
        for command in ['errors', 'console']:
            log = subprocess.run(['agent-browser', '--session', base.SESSION, command], capture_output=True, text=True, timeout=20)
            (directory / f'browser-{command}.log').write_text(log.stdout + log.stderr)
        result = base.browser('''(async()=>{
          const images=[];for(const url of ['/ground-truth.png','/runs/NAME/browser.png']){
            const i=new Image();i.src=url;await i.decode();if(i.width!==962||i.height!==642)throw Error('Unequal native bounds');
            const c=document.createElement('canvas');c.width=962;c.height=642;const x=c.getContext('2d');x.fillStyle='#fff';x.fillRect(0,0,962,642);x.drawImage(i,0,0);images.push(c);}
          const a=images[0].getContext('2d').getImageData(0,0,962,642).data,b=images[1].getContext('2d').getImageData(0,0,962,642).data;
          const diff=document.createElement('canvas');diff.width=962;diff.height=642;const dx=diff.getContext('2d'),id=dx.createImageData(962,642);let sum=0,changed=0;
          for(let i=0;i<a.length;i+=4){let max=0;for(let c=0;c<3;c++){const d=Math.abs(a[i+c]-b[i+c]);sum+=d;max=Math.max(max,d);id.data[i+c]=Math.min(255,d*3);}if(max>48)changed++;id.data[i+3]=255;}dx.putImageData(id,0,0);
          const overlay=document.createElement('canvas');overlay.width=962;overlay.height=642;const ox=overlay.getContext('2d');ox.drawImage(images[0],0,0);ox.globalAlpha=.5;ox.drawImage(images[1],0,0);
          return {mae255:sum/(962*642*3),fractionMaxChannelOver48:changed/(962*642),overlay:overlay.toDataURL('image/png').split(',')[1],difference:diff.toDataURL('image/png').split(',')[1]};})()'''.replace('NAME', name))
        for key in ['overlay', 'difference']:
            (directory / f'{key}.png').write_bytes(base64.b64decode(result.pop(key)))
        result.update({'matchedTextNodes':state['matchedTextNodes'], 'expectedTextNodes':len(expected)})
        metrics[name] = result
        base.save_json(directory / 'pixels.json', result)
    base.save_json(root / 'pixel-summary.json', metrics)
    print(json.dumps(metrics, indent=2), flush=True)


def main():
    assert len(sys.argv) == 3 and sys.argv[1] == 'execute'
    root = Path(sys.argv[2]).resolve()
    spec = importlib.util.spec_from_file_location('original_pilot', Path(__file__).with_name('e2e-pilot.py'))
    base = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(base)
    base.SESSION = 'figlens-owned-preflight'
    with (root / 'http.log').open('x') as log:
        server = subprocess.Popen([sys.executable, '-m', 'http.server', '8877', '--bind', '127.0.0.1', '--directory', str(root)], stdout=log, stderr=log)
        try:
            # Executable readiness check, not a fixed sleep.
            import urllib.request
            import time
            for _ in range(50):
                if server.poll() is not None:
                    raise RuntimeError('Private HTTP server failed to start')
                try:
                    with urllib.request.urlopen('http://127.0.0.1:8877/provenance.json', timeout=1) as response:
                        assert response.status == 200
                    break
                except OSError:
                    time.sleep(.1)
            else:
                raise RuntimeError('Private HTTP server not ready')
            prepare(root, base)
            subprocess.run(['agent-browser', '--session', base.SESSION, 'close'], capture_output=True, timeout=20)
            base.run()
            evaluate(root, base)
        finally:
            subprocess.run(['agent-browser', '--session', base.SESSION, 'close'], capture_output=True, timeout=20)
            server.terminate()
            server.wait(timeout=10)


if __name__ == '__main__':
    main()
