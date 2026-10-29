const {test} = require('node:test');
const assert = require('node:assert/strict');
const {flat, clean, localBox, hits, prototypeRows, relQuery, acceptReply, reasonCodes, rowModel, groupUsages, localSearch} = require('../src/preview/viewer.js');

test('prototype missing fields remain unspecified without click or navigation defaults', () => {
  const rows = prototypeRows({state:'recorded', sourceNodeId:'0:1', interactions:[
    {sourceField:'prototypeInteractions[0]', trigger:{state:'absent'}, actionsState:'absent', actions:[]},
  ]});
  assert.equal(rows[0].label, 'Trigger not specified → Action not specified');
  assert.equal(rows[0].destination, 'Node destination not specified');
  assert.equal(rows[0].target, null);
  assert.equal(rows.some(r => /on click|navigate/i.test(r.label)), false);
});

test('prototype rows retain both action fields and do not link unresolved destinations', () => {
  const rows = prototypeRows({state:'recorded',sourceNodeId:'0:1',truncated:true, interactions:[
    {trigger:{state:'recorded',value:'ON_HOVER'},actionsState:'recorded',actions:[
      {sourceField:'prototypeInteractions[0].actions[0]',navigationType:{state:'recorded',value:'NAVIGATE'},connectionType:{state:'recorded',value:'OPEN_OVERLAY'},destination:{state:'unresolved',nodeId:'0:999'}},
      {sourceField:'prototypeInteractions[0].actions[1]',navigationType:{state:'absent'},connectionType:{state:'recorded',value:'BACK'},destination:{state:'resolved',nodeId:'0:2',name:'</script>'}},
    ]},
  ]});
  assert.match(rows[0].label, /On hover.*Navigation: Navigate.*Connection: Open overlay/);
  assert.equal(rows[0].destination,'0:999 (unresolved)');
  assert.equal(rows[0].target,null);
  assert.equal(rows[1].target,'0:2');
  assert.equal(rows[1].destination,'</script>');
  assert.match(rows[1].source,/0:1.*prototypeInteractions\[0\].actions\[1\]/);
  assert.equal(rows.at(-1).label,'Prototype evidence truncated');
});

test('prototype unsupported values and explicit empty actions keep their own meanings', () => {
  const rows = prototypeRows({state:'recorded',sourceNodeId:'0:1',interactions:[
    {trigger:{state:'unsupported',value:'ON_CLICK'},actionsState:'unsupported',actions:[]},
    {trigger:{state:'absent'},actionsState:'recorded',actions:[]},
  ]});
  assert.equal(rows[0].label,'Trigger unsupported → Action unsupported');
  assert.equal(rows[1].label,'Trigger not specified → No actions recorded');
});

test('empty and unsupported prototype evidence has a renderable status row', () => {
  for (const evidence of [
    {state:'recorded', interactions:[]},
    {state:'recorded', interactions:[], deletedInteractionCount:1},
    {state:'unsupported', reason:'malformed_interaction_list'},
  ]) {
    const rows = prototypeRows(evidence);
    assert.equal(rows.length, 1);
    assert.equal(rows[0].destination, 'No destination to display');
    assert.equal(rows[0].target, null);
  }
});

test('prototype omitted action lists stay unknown, including older truncated envelopes', () => {
  for (const actionsTruncated of [true, undefined]) {
    const rows = prototypeRows({state:'recorded', sourceNodeId:'0:1', truncated:true, interactions:[
      {trigger:{state:'recorded',value:'ON_HOVER'},actionsState:'recorded',actions:[],actionsTruncated},
      {trigger:{state:'absent'},actionsState:'recorded',actions:[],actionsTruncated:false},
      {trigger:{state:'absent'},actionsState:'unsupported',actions:[]},
    ]});
    assert.equal(rows[0].label, 'On hover → Actions omitted by evidence limits');
    assert.equal(rows[0].destination, 'Node destination not extracted');
    assert.equal(rows[0].target, null);
    assert.equal(rows[1].label, 'Trigger not specified → No actions recorded');
    assert.equal(rows[1].destination, 'Node destination not specified');
    assert.equal(rows[2].label, 'Trigger not specified → Action unsupported');
  }
});

