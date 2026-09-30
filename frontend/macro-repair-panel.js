/* 目标诊断的交互层：提出有限改法，完整模拟验证，显式应用到独立草稿。 */
(function () {
  'use strict';
  const node = (tag,text,cls) => { const el=document.createElement(tag); if(text!=null)el.textContent=text; if(cls)el.className=cls; return el; };
  const sec = time => Number.isFinite(time) ? `${time.toFixed(1)}s` : '无';
  const button = (text,action) => { const el=node('button',text,'ma-button');el.type='button';el.onclick=action;return el; };
  let requestQueue=Promise.resolve();
  async function request(path,body) {
    const response=await fetch(path,{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(body),signal:AbortSignal.timeout(60000)});
    const data=await response.json().catch(()=>null);
    if(!response.ok || !data)throw new Error(data?.error || '验证未完成，请重试。');
    return data;
  }
  async function* suggestions(saved, index, request) {
    if(index==null)return;
    const timeline=saved.template.timeline;
    const active=timeline.slice(0,index+1).filter(event=>!event.triggered).length-1;
    const target=Jx3MacroRepair.skill(timeline[index]);
    const seen=new Set();
    const variants=seed=>[seed,...Jx3MacroRepair.relax(seed,target)].filter(c=>{
      if(seen.has(c.macro_text))return false;seen.add(c.macro_text);return true;
    });
    // 原子条件先进入验证，不等待组合搜索和删减。缓存每个阶段，切换焦点可以复用。
    saved.repairSuggestions ||= new Map();
    const cached=async(key,path,body)=>{
      if(!saved.repairSuggestions.has(key))saved.repairSuggestions.set(key,request(path,body).catch(error=>{saved.repairSuggestions.delete(key);throw error;}));
      return saved.repairSuggestions.get(key);
    };
    for(const terms of [1,2]) {
      const data=await cached(`${index}:${terms}`,'/api/macro/assist',{
        timeline:(saved.repairInput ||= Jx3MacroRepair.assistTimeline(timeline)),selection_start:active,selection_end:active,step:0,options:{max_terms:terms,max_candidates:120}});
      const seeds=Jx3MacroRepairSearch.chooseDiverse((data.candidates || []).filter(c=>terms===1 || c.terms>1),terms===1?4:2);
      for(const seed of seeds) {
        yield variants(seed);
        if(terms===2) {
          const pruned=await cached(`prune:${seed.macro_text}`,'/api/macro/prune_candidates',{macro_text:seed.macro_text});
          for(const item of pruned.candidates || []) {
            const expression=item.after_cond==='(无)'?'':item.after_cond;
            yield variants({expression,macro_text:`/cast ${expression?`[${expression}] `:''}${target}`,origin:'从轴状态生成的组合删减子条件（放宽或收紧均需验证）'});
          }
        }
      }
    }
    yield variants({expression:'',macro_text:`/cast ${target}`,origin:'移除全部条件的放宽对照'});
  }
  async function verify(saved,candidate,request) {
    saved.repairRuns ||= new Map();
    if(!saved.repairRuns.has(candidate.text)) {
      const promise=request('/api/macro/diagnose',{simulation:{...saved.request,macro_text:candidate.text},version:saved.version,mount:saved.mount,
        start:0,end:0,include_result:true}).then(data=>{
          if(!Array.isArray(data.simulation?.timeline))throw new Error('后端未返回候选循环，请更新后重试。');
          const actual=data.simulation;
          if(actual.skipped?.length)throw new Error(`候选存在未完成动作：${actual.skipped[0][1] || '无法释放'}`);
          const windowed=timeline=>timeline.map(event=>event.cast_time>=0 && event.cast_time<saved.window ? event : null);
          return {actual,alignment:Jx3MacroAlignment.align(windowed(saved.template.timeline),windowed(actual.timeline))};
        }).catch(error=>{saved.repairRuns.delete(candidate.text);throw error;});
      if(saved.repairRuns.size>=24)saved.repairRuns.delete(saved.repairRuns.keys().next().value);
      saved.repairRuns.set(candidate.text,promise);
    }
    return saved.repairRuns.get(candidate.text);
  }
  function render(host, options) {
    const grid=host; grid.classList.add('ma-diagnostic-columns');
    host=node('section',null,'ma-diagnostic-flow');
    const repairColumn=node('section',null,'ma-diagnostic-repairs');
    host.append(node('h4','判定流程'));repairColumn.append(node('h4','自动验证改法'));
    grid.append(host,repairColumn);
    const {saved,step,focus,current,locate,apply,first}=options, repair=Jx3MacroRepair;
    const why=repair.diagnosis(step,focus), target=step.target;
    const group=Jx3MacroRepairGroup.scope(saved,focus.index,document.getElementById('macro_compare_layer')?.value==='states');
    const groupMode=group.end>group.start;
    if(groupMode)repairColumn.querySelector('h4').textContent='自动验证整段改法';
    const valid=()=>host.isConnected && current();
    // 同一浏览器只派发一条改法请求，过期焦点在队列中直接丢弃，避免积压后端任务。
    const queuedRequest=(path,body)=>{
      progress.textContent='等待验证队列…';
      const work=requestQueue.then(()=>{
        if(!valid())throw new Error('焦点已变化');
        const stage=path.endsWith('/program')?'分析连续组合':path.endsWith('/assist')?'提取状态条件':path.endsWith('/prune_candidates')?'简化条件':'完整模拟验证';
        progress.textContent=`正在${stage}…`;
        return request(path,body).catch(error=>{
          if(error.name==='TimeoutError')throw new Error(`${stage}超过 60 秒，本项已跳过`);
          throw error;
        });
      });
      requestQueue=work.catch(()=>{});return work;
    };
    host.append(node('p',why.text,'ma-diagnostic-conclusion'));
    const resource=saved.mount==='TieGuYi'?` / 格挡 ${step.block_value}`:saved.version==='CangShengZhuShiTest'?` / 暴怒 ${step.berserk_value}`:'';
    host.append(node('p',`该轮状态：怒气 ${step.rage}${resource} · 上次成功技能：${step.last_skill || '无'}`,'ma-explanation'));
    if(focus.earlier) {
      const hint=node('div',null,'ma-repair-actions');
      hint.append(node('span','前面已有技能分歧，这里可能受其影响。','ma-explanation'),button('先看首处技能分歧',()=>{if(current())first();}));host.append(hint);
    }
    if(target) {
      host.append(node('h4',`Step 1 · ${target.skill} 是否进入技能池`));
      if(!target.lines.length)host.append(node('p',target.other_pages.length ? `当前页无此技能；其他页：${target.other_pages.join('、')}。` : '整个宏中未找到该技能语句。'));
      for(const line of target.lines) {
        const row=node('div',null,`ma-diagnostic-row ${line.passed?'ma-diagnostic-pass':'ma-diagnostic-fail'}`);
        row.append(button(`第 ${line.line} 行`,()=>{if(current())locate(step.page,line.line);}),node('code',`${line.condition} → ${line.passed?'成立':'不成立'}`));host.append(row);
        if(line.atoms.length) {
          const values=node('div');
          line.atoms.forEach(atom=>values.append(node('p',`${atom.condition}：${atom.actual} → ${atom.passed?'成立':'不成立'}`,atom.passed?'':'ma-diagnostic-warning')));
          host.append(values);
        }
        if(line.truncated)host.append(node('p','子条件过多，未全部展开。','ma-explanation'));
      }
      if(target.lines.some(line=>line.atoms.length>1))host.append(node('p','上面各子条件独立复查；整体结果按括号中的 AND / OR 关系计算。','ma-explanation'));
      host.append(node('h4',`Step 2 · ${target.skill} 能否释放，是否被前行抢先`));
      const probes=target.lines.length ? target.lines.map(line=>({line:line.line,probe:line.probe})) : [{line:0,probe:target.probe}];
      for(const {line,probe} of probes)host.append(node('p',`${line?`第 ${line} 行`:'按 /cast 复查'}：${probe.castable?'此刻可释放':probe.reason}`,probe.castable?'':'ma-diagnostic-warning'));
      if(step.selected_line!=null)host.append(node('p',`实际本轮选中：第 ${step.selected_line} 行 ${step.selected}。`));
      host.append(node('p','这里复用 Step 2 独立检查目标技能；未改变实际技能池，也不代表原执行器检查过后续行。','ma-explanation'));
    }
    const area=node('div',null,'ma-repair-results'), actions=node('div',null,'ma-repair-actions');
    const progress=node('span','准备自动验证…','ma-repair-progress');progress.setAttribute('role','status');
    const tabs=node('div',null,'ma-repair-tabs');tabs.setAttribute('role','tablist');tabs.setAttribute('aria-label','已验证改法');
    let selectedCard=null;
    function selectCard(card) { selectedCard=card;for(const item of area.children)item.hidden=item!==card;for(const tab of tabs.children)tab.setAttribute('aria-selected',String(tab.card===card)); }
    let removed=0, filtered=0;
    const run=async()=>{
      if(!valid())return;progress.textContent='正在提取轴状态条件…';
      try {
        const seen=new Set(), ranked=[], related=[];
        let count=0;
        saved.relatedChecks ||= new Map();
        for(const skill of Jx3MacroRepairSearch.relatedSkills(step,focus)) {
          if(!valid())return;
          const key=`${step.time}:${step.page}:${step.selected_line}:${skill}`;
          try {
            if(!saved.relatedChecks.has(key))saved.relatedChecks.set(key,queuedRequest('/api/macro/diagnose',{
              simulation:saved.request,version:saved.version,mount:saved.mount,
              start:Math.max(0,step.time-.001),end:Math.min(saved.window,step.time+.001),target_skill:skill,
            }).then(data=>{
              if(data.fingerprint!==saved.actual.fingerprint)throw new Error('关联复查与冻结场景不一致');
              const round=data.trace?.decisions.find(s=>Math.abs(s.time-step.time)<.0001&&s.page===step.page&&s.selected_line===step.selected_line&&s.last_skill===step.last_skill&&s.rage===step.rage);
              if(!round?.target)throw new Error('未记录到同一轮的关联技能条件');
              return round.target;
            }).catch(error=>{saved.relatedChecks.delete(key);throw error;}));
            const checked=await saved.relatedChecks.get(key);if(!valid())return;related.push(checked);
            const section=node('details',null,'ma-related-check');section.open=true;section.dataset.time=String(step.time);
            section.append(node('summary',`关联技能 · ${skill}（同一轮复查）`));
            if(!checked.lines.length)section.append(node('p','当前页没有此技能语句。','ma-explanation'));
            for(const line of checked.lines) {
              const role=Jx3MacroRepairSearch.evidenceRole(checked,line,step);
              section.append(node('p',role.reason,'ma-explanation'));
              const row=node('div',null,`ma-diagnostic-row ${line.passed?'ma-diagnostic-pass':'ma-diagnostic-fail'}`);
              row.append(button(`第 ${line.line} 行`,()=>{if(valid())locate(step.page,line.line);}),node('code',`${line.condition} → ${line.passed?'成立':'不成立'}`));section.append(row);
              for(const atom of line.atoms)section.append(node('p',`${atom.condition}：${atom.actual} → ${atom.passed?'成立':'不成立'}`));
              section.append(node('p',`Step 2：${line.probe.castable?'此刻可释放':line.probe.reason}`));
            }
            host.append(section);
          } catch(error) {if(valid())actions.append(node('span',`关联技能 ${skill}：${error.message}`,'ma-diagnostic-warning'));}
        }
        const bank=Jx3MacroRepairSearch.thresholds(saved,step,focus,group,related);
        async function* batches() {
          // 按同轮证据优先验证阻断原子的边界，并给关联行与两行联动预留预算。
          yield bank.slice(0,6);
          yield Jx3MacroRepairSearch.combine(saved,bank,2);
          // 整段与单点候选在同一轮验证和排序。
          if(groupMode) {
            saved.repairGroups ||= new Map();
            saved.repairInput ||= repair.assistTimeline(saved.template.timeline);
            const key=`${group.start}:${group.end}:${step.page}`;
            try {
              if(!saved.repairGroups.has(key))saved.repairGroups.set(key,Jx3MacroRepairGroup.generate(saved,group,async(path,body)=>{
                if(!valid())throw new Error('焦点已变化');return queuedRequest(path,body);
              },step.page).catch(error=>{saved.repairGroups.delete(key);throw error;}));
              const joint=await saved.repairGroups.get(key);
              if(!valid())return;
              yield joint;
            } catch(error) {if(valid())actions.append(node('span',`整段条件分析未完成：${error.message}`,'ma-diagnostic-warning'));}
          }
          yield repair.proposals(saved,step,focus);
          const sources=[];
          if(!['castability','unavailable'].includes(why.kind))sources.push({index:focus.referenceIndex,step,focus});
          for(const checked of related) {
            const index=saved.template.timeline.findIndex(e=>!e.triggered&&repair.skill(e)===checked.skill);
            if(index>=0)sources.push({index,step:{...step,target:checked},focus:{...focus,referenceIndex:index,reference:saved.template.timeline[index]}});
          }
          const streams=sources.map(source=>({...source,iterator:suggestions(saved,source.index,queuedRequest)}));
          let combined=false;
          while(streams.length)for(let i=0;i<streams.length;) {
            const source=streams[i];
            try {
              const next=await source.iterator.next();if(!valid())return;
              if(next.done){streams.splice(i,1);continue;}
              const choices=repair.proposals(saved,source.step,source.focus,next.value);
              bank.push(...choices);yield choices;
              if(!combined && i===streams.length-1) {
                combined=true;
                yield Jx3MacroRepairSearch.combine(saved,bank.filter(c=>c.priority===undefined),2);
              }
              i++;
            } catch(error) {if(valid())actions.append(node('span',`部分条件生成失败：${error.message}`,'ma-diagnostic-warning'));streams.splice(i,1);}
          }
        }
        outer: for await(const candidates of batches()) for(const candidate of candidates) {
          if(!valid())return;
          if(seen.has(candidate.text))continue;
          if(count>=24)break outer;
          seen.add(candidate.text);count++;
          progress.textContent=`已完成 ${count-1} · 验证中…`;
          const card=node('div',null,'ma-repair-card');card.append(node('strong',candidate.title));
          if(candidate.evidence) {
            const e=candidate.evidence;
            card.append(node('p',Number.isFinite(e.tp)?`轴状态初筛：目标位置命中 ${e.tp}/${e.tp+e.fn}，其他技能位置命中 ${e.fp}；${e.terms} 个条件。`:`${e.origin}，是否有效以下方整轴试跑为准。`,'ma-explanation'));
          }
          card.append(node('pre',`− ${candidate.before}`,'ma-diagnostic-fail'),node('pre',`+ ${candidate.after}`,'ma-diagnostic-pass'));
          const state=node('p','正在运行同环境完整循环…');card.append(state);
          try {
            const result=await verify(saved,candidate,queuedRequest);if(!valid())return;
            const score=repair.compare(saved,result.actual,result.alignment,focus);
            if(!repair.worthShowing(score,candidate.text,saved.text)){filtered++;continue;}
            const simpler=score.equivalent && candidate.text.length<saved.text.length;
            state.textContent=`${score.improved?'技能差异减少':simpler?'技能序列等效，条件已简化':'技能差异未减少'}${focus.reference?' · 目标位置'+(score.targetMatched?'已匹配':'仍未匹配'):''}${score.clean?' · 未新增技能分歧':' · 有新增分歧'}`;
            if(groupMode) {
              const matched=group.references.filter(i=>result.alignment.rows.some(row=>row.referenceIndex===i&&row.actualIndex!=null)).length;
              card.append(node('p',`整段模板技能匹配 ${matched}/${group.references.length} 次；下方为完整循环差异。`,'ma-explanation'));
            }
            const metrics=node('div',null,'ma-repair-metrics');
            for(const [key,label] of [['missing','缺失'],['extra','多放'],['changed','时间 / 状态']])metrics.append(node('span',`${label} ${score.before[key]} → ${score.after[key]}`));card.append(metrics);
            const positions=indices=>indices.slice(0,5).map(i=>`${saved.template.timeline[i].name} ${sec(saved.template.timeline[i].cast_time)}`).join('、');
            if(score.recovered.length)card.append(node('p',`补回 ${score.recovered.length} 处：${positions(score.recovered)}`));
            if(score.newMissing.length)card.append(node('p',`新增缺失 ${score.newMissing.length} 处：${positions(score.newMissing)}`,'ma-diagnostic-warning'));
            if(score.newExtra.length)card.append(node('p',`新增多放位置 ${score.newExtra.length} 处：${score.newExtraEvents.map(e=>`${e.name} ${sec(e.time)}`).join('、')}`,'ma-diagnostic-warning'));
            const use=button('应用到草稿并重跑',async()=>{if(!valid())return;use.disabled=true;await apply(candidate.text);});
            use.disabled=!score.improved && !simpler;card.append(use);
            const entry={card,score,text:candidate.text,outcome:repair.outcomeKey(result.actual)};
            if(ranked.some(item=>repair.supersedes(item,entry))) {removed++;continue;}
            for(let i=ranked.length-1;i>=0;i--)if(repair.supersedes(entry,ranked[i])) {
              const old=ranked.splice(i,1)[0];old.card.remove();old.tab.remove();removed++;
              if(selectedCard===old.card)selectedCard=card;
            }
            ranked.push(entry);
            ranked.sort((a,b)=>Jx3MacroRepair.rank(a,b));
            const tab=button(`改法 ${count}`,()=>selectCard(card));tab.setAttribute('role','tab');tab.card=card;tab.title=candidate.title;
            ranked.find(item=>item.card===card).tab=tab;
            area.append(card);
            while(ranked.length>5) {
              const old=ranked.pop();old.card.remove();old.tab.remove();filtered++;
              if(selectedCard===old.card)selectedCard=null;
            }
            for(const [i,item] of ranked.entries()){item.tab.textContent=`改法 ${i+1}`;tabs.append(item.tab);}
            selectCard(selectedCard || ranked[0].card);
          } catch(error) {if(!valid())return;actions.append(node('span',`第 ${count} 条验证失败：${error.message}`,'ma-diagnostic-warning'));}
        }
        if(!ranked.length && valid())area.append(node('p','暂未找到有效的局部改法。已过滤未改善或新增技能分歧的方案，可先检查前序分歧。','ma-explanation'));
      } catch(error) {if(valid())area.append(node('p',error.message,'ma-diagnostic-warning'));}
      finally {if(valid()){progress.textContent=`保留 ${tabs.children.length} 条${removed?` · 合并 ${removed} 条`:""}`;progress.title=`已过滤 ${filtered} 条效果不足或排名靠后的方案，合并 ${removed} 条复杂方案。`;progress.dataset.complete='true';}}
    };
    const help=node('span','ⓘ','ma-repair-help');help.tabIndex=0;
    help.setAttribute('aria-label','自动验证说明');
    help.title=`从技能轴正反例生成并删减条件；相同战斗结果只保留更简单的版本。按新增分歧、总差异、时间误差及复杂度排序；覆盖 ${sec(saved.window)}，仅展示无新增技能分歧的改善或等效简化，最多 5 条；不自动修改草稿。`;
    const heading=repairColumn.querySelector('h4');
    actions.classList.add('ma-repair-toolbar');
    actions.append(heading,progress,help);
    repairColumn.append(actions,tabs,area);
    setTimeout(()=>{if(valid())run();},0);
  }
  window.Jx3MacroRepairPanel=Object.freeze({render});
})();
