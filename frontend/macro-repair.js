/* 模板意图、局部改法及全窗口验证指标。宏事实由后端执行器提供。 */
(function (root, factory) {
  const api = factory(typeof module === 'object' && module.exports ? require('./macro-alignment.js').align : (...args)=>root.Jx3MacroAlignment.align(...args));
  if (typeof module === 'object' && module.exports) module.exports = api;
  if (root) root.Jx3MacroRepair = api;
})(typeof window !== 'undefined' ? window : null, function (alignActual) {
  'use strict';
  function skill(event) {
    const name = event?.name || '';
    return /^(阵云结晦|月照连营|雁门迢递)·雾海(?:·|$)/.test(name) ? `${name.split('·')[0]}·雾海` : name.split('·')[0];
  }
  function intent(saved, index) {
    const rows = saved.alignment.rows, row = rows[index];
    let referenceIndex = row.referenceIndex, actualIndex = row.actualIndex;
    // LCS 的删除/插入是一个缺口；用后续实际施放作为观察锚点，不声称二者精确配对。
    if (referenceIndex == null) referenceIndex = rows.slice(index + 1).find(r => r.referenceIndex != null)?.referenceIndex ?? null;
    if (actualIndex == null) actualIndex = rows.slice(index + 1).find(r => r.actualIndex != null)?.actualIndex ?? null;
    const firstSkill = rows.findIndex(r => r.kind === 'missing' || r.kind === 'extra');
    return { referenceIndex, actualIndex, reference: saved.template.timeline[referenceIndex] || null,
      actual: saved.actual.timeline[actualIndex] || null, approximate: row.referenceIndex == null || row.actualIndex == null,
      kind: row.kind, firstSkill, earlier: firstSkill >= 0 && firstSkill < index, index };
  }
  function diagnosis(step, focus) {
    const target = step.target;
    if (!focus.reference) return { kind:'extra', text:'模板后续没有对应技能，检查这条宏是否在不需要的位置仍然成立。' };
    if (!target) return { kind:'unavailable', text:'该轮目标复查未记录，不能据此判断阻断原因。' };
    if (step.cast_success && target.lines.some(line => line.line === step.selected_line))
      return { kind:'selected', text:`本轮已选择 ${target.skill}。这里的差异在时间、档位或前序状态，单改该行条件未必有效。` };
    if (!target.lines.length) return { kind:'absent', text:target.other_pages.length
      ? `${target.skill} 只在第 ${target.other_pages.join('、')} 页；当前第 ${step.page} 页没有该技能。`
      : `当前宏没有 ${target.skill} 的释放语句。` };
    const passed = target.lines.filter(line => line.passed);
    if (!passed.length) return { kind:'condition', text:`Step 1 阻断：${target.skill} 的 ${target.lines.length} 条语句条件均不成立。` };
    const ready = passed.find(line => line.probe.castable);
    if (!ready) return { kind:'castability', text:`Step 2 阻断：条件已成立，但 ${passed.map(line => `第 ${line.line} 行：${line.probe.reason}`).join('；')}。` };
    if (step.selected_line != null && step.selected_line < ready.line)
      return { kind:'priority', line:ready.line, text:`Step 2 顺序阻断：${target.skill} 此时可释放，但前面的第 ${step.selected_line} 行 ${step.selected} 先被选中。` };
    return { kind:'execution', text:'目标通过了条件及独立可释放性复查；需检查最终施放记录，不能归因为条件失败。' };
  }
  function source(saved, page, line) { return saved.pages[page-1]?.[line-1]; }
  function remove(text, line) { const end = line.end + (text[line.end] === '\n' ? 1 : 0); return text.slice(0,line.start) + text.slice(end); }
  function replace(saved, line, macro) { return saved.text.slice(0,line.start) + macro + saved.text.slice(line.end); }
  function putBefore(saved, line, macro, anchor) {
    if (!anchor || anchor === line) return line ? replace(saved,line,macro) : saved.text;
    if (!line) return saved.text.slice(0,anchor.start) + macro + '\n' + saved.text.slice(anchor.start);
    const text = remove(saved.text,line), removed = saved.text.length - text.length;
    const offset = anchor.start > line.start ? anchor.start - removed : anchor.start;
    return text.slice(0,offset) + macro + '\n' + text.slice(offset);
  }
  function proposals(saved, step, focus, suggestions = [], suppressions = []) {
    const out = [], seen = new Set([saved.text]), target = step.target, why = diagnosis(step,focus);
    const add = (title,text,before,after,evidence=null,edits=undefined) => {
      if (text && !seen.has(text) && out.length < 24) { seen.add(text); out.push({title,text,before,after,evidence,edits}); }
    };
    const selected = source(saved,step.page,step.selected_line);
    if (why.kind === 'priority') {
      const line = source(saved,step.page,why.line);
      if (line && selected) add(`把 ${target.skill} 提到 ${step.selected} 前`,putBefore(saved,line,line.text,selected),line.text,`移至第 ${step.selected_line} 条之前：${line.text}`);
    }
    if (target && !['castability','unavailable'].includes(why.kind)) {
      const own = target.lines.find(line => line.probe.castable) || target.lines[0];
      const line = source(saved,step.page,own?.line), anchor = selected || saved.pages[step.page-1]?.[0];
      for (const candidate of suggestions.slice(0,8)) {
        const macro = /^\s*\/fcast\b/.test(line?.text || '') ? candidate.macro_text.replace(/^\/cast\b/,'/fcast') : candidate.macro_text;
        if (line) add(`调整 ${target.skill} 条件`,replace(saved,line,macro),line.text,macro,candidate,[{start:line.start,end:line.end,text:macro}]);
        if (anchor && (!line || anchor.start < line.start)) add(`${line ? '调整条件并前移' : '补充'} ${target.skill}`,putBefore(saved,line,macro,anchor),line?.text || '（当前页无此技能）',`${macro}\n放在当前页第 ${step.selected_line || 1} 条之前`,candidate);
      }
    }
    if (selected && focus.approximate) {
      // 删除是待验证假设；全窗口统计会暴露其他位置受损，不默认应用。
      if (!saved.template.timeline.some(event => !event.triggered && skill(event) === step.selected))
        add(`移除模板未使用的 ${step.selected} 语句`,remove(saved.text,selected),selected.text,'（删除该行）');
      for (const candidate of suppressions.slice(0,8)) add(`调整 ${step.selected} 的触发条件`,replace(saved,selected,candidate.macro_text),selected.text,candidate.macro_text,candidate);
    }
    return out;
  }
  function compare(saved, actual, alignment, focus) {
    const oldMissing = new Set(saved.alignment.rows.filter(r=>r.kind==='missing').map(r=>r.referenceIndex));
    const missing = new Set(alignment.rows.filter(r=>r.kind==='missing').map(r=>r.referenceIndex));
    const newMissing = [...missing].filter(i=>!oldMissing.has(i));
    // 比较两次实际轨迹中的额外动作身份，不能因补回模板动作导致邻接位置变化而误报新增。
    const oldExtra=new Set(saved.alignment.rows.filter(row=>row.kind==='extra').map(row=>row.actualIndex));
    const windowed=timeline=>timeline.map(e=>e && e.cast_time>=0 && (!Number.isFinite(saved.window)||e.cast_time<saved.window)?e:null);
    const correspondence=alignActual(windowed(saved.actual.timeline),windowed(actual.timeline));
    const inheritedExtra=new Set(correspondence.rows.filter(row=>row.referenceIndex!=null&&row.actualIndex!=null&&oldExtra.has(row.referenceIndex)).map(row=>row.actualIndex));
    const newExtra=alignment.rows.filter(row=>row.kind==='extra'&&!inheritedExtra.has(row.actualIndex)).map(row=>row.actualIndex);
    const before = saved.alignment.summary, after = alignment.summary;
    const targetRow = focus.referenceIndex == null ? null : alignment.rows.find(r=>r.referenceIndex===focus.referenceIndex);
    const active=timeline=>timeline.filter(e=>e && !e.triggered && e.cast_time>=0 && e.cast_time<saved.window && e.skill_id!==90001 && !e.name.startsWith('__'));
    const previous=active(saved.actual.timeline), next=active(actual.timeline);
    const equivalent=previous.length===next.length && previous.every((e,i)=>e.name===next[i].name && Math.abs(e.cast_time-next[i].cast_time)<=1/16);
    return {before,after, recovered:[...oldMissing].filter(i=>!missing.has(i)),newMissing,newExtra,
      equivalent, timeError:alignment.rows.reduce((sum,row)=>sum+Math.abs(row.timeDelta || 0),0),
      targetMatched:!!targetRow && targetRow.actualIndex != null,
      improved: after.missing+after.extra < before.missing+before.extra,
      clean:!newMissing.length && !newExtra.length,
      newExtraEvents:newExtra.slice(0,5).map(i=>({name:actual.timeline[i].name,time:actual.timeline[i].cast_time}))};
  }
  // 只处理完整原子表达式，不用字符串替换改变 AND/OR 的右结合语义。
  function relax(candidate,target) {
    const expression=(candidate.expression || '').trim();
    const match=/^(bufftime|tbufftime|buff):([^&|<>=()]+)(>=|<=|>|<|=)(\d+(?:\.\d+)?)$/.exec(expression);
    if(!match)return [];
    const [,kind,name,op,value]=match;
    // buff 层数为零可能表示不存在；只有排除零的条件才能放宽为存在。
    if(kind==='buff' && !((op==='>' && +value>=0) || ((op==='>=' || op==='=') && +value>0)))return [];
    const relaxed=`${kind==='tbufftime'?'tbuff':'buff'}:${name}`;
    return [{expression:relaxed,macro_text:`/cast [${relaxed}] ${target}`,origin:`轴状态条件 ${expression} 放宽为 ${relaxed}，不沿用原宏条件`}];
  }
  function assistTimeline(timeline) {
    const pick=(value,keys)=>Object.fromEntries(keys.filter(key=>value[key]!==undefined).map(key=>[key,value[key]]));
    const buff=b=>pick(b,['name','buff_id','remaining','stacks','permanent']);
    return timeline.map(event=>{
      const row=pick(event,['name','skill_id','triggered','cast_time']);
      const state=event.state_before;
      if(!state){row.state_before=null;return row;}
      row.state_before=pick(state,['time','rage','block_value','berserk_value','max_berserk_value']);
      for(const key of ['buffs','target_buffs'])if(state[key]!=null)row.state_before[key]=state[key].map(buff);
      if(state.skill_states!=null)row.state_before.skill_states=state.skill_states.map(s=>pick(s,['name','skill_id','charges','max_charges','not_in_cd']));
      return row;
    });
  }
  function worthShowing(score,text,original) {
    return score.clean && (score.improved || (score.equivalent && text.length<original.length));
  }
  function outcomeKey(actual) {
    // 宏行号随删减/前移变化，不是战斗差异；保留时间、档位、伤害及全部状态快照。
    return JSON.stringify([actual.dps,actual.fight_time,(actual.timeline || []).map(event=>{
      const {macro_line,macro_page,...combat}=event;return combat;
    }),actual.skipped || []]);
  }
  function complexity(text) {
    const conditions=[...text.matchAll(/^\s*\/(?:fcast|cast)\s+\[([^\]]*)\]/gm)];
    return [conditions.reduce((sum,m)=>sum+1+(m[1].match(/[&|]/g) || []).length,0),text.length];
  }
  function supersedes(a,b) {
    if(a.outcome!==b.outcome)return false;
    const x=complexity(a.text),y=complexity(b.text);
    return x[0]<y[0] || (x[0]===y[0] && x[1]<y[1]);
  }
  function rank(a,b) {
    return (a.score.newMissing.length+a.score.newExtra.length)-(b.score.newMissing.length+b.score.newExtra.length)
      || (a.score.after.missing+a.score.after.extra)-(b.score.after.missing+b.score.after.extra)
      || Math.round(a.score.timeError*16)-Math.round(b.score.timeError*16)
      || a.text.length-b.text.length || a.text.localeCompare(b.text);
  }
  return Object.freeze({skill,intent,diagnosis,proposals,compare,rank,relax,outcomeKey,complexity,supersedes,worthShowing,assistTimeline});
});