test('export-area origin includes section margins, negative coordinates and shadows', () => {
  const n = {id:'I1:2;3:4',absoluteBoundingBox:{x:-60,y:20,width:10,height:20},absoluteRenderBounds:{x:-65,y:15,width:20,height:30}};
  assert.deepEqual(localBox(n,[-100,-25,0,100]),{x:35,y:40,width:20,height:30});
  assert.deepEqual(hits([n],[-100,-25,0,100],36,41).map(n=>n.id),['I1:2;3:4']);
  assert.deepEqual(hits([n],[-100,-25,0,100],34,41),[]);
});
test('overlap picker keeps exact expanded ids and excludes hidden / fully clipped nodes', () => {
  const box={x:0,y:0,width:20,height:20};
  const nodes=[{id:'1:2',absoluteBoundingBox:box},{id:'I1:2;3:4',absoluteBoundingBox:box},{id:'hidden',visible:false,absoluteBoundingBox:box},{id:'clipped',absoluteBoundingBox:box,absoluteRenderBounds:null}];
  assert.deepEqual(hits(nodes,[0,0,20,20],10,10).map(n=>n.id),['I1:2;3:4','1:2']);
});
test('context and traversal preserve child ids and untrusted text as data', () => {
  const node={id:'I1:2;3:4',name:'</script><img onerror=alert(1)>',children:[{id:'I1:2;5:6'}]};
  assert.deepEqual(flat(node).map(n=>n.id),['I1:2;3:4','I1:2;5:6']);
  assert.equal(clean(node).name,node.name);
  assert.equal(clean(node).children,undefined);
});
test('picking follows rotated and elliptical geometry, and groups only through their children', () => {
  // A 10x10 square rotated 45° about its top-left corner at (10, 0): its box is (2.93..17.07, 0..14.14).
  const r = Math.SQRT1_2;
  const rotated = {id:'rot',type:'RECTANGLE',absoluteBoundingBox:{x:10-10*r,y:0,width:20*r,height:20*r}};
  const ellipse = {id:'oval',type:'ELLIPSE',absoluteBoundingBox:{x:0,y:20,width:10,height:10}};
  const group = {id:'group',type:'GROUP',absoluteBoundingBox:{x:0,y:0,width:30,height:30}};
  const geometry = {rot:[r,r,-r,r,10,0,10,10,0], oval:[1,0,0,1,0,20,10,10,1]};
  const at = (x, y) => hits([group,rotated,ellipse],[0,0,30,30],x,y,geometry).map(n=>n.id);
  assert.deepEqual(at(10,7),['rot']);       // centre of the diamond
  assert.deepEqual(at(4,1),[]);             // inside its box, outside the diamond
  assert.deepEqual(at(5,25),['oval']);
  assert.deepEqual(at(0.5,20.5),[]);        // box corner outside the ellipse
  assert.deepEqual(hits([group],[0,0,30,30],15,15),[]); // a group alone is never picked
});
test('thin layers keep a pointer-sized margin so lines stay pickable', () => {
  const line = {id:'line',type:'LINE',absoluteBoundingBox:{x:10,y:10,width:20,height:0}};
  const tilted = {id:'tilted',type:'LINE',absoluteBoundingBox:{x:0,y:0,width:10,height:10}};
  const geometry = {tilted:[Math.SQRT1_2,Math.SQRT1_2,-Math.SQRT1_2,Math.SQRT1_2,0,0,Math.SQRT2*10,0,0]};
  assert.deepEqual(hits([line],[0,0,40,40],20,11,{},2).map(n=>n.id),['line']);
  assert.deepEqual(hits([line],[0,0,40,40],20,15,{},2),[]);
  assert.deepEqual(hits([tilted],[0,0,40,40],5,5.5,geometry,2).map(n=>n.id),['tilted']);
  assert.deepEqual(hits([tilted],[0,0,40,40],8,2,geometry,2),[]);
});
test('vector outlines decide picks inside their box', () => {
  const icon = {id:'icon',type:'VECTOR',absoluteBoundingBox:{x:0,y:0,width:10,height:10}};
  const triangle = (x, y) => y >= x; // lower-left half of the box, in page coordinates
  assert.deepEqual(hits([icon],[0,0,10,10],2,8,{},0,{icon:triangle}).map(n=>n.id),['icon']);
  assert.deepEqual(hits([icon],[0,0,10,10],8,2,{},0,{icon:triangle}),[]);
});

