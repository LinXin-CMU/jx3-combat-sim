// node --test tools/harness-run-ui-test.js — no model calls, server, browser or userdata.
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const shell = require('../frontend/assistant-shell.js');
const workspace = require('../frontend/harness-workspace.js');
const run = require('../frontend/harness-run.js');
const exact = require('../frontend/macro-exact.js');
const copy = value => JSON.parse(JSON.stringify(value));
const settle = async () => { for (let i = 0; i < 4; i++) await new Promise(resolve => setImmediate(resolve)); };

test('geometry stays usable on narrow screens and all resize directions', () => {
  for (const viewport of [{width:1440,height:900},{width:375,height:667},{width:240,height:320}]) {
    const fitted = shell.fit({x:-500,y:10000,width:800,height:1500},viewport);
    assert.ok(fitted.x >= 8 && fitted.x + fitted.width <= viewport.width - 8);
    assert.ok(fitted.y >= 8 && fitted.y + fitted.height <= viewport.height - 8);
    const shrunk = shell.resize(fitted,900,900,'left',viewport);
    assert.ok(shrunk.width >= Math.min(360,viewport.width-16));
    const ball = shell.fitBall({x:10000,y:-3},viewport);
    assert.ok(ball.x+56 <= viewport.width-8); assert.equal(ball.y,8);
  }
});
test('provider default prefers available DeepSeek Flash and respects explicit available choice', () => {
  const profiles = [{id:'ds-pro',model:'deepseek-v4-pro',available:true},{id:'offline',available:true},{id:'ds-flash',model:'deepseek-v4-flash',available:true},{id:'missing',available:false}];
  assert.equal(run.chooseProvider(profiles),'ds-flash'); assert.equal(run.chooseProvider(profiles,'ds-pro'),'ds-pro');
  assert.equal(run.chooseProvider(profiles,'missing'),'ds-flash'); assert.equal(run.chooseProvider([]),'');
});
test('run identity and sequence prevent old SSE results replacing current evidence', () => {
  assert.equal(run.acceptSnapshot({sequence:8},{run_id:'a',sequence:7,status:'running'},'a'),false);
  assert.equal(run.acceptSnapshot({sequence:8},{run_id:'b',sequence:9,status:'running'},'a'),false);
  assert.equal(run.acceptSnapshot({sequence:8},{run_id:'a',sequence:9,status:'running'},'a'),true);
  assert.equal(run.acceptSnapshot({sequence:8},{run_id:'a',accepted:true},'a'),false);
});
test('DPS improvement requires a comparable replay window or identical equipment policy', () => {
  assert.equal(run.dpsComparable({kind:'compile_macro',result:{baseline:{dps:100},best:{metrics:{dps:200}},dps_comparable:false}}),false);
  assert.equal(run.dpsComparable({kind:'evaluate',result:{dps_comparable:true}}),true);
  assert.equal(run.dpsComparable({kind:'optimize_equipment',result:{baseline_policy:{hash:'same'},candidate_policy:{hash:'same'}}}),true);
  assert.equal(run.dpsComparable({kind:'optimize_equipment',result:{baseline_policy:{hash:'one'},candidate_policy:{hash:'two'}}}),false);
});
function fixtureScene() {
  return {version:'AnYingQianJi',mount:'FenShanJin',simulation:{sequence:['盾击','盾压'],haste_level:12,attributes:{base_attack:100,haste_level:12},target:{level:54},pre_releases:[{skill:'血怒',time_before:1}],pauses:[[3,1]],team_buffs:[{key:'x',release_times:[1,6]}],channel_ticks:{1:2},timing_offsets:{0:.125},qijin_buffs:{1:33},dunya_reset_seed:7,network_delay:80}, equipment:workspace.equipmentSnapshot({slots:{PRIMARY_WEAPON:{equip_id:33,strength:4,embedding:[1],enhance_id:2,enchant_id:3}},stoneId:4})};
}
test('capture clones complete manual/macro environments and all twelve equipment positions', async () => {
  const scene=fixtureScene(); let ready=0;
  const bridge=workspace.createBridge({ready:async()=>{ready++;},read:()=>scene});
  const captured=await bridge.capture(); scene.simulation.pre_releases[0].time_before=9;
  assert.equal(ready,1); assert.equal(captured.simulation.pre_releases[0].time_before,1);
  assert.equal(Object.keys(captured.equipment.slots).length,12); assert.equal(captured.equipment.slots.PRIMARY_WEAPON.strength,4);
  const hydrated=copy(captured.equipment);hydrated.slots.PRIMARY_WEAPON.name='display name';hydrated.slots.PRIMARY_WEAPON.quality=4;
  assert.deepEqual(workspace.equipmentSnapshot(hydrated),captured.equipment);
  scene.simulation.sequence=['__macro__'];scene.simulation.macro_text='/cast 盾击';scene.simulation.macro_duration=120;
  const macro=await bridge.capture();assert.equal(macro.simulation.macro_duration,120);assert.deepEqual(macro.simulation.pauses,[[3,1]]);
});
test('loop conversion preserves per-action state and separates pre-release indexing', () => {
  const simulation=fixtureScene().simulation;
  const cfg=workspace.toLoop({version:1,talents:{1:9},team_buffs:[{enabled:false,key:'saved'}]},simulation);
  assert.deepEqual(cfg.sequence[0],{type:'skill',skill:'盾击',offset:.125});
  assert.deepEqual(cfg.sequence[1],{type:'skill',skill:'盾压',channel_ticks:2,qijin_buff:33});
  assert.equal(cfg.sequence[2].type,'pre_release');assert.equal(cfg.talents[1],9);assert.equal(cfg.team_buffs[0].enabled,false);
  assert.deepEqual(workspace.macroConfig('#page shield\n/cast 盾击\n#page blade\n/cast 绝刀'),{mode:'stance',general:'',shield:'/cast 盾击',blade:'/cast 绝刀'});
  assert.deepEqual(workspace.macroConfig('#page 擎盾\n/cast 盾击\n#page 擎刀\n/cast 绝刀'),{mode:'stance',general:'',shield:'/cast 盾击',blade:'/cast 绝刀'});
  for (const macro of ['#page wall\n/cast 盾击','#page shield\n/cast 盾击\n#page shield\n/cast 盾压','#page general\n/cast 盾击','#page blade\n/cast 绝刀\n#page shield\n/cast 盾击']) assert.throws(()=>workspace.macroConfig(macro),/分页结构/);
});
test('workspace apply refuses stale edits before writing, rolls back failure, and restores undo point', async () => {
  let scene=fixtureScene(), writes=0, restores=0, fail=false;
  const bridge=workspace.createBridge({ready:async()=>{},read:()=>scene,write:async after=>{writes++;scene={...scene,...after};},restore:async before=>{restores++;scene=copy(before);},verify:async()=>{if(fail)throw new Error('verification failed');}});
  const before=await bridge.capture(), after=copy(before);after.simulation.sequence=['盾压'];
  scene.simulation.network_delay=90;
  await assert.rejects(bridge.apply(after,before.sourceKey,'tx1'),/已改变/);assert.equal(writes,0);
  scene=copy(before);fail=true;
  await assert.rejects(bridge.apply(after,before.sourceKey,'tx1'),/verification/);assert.equal(restores,1);assert.deepEqual(scene.simulation.sequence,['盾击','盾压']);
  fail=false;await bridge.apply(after,before.sourceKey,'tx2');assert.deepEqual(scene.simulation.sequence,['盾压']);
  scene.simulation.network_delay=90;await assert.rejects(bridge.undo('tx2'),/又有编辑/);
  scene.simulation.network_delay=80;const undone=await bridge.undo('tx2');assert.deepEqual(undone.simulation.sequence,['盾击','盾压']);assert.equal(bridge.undoPoint(),null);
});
test('async version change aborts apply; incomplete undo is reported and retains its checkpoint', async () => {
  let scene=fixtureScene(), switchIdentity=true;
  const bridge=workspace.createBridge({ready:async()=>{},read:()=>scene,write:async after=>{scene={...scene,...after};if(switchIdentity)scene.mount='TieGuYi';},restore:async before=>{scene=copy(before);},verify:async()=>{}});
  const original=await bridge.capture(),after=copy(original);after.simulation.sequence=['盾压'];
  await assert.rejects(bridge.apply(after,original.sourceKey,'wrong-mount'),/版本或心法/);assert.equal(bridge.undoPoint(),null);
  switchIdentity=false;await bridge.apply(after,original.sourceKey,'good');
  let broken=copy(after);
  const other=workspace.createBridge({ready:async()=>{},read:()=>broken,write:async value=>{broken=copy(value);},verify:async()=>{},restore:async()=>{broken.simulation.network_delay=999;}});
  const checkpoint=await other.capture(),candidate=copy(checkpoint);candidate.simulation.sequence=['盾击'];await other.apply(candidate,checkpoint.sourceKey,'undo-test');
  await assert.rejects(other.undo('undo-test'),/不一致/);assert.equal(other.undoPoint().transactionId,'undo-test');
});
test('serde empty/default fields do not create false workspace changes; real resource changes do', () => {
  const original=fixtureScene().simulation, serialized={...copy(original),macro_text:null,macro_duration:null,lite:false,lite_keep_timeline:false,formation:null};
  serialized.target.damage_cof=0;serialized.target.defense_bonus=0;
  assert.equal(workspace.equivalent(original,serialized),true);
  assert.equal(workspace.equivalent(null,[]),true);
  serialized.pre_releases[0].time_before=2;assert.equal(workspace.equivalent(original,serialized),false);
});
test('non-DOM attributes survive repeat simulation and expire on attribute or identity edits', () => {
  const store=workspace.createAttributeStore(),base={base_attack:100,haste_level:12},identity={version:'one',mount:'FenShanJin'};
  store.set({gen_gu:77,yuan_qi:66,base_magical_attack:123},base,identity);
  assert.deepEqual(store.read(base,identity),{...base,gen_gu:77,yuan_qi:66,base_magical_attack:123});
  assert.deepEqual(store.read({...base,base_attack:101},identity),{...base,base_attack:101});
  store.set({gen_gu:77},base,identity);assert.deepEqual(store.read(base,{...identity,version:'two'}),base);
  const before=fixtureScene(),after=copy(before);after.equipment.stone_id=9;
  assert.ok(workspace.diff(before,after).some(row=>row.field==='equipment.stone_id'));
});

