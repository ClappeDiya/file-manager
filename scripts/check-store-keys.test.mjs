import {test} from 'node:test';
import assert from 'node:assert/strict';
import {inspectStore,checkStores} from './check-store-keys.mjs';
test('resolves a constant key without confusing nested state fields or text',()=>{
 const source='const KEY="ufop-files"; const x=persist(() => ({name:"unrelated", note:`name: "fake"`, matcher:/[{}]/}), {name:KEY});';
 assert.equal(inspectStore(source)[0].name,'ufop-files');assert.deepEqual(checkStores(inspectStore(source)),[]);
});
test('missing, dynamic, unnamespaced and duplicate keys fail',()=>{
 for(const source of ['persist(()=>({name:"fake"}),{})','persist(()=>({}),{name:runtime()})','persist(()=>({}),{name:"files"})'])assert.equal(checkStores(inspectStore(source)).length,1);
 assert.equal(checkStores([...inspectStore('persist(()=>({}),{name:"ufop-dupe"})','a.ts'),...inspectStore('persist(()=>({}),{name:"ufop-dupe"})','b.ts')]).length,1);
});
test('comments and quoted persist examples are not actual stores',()=>{
 assert.deepEqual(inspectStore('/* persist(()=>({}),{}) */ const example="persist()"; // persist()'),[]);
});