test('relation queries repeat keys, encode plus and spaces, and drop empty values', () => {
  const q = new URLSearchParams(relQuery({q:'a+b c', type:['FRAME','TEXT'], hidden:false, internal:true, scope:null, cursor:''}));
  assert.equal(q.get('q'), 'a+b c');
  assert.deepEqual(q.getAll('type'), ['FRAME','TEXT']);
  assert.equal(q.get('hidden'), '0');
  assert.equal(q.get('internal'), '1');
  assert.equal(q.has('scope') || q.has('cursor'), false);
  assert.match(relQuery({q:'a+b c'}), /q=a%2Bb\+c/);
});
test('a reply counts only for the newest request on the loaded snapshot', () => {
  assert.equal(acceptReply(3, 3, 's1', 's1'), true);
  assert.equal(acceptReply(2, 3, 's1', 's1'), false);
  assert.equal(acceptReply(3, 3, 's2', 's1'), false);
  assert.equal(acceptReply(3, 3, undefined, undefined), false);
});
test('coverage reason codes stay verbatim and are not repeated', () => {
  const r = {code:'relation_unavailable', relations:['library_identity']};
  assert.deepEqual(reasonCodes({coverage:{reasons:[{code:'scan_limit'}, r], layers:{relations:{reasons:[r]}}}}),
    ['scan_limit', 'relation_unavailable (library_identity)']);
  assert.deepEqual(reasonCodes({}), []);
});
test('usages rows keep identity on ref.nodeId and show unknown targets as rows', () => {
  const ref = {sourceId:'f', snapshotId:'s1', nodeId:'I1:2;3:4'};
  const m = rowModel({target:{resource:'VariableID:x/1'}, relation:'binds_variable', state:'external'}, 's1');
  assert.equal(m.nodeId, null);
  assert.equal(m.resource, 'VariableID:x/1');
  assert.equal(m.state, 'external');
  assert.equal(rowModel({ref, name:'Same name'}).nodeId, 'I1:2;3:4');
  assert.equal(rowModel({target:null, relation:'instance_of', state:'missing'}).name, '(no target)');
});
test('usages group members apart from results, and results by relation', () => {
  const g = groupUsages({source:{snapshotId:'s1'}, members:{nodeIds:['1:2','1:3'], complete:false}, results:[
    {ref:{nodeId:'5:1', snapshotId:'s1'}, name:'A', relation:'instance_of'},
    {ref:{nodeId:'5:2', snapshotId:'s1'}, name:'B', relation:'contains'},
    {ref:{nodeId:'5:3', snapshotId:'s1'}, name:'C', relation:'instance_of', via:'1:2'},
  ]});
  assert.deepEqual(g.members, {ids:['1:2','1:3'], complete:false});
  assert.deepEqual(g.groups.map(([k, rows]) => [k, rows.map(r => r.nodeId)]), [['instance_of',['5:1','5:3']],['contains',['5:2']]]);
  assert.equal(groupUsages({results:[]}).members, null);
});
test('offline search covers the packaged layers only and hides hidden layers by default', () => {
  const list = [{id:'0:1',name:'Page',type:'FRAME',children:[{id:'1:1',name:'Button',type:'FRAME'},{id:'1:2',name:'x',type:'TEXT',characters:'button text'},
    {id:'1:3',name:'Button 2',type:'FRAME',visible:false,children:[{id:'1:4',name:'Inner button',type:'TEXT'}]}]}];
  assert.deepEqual(localSearch(list, 'BUTTON').results.map(r => r.ref.nodeId), ['1:1','1:2']);
  assert.deepEqual(localSearch(list, 'button', {types:['TEXT']}).results.map(r => r.ref.nodeId), ['1:2']);
  assert.deepEqual(localSearch(list, 'button', {hidden:true}).results.map(r => [r.ref.nodeId, r.hidden]), [['1:1',false],['1:2',false],['1:3',true],['1:4',true]]);
  assert.deepEqual(localSearch(list, '').results, []);
});