class Element {
  constructor(tag='div',doc) {this.tagName=tag;this.doc=doc;this.children=[];this.dataset={};this.style={};this.attrs={};this.value='';this.hidden=false;this.disabled=false;this.listeners=new Map();this.scrollTop=0;this.scrollHeight=0;this.clientHeight=0;this.classes=new Set();this._text='';this.classList={add:(...v)=>v.forEach(x=>this.classes.add(x)),remove:(...v)=>v.forEach(x=>this.classes.delete(x)),contains:x=>this.classes.has(x),toggle:(x,on)=>on?this.classes.add(x):this.classes.delete(x)};}
  set id(value){this._id=value;this.doc?.elements.set(value,this);} get id(){return this._id;}
  set className(value){this.classes=new Set(value.split(' '));} get className(){return [...this.classes].join(' ');}
  set textContent(value){this._text=String(value);this.children=[];} get textContent(){return this._text+this.children.map(x=>x.textContent).join('');}
  get firstChild(){return this.children[0] || {textContent:this._text};} get firstElementChild(){return this.children[0];}
  set innerHTML(html){this.children=[];const stack=[this];for(const match of html.matchAll(/<\/?[a-z][^>]*>|[^<]+/gi)){const token=match[0];if(token.startsWith('</')){if(stack.length>1)stack.pop();continue;}if(!token.startsWith('<')){stack.at(-1)._text+=token;continue;}const tag=token.match(/^<([a-z0-9-]+)/i)[1];const el=new Element(tag,this.doc);for(const attr of token.matchAll(/([a-z][a-z0-9_-]*)(?:="([^"]*)")?/gi)){const [all,name,value]=attr;if(name===tag)continue;el.setAttribute(name,value??'');if(name==='value')el.value=value||'';if(name==='hidden')el.hidden=true;}stack.at(-1).append(el);if(!['input','br','hr','img','option'].includes(tag))stack.push(el);} }
  setAttribute(name,value){this.attrs[name]=String(value);if(name==='id')this.id=value;if(name==='class')this.className=value;if(name.startsWith('data-'))this.dataset[name.slice(5).replace(/-([a-z])/g,(_,c)=>c.toUpperCase())]=value;}
  getAttribute(name){return this.attrs[name];}
  append(...items){for(const el of items){el.parentElement=this;this.children.push(el);}} prepend(el){el.parentElement=this;this.children.unshift(el);} replaceChildren(...items){this._text='';this.children=[];this.append(...items);}
  matches(selector){if(selector.startsWith('#'))return this.id===selector.slice(1);if(selector.startsWith('.'))return this.classes.has(selector.slice(1));if(selector==='input:checked')return this.tagName==='input'&&this.checked;if(selector.startsWith('[')){const m=selector.match(/^\[([^=\]]+)(?:="([^"]*)")?\]$/);return !!m&&Object.hasOwn(this.attrs,m[1])&&(m[2]===undefined||this.attrs[m[1]]===m[2]);}return this.tagName===selector;}
  querySelectorAll(selector){const selectors=selector.split(',');return this.children.flatMap(el=>[...(selectors.some(s=>el.matches(s))?[el]:[]),...el.querySelectorAll(selector)]);}querySelector(s){return this.querySelectorAll(s)[0]||null;}closest(s){return s.split(',').some(x=>this.matches(x))?this:this.parentElement?.closest(s)||null;}
  addEventListener(type,action){this.listeners.set(type,[...(this.listeners.get(type)||[]),action]);}removeEventListener(type,action){this.listeners.set(type,(this.listeners.get(type)||[]).filter(x=>x!==action));}
  async emit(type,data={}){const event={target:this,button:0,preventDefault(){},...data};for(const fn of [...(this.listeners.get(type)||[])])await fn(event);}
  click(){if(!this.disabled)return this.emit('click');}focus(){this.doc.focused=this;}scrollIntoView(){}setPointerCapture(){}remove(){}showModal(){this.open=true;}close(){this.open=false;return this.emit('close');}
}
function domRoot(){
  const doc={elements:new Map(),getElementById(id){return this.elements.get(id)||null;},createElement(tag){return new Element(tag,this);}};doc.body=new Element('body',doc);doc.querySelector=s=>doc.body.querySelector(s);doc.querySelectorAll=s=>doc.body.querySelectorAll(s);doc.addEventListener=(type,fn)=>doc.body.addEventListener(type,fn);
  const handlers=new Map(),store=new Map();return {document:doc,innerWidth:1280,innerHeight:900,localStorage:{getItem:k=>store.get(k),setItem:(k,v)=>store.set(k,v)},CustomEvent:class{constructor(type,options){this.type=type;this.detail=options?.detail;}},addEventListener(type,fn){handlers.set(type,[...(handlers.get(type)||[]),fn]);},dispatchEvent(event){for(const fn of handlers.get(event.type)||[])fn(event);},AbortSignal,Event:class{},Jx3Nav:{switchPage(){}},_store:store};
}

test('exact panel sends a complete scene without a time budget and preserves preview across pause/resume', async()=>{
  const root=domRoot(),doc=root.document,panel=doc.createElement('section');panel.id='assistant_exact_panel';doc.body.append(panel);
  const calls=[],timers=new Map();let timerId=0,submitted;
  let job={id:'exact-test',done:false,status:'running',phase:'solving',version:'CangShengZhuShiTest',mount:'FenShanJin',horizon:30,progress:[],best:{macro:'/cast [bufftime:盾飞<12.5] 绝刀',comparison:{reproduced:false,target_count:23,actual_count:25,order_prefix:6,exact_prefix:6,state_prefix:6,max_time_error_on_order_prefix:0,first_difference:{index:6,expected:{name:'斩刀',time:6},actual:{name:'血怒',time:5}}}}};
  root.Jx3Assistant={setActivity(){}};root.Jx3HarnessWorkspace={capture:async()=>fixtureScene()};
  root.setTimeout=fn=>{timers.set(++timerId,fn);return timerId;};root.clearTimeout=id=>timers.delete(id);
  root.fetch=async(url,options={})=>{
    calls.push({url,options});let result;
    if(url==='/api/macro/exact'&&options.method==='POST'){submitted=JSON.parse(options.body);result=job;}
    else if(url==='/api/macro/exact')result={available:true,job:null};
    else if(url.endsWith('/pause')){job={...job,pause_requested:true,phase:'paused'};result=job;}
    else if(url.endsWith('/resume')){job={...job,pause_requested:false,phase:'solving'};result=job;}
    else result=job;
    return {ok:true,text:async()=>JSON.stringify(result)};
  };
  exact.mount(root);await settle();const el=id=>doc.getElementById('em_'+id);assert.equal(el('horizon'),null);assert.equal(el('compress'),null);
  await el('start').click();assert.deepEqual(submitted.simulation,fixtureScene().simulation);assert.equal(submitted.horizon,30);
  assert.equal(Object.keys(submitted).some(k=>/budget|seconds|timeout/.test(k)),false);
  assert.equal(el('macro').value,job.best.macro);assert.match(el('verdict').textContent,/未通过/);assert.equal(el('start').disabled,true);
  job={...job,phase:'candidate',candidate:{macro:'/cast [rage>5] 斩刀',iteration:2,rule_count:1,comparison:null}};
  await [...timers.values()].at(-1)();assert.equal(el('macro').value,job.best.macro);await el('view').click();assert.equal(el('macro').value,job.candidate.macro);assert.match(el('verdict').textContent,/未验证/);
  await el('pause').click();assert.equal(el('pause').textContent,'继续');assert.equal(el('status').textContent,'已暂停');assert.equal(el('macro').value,job.candidate.macro);
  await el('pause').click();assert.equal(el('pause').textContent,'暂停');assert.equal(calls.filter(c=>c.url.endsWith('/resume')).length,1);
  job={...job,candidate:null,done:true,status:'exact',download_ready:true,best:{...job.best,comparison:{...job.best.comparison,reproduced:true,actual_count:23,order_prefix:23,exact_prefix:23,state_prefix:23,first_difference:null}}};
  await [...timers.values()].at(-1)();assert.equal(el('start').disabled,false);assert.equal(el('pause').disabled,true);assert.equal(el('download').disabled,false);assert.equal(el('verdict').textContent,'容差内匹配');
});
test('one shell moves original AI dock; ball drag does not open, keyboard moves, tabs preserve dock',async()=>{
  const root=domRoot(),doc=root.document,dock=doc.createElement('aside');dock.id='sim_ai_dock';doc.body.append(dock);const visibility=[];root.Jx3AgentDock={setEmbeddedVisible:open=>visibility.push(open)};
  shell.mount(root);const ball=doc.getElementById('assistant_ball'),container=doc.getElementById('assistant_shell');
  assert.equal(doc.getElementById('assistant_analysis_panel').children[0],dock);
  const startX=parseFloat(ball.style.left);await ball.emit('keydown',{key:'ArrowLeft'});assert.equal(parseFloat(ball.style.left),startX-16);
  await ball.emit('pointerdown',{clientX:100,clientY:100,pointerId:1});await ball.emit('pointermove',{clientX:70,clientY:60});await ball.emit('pointerup');await ball.click();assert.equal(container.hidden,true);
  await new Promise(resolve=>setTimeout(resolve,1));await ball.click();assert.equal(container.hidden,false);
  await doc.getElementById('assistant_tab_analysis').click();assert.equal(root.Jx3Assistant.mode(),'analysis');assert.equal(visibility.at(-1),true);
  await doc.getElementById('assistant_tab_analysis').emit('keydown',{key:'ArrowLeft'});assert.equal(root.Jx3Assistant.mode(),'harness');assert.equal(visibility.at(-1),false);
  await doc.getElementById('assistant_tab_exact').click();assert.equal(root.Jx3Assistant.mode(),'exact');assert.equal(doc.getElementById('assistant_exact_panel').hidden,false);assert.equal(doc.getElementById('assistant_harness_panel').hidden,true);
  await doc.getElementById('assistant_tab_exact').emit('keydown',{key:'ArrowRight'});assert.equal(root.Jx3Assistant.mode(),'harness');assert.equal(doc.getElementById('assistant_exact_panel').hidden,true);
  root.Jx3Assistant.setActivity(true,'analysis');root.Jx3Assistant.setActivity(false,'harness');assert.equal(ball.dataset.busy,'true');
  root.Jx3Assistant.close();assert.equal(container.hidden,true);assert.ok(root._store.has('jx3_assistant_geometry_v1'));
});
function uiFixture(handler){
  const root=domRoot(),panel=root.document.createElement('section');panel.id='assistant_harness_panel';root.document.body.append(panel);const calls=[],streams=[],timers=new Map();let timerId=0;
  root.Jx3Assistant={isOpen:()=>true,setActivity(){}};root.Jx3HarnessWorkspace={capture:async()=>({...fixtureScene(),sourceKey:'scene-key'})};
  root.fetch=async(url,options={})=>{calls.push({url,options});const result=await handler(url,options);return {ok:!result?.error,status:result?.error?409:200,json:async()=>result};};
  root.EventSource=class{constructor(url){this.url=url;this.listeners=new Map();streams.push(this);}addEventListener(type,fn){this.listeners.set(type,fn);}close(){this.closed=true;}emit(type,snapshot){this.listeners.get(type)?.({data:JSON.stringify(snapshot)});}};
  const context={window:root,module:{exports:{}},console,AbortSignal,setTimeout:(fn,ms)=>{const id=++timerId;timers.set(id,{fn,ms});return id;},clearTimeout:id=>timers.delete(id)};
  vm.runInNewContext(fs.readFileSync(require.resolve('../frontend/harness-run.js'),'utf8'),context);
  const el=id=>root.document.getElementById('hr_'+id);
  for(const [id,value]of Object.entries({duration:'120',pages:'2',calls:'16',sims:'192',seconds:'240',tokens:'192000'}))el(id).value=value;
  return {root,el,calls,streams,timers,panel};
}
function snap(status='running',sequence=1){return {run_id:'experiment-test',sequence,status,running:status==='running',mount:'FenShanJin',version:'AnYingQianJi',scenario_hash:'scene',events:[],artifacts:[],budget:{max_model_calls:12,max_simulations:128,wall_time_ms:180000,max_total_tokens:48000},usage:{model_calls:1,simulations:2,total_tokens:120,elapsed_ms:1000},resumable:false};}
test('real run start sends full fresh scene once; SSE updates and cancellation preserve terminal evidence',async()=>{
  let submitted;
  const ui=uiFixture((url,options)=>{
    if(url==='/api/agent/providers')return {profiles:[{id:'deepseek-flash',model:'deepseek-v4-flash',available:true}]};
    if(url==='/api/harness/runs'&&options.method==='POST'){submitted=JSON.parse(options.body);return {run_id:'experiment-test'};}
    if(url==='/api/harness/runs')return {runs:[snap()]};
    if(url.endsWith('/cancel'))return {run_id:'experiment-test',accepted:true};return snap();
  });await settle();
  const chip=ui.panel.querySelectorAll('[data-hr-goal]')[3];await chip.click();assert.match(ui.el('goal').value,/联合优化/);assert.equal(ui.calls.filter(c=>c.options.method==='POST').length,0);
  assert.equal(ui.el('output_tokens').value,'8192');assert.equal(ui.el('output_tokens').getAttribute('min'),'1024');assert.equal(ui.el('output_tokens').getAttribute('max'),'8192');ui.el('output_tokens').value='6144';
  await ui.el('start').click();await settle();
  assert.ok(submitted);assert.equal(submitted.budget.max_simulations,192);assert.equal(submitted.budget.max_model_calls,16);assert.equal(submitted.budget.max_total_tokens,192000);assert.equal(submitted.budget.max_output_tokens,6144);assert.deepEqual(submitted.simulation.pauses,[[3,1]]);assert.deepEqual(submitted.simulation.pre_releases,[{skill:'血怒',time_before:1}]);assert.equal(submitted.equipment.slots.PRIMARY_WEAPON.strength,4);
  assert.equal(ui.streams.length,1);await ui.el('cancel').click();assert.equal(ui.el('cancel').disabled,true);
  const done={...snap('cancelled',8),events:[{sequence:8,kind:'stopped',message:'保留证据'}],result:{summary:'已保留候选',completion:'partial',limitations:[]}};
  ui.streams[0].emit('completed',done);ui.streams[0].emit('progress',snap('running',3));
  assert.equal(ui.el('status').textContent,'已停止');assert.equal(ui.el('conclusion').textContent,'已保留候选');assert.equal(ui.streams[0].closed,true);assert.equal(ui.timers.size,0);
});
test('goal or invalid budget blocks API creation without discarding existing workspace',async()=>{
  const ui=uiFixture(url=>url==='/api/agent/providers'?{profiles:[{id:'deepseek-flash',available:true}]}:{runs:[]});await settle();
  await ui.el('start').click();assert.equal(ui.root.document.focused,ui.el('goal'));
  ui.el('goal').value='验证';ui.el('sims').value='-1';await ui.el('start').click();assert.match(ui.el('feedback').textContent,/范围/);assert.equal(ui.calls.filter(c=>c.options.method==='POST').length,0);
  ui.el('sims').value='192';
  for(const invalid of ['1023','8193','NaN','4096.5','']){ui.el('output_tokens').value=invalid;await ui.el('start').click();assert.match(ui.el('feedback').textContent,/范围/);assert.equal(ui.calls.filter(c=>c.options.method==='POST').length,0);}
});
test('application freezes run identity, blocks switching and rolls back on its original endpoint',async()=>{
  const scene=fixtureScene(),artifact={id:'evidence-one',kind:'evaluate',summary:'candidate',simulation:{...copy(scene.simulation),sequence:['盾压']},equipment:scene.equipment,result:{best:{verified:true,metrics:{dps:100},fingerprint:'123'}}};
  const finished={...snap('completed',8),artifacts:[artifact],result:{selected_artifact_id:artifact.id,summary:'done',completion:'verified',limitations:[]}};
  let releaseApply;
  const ui=uiFixture((url,options)=>{
    if(url==='/api/agent/providers')return {profiles:[{id:'offline',available:true}]};
    if(url==='/api/harness/runs')return options.method==='POST'?{run_id:'experiment-test'}:{runs:[finished]};
    if(url.endsWith('/artifacts'))return {request:scene,artifacts:[artifact]};
    if(url.endsWith('/apply'))return new Promise(resolve=>{releaseApply=resolve;});
    if(url.endsWith('/undo'))return {status:'undo_prepared'};
    return finished;
  });await settle();ui.el('goal').value='验证方案';await ui.el('start').click();await settle();
  ui.root.Jx3HarnessWorkspace.currentKey=()=> 'scene-key';ui.root.Jx3HarnessWorkspace.preview=()=>[];ui.root.Jx3HarnessWorkspace.apply=async()=>{throw new Error('replay mismatch');};
  await ui.el('preview').click();
  const dialog=ui.root.document.querySelector('.hr-dialog'),button=dialog.querySelectorAll('button').find(el=>el.textContent==='确认应用方案');
  const applying=button.click();await settle();assert.equal(ui.el('recent').disabled,true);assert.equal(ui.el('start').disabled,true);
  assert.equal(dialog.querySelector('[aria-label="关闭预览"]').disabled,true);
  let prevented=false;await dialog.emit('cancel',{preventDefault(){prevented=true;}});assert.equal(prevented,true);
  ui.el('recent').value='experiment-other';await ui.el('recent').emit('change');
  releaseApply({transaction_id:'tx1',after:{simulation:artifact.simulation,equipment:artifact.equipment}});await applying;
  assert.ok(ui.calls.some(call=>call.url==='/api/harness/runs/experiment-test/undo'));
  assert.equal(ui.calls.some(call=>call.url.includes('experiment-other')),false);
  assert.equal(ui.el('recent').disabled,false);assert.match(dialog.textContent,/replay mismatch/);
});
test('shipped scripts load bridge before run UI and share release cache version',()=>{
  const html=fs.readFileSync(require.resolve('../frontend/index.html'),'utf8');assert.ok(html.indexOf("'assistant-shell.js?")<html.indexOf("'harness-workspace.js?"));assert.ok(html.indexOf("'harness-workspace.js?")<html.indexOf("'harness-run.js?"));
  assert.match(html,/2\.1\.0-20260930/);for(const match of html.matchAll(/(?:\.js|\.css)\?v=([^'"\s]+)/g))assert.equal(match[1],'20260930-exactsequence1');
});
