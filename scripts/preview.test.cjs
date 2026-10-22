const {test} = require('node:test');
const assert = require('node:assert/strict');
const {flat, clean, localBox, hits} = require('../src/preview/viewer.js');

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
