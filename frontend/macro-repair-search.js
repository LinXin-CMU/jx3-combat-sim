/* 宏局部邻域搜索：阈值由时间轴状态产生，方向与结果由反事实模拟检验。 */
(function(root,factory){const api=factory();if(typeof module==='object'&&module.exports)module.exports=api;if(root)root.Jx3MacroRepairSearch=api;})(typeof window!=='undefined'?window:null,function(){
  'use strict';
  const skill=event=>(event?.name || '').replace(/·(?!雾海).*/, '');
  function value(state,field) {
    if(!state)return null;
    const resource={rage:'rage',sun:'berserk_value',energy:'block_value'}[field];
    if(resource)return Number.isFinite(state[resource])?state[resource]:null;
    const colon=field.indexOf(':'),kind=field.slice(0,colon),name=field.slice(colon+1);
    if(kind==='skill_energy') {
      const item=state.skill_states?.find(item=>item.name===name);
      return Number.isFinite(item?.charges)?item.charges:null;
    }
    const buffs=kind==='tbufftime'?state.target_buffs:state.buffs;
    if(!Array.isArray(buffs))return null;
    const buff=buffs.find(item=>item.name===name);
    if(kind==='buff')return buff?(Number.isFinite(buff.stacks)?buff.stacks:null):0;
    if(!buff)return null; // 缺失的 bufftime 是不成立，不把它伪造为时间零。
    return buff.permanent?Infinity:(Number.isFinite(buff.remaining)?buff.remaining:null);
  }
  function test(x,op,y) {
    if(x==null)return false;
    return op==='<'?x<y:op==='>'?x>y:op==='<='?x<=y+.001:op==='>='?x>=y-.001:op==='='?Math.abs(x-y)<.001:Math.abs(x-y)>=.001;
  }
  function boundaries(field,op,current,positive,negative) {
    const unit=/^(?:t?bufftime):/.test(field)?.1:1, values=new Set();
    for(const x of [...positive,...negative].filter(Number.isFinite)) {
      const lower=Math.floor(x/unit),upper=Math.ceil(x/unit);
      for(const n of [lower-1,lower,upper,upper+1])if(n>=0)values.add(Number((n*unit).toFixed(unit===1?0:1)));
    }
    const score=y=>positive.filter(x=>!test(x,op,y)).length/Math.max(1,positive.length)+negative.filter(x=>test(x,op,y)).length/Math.max(1,negative.length);
    const choose=direction=>[...values].filter(x=>x!==current && (direction<0?x<current:x>current))
      .sort((a,b)=>score(a)-score(b)||Math.abs(a-current)-Math.abs(b-current)||a-b).slice(0,2);
    const lower=choose(-1),upper=choose(1),out=[];
    for(let i=0;i<2;i++){if(lower[i]!=null)out.push(lower[i]);if(upper[i]!=null)out.push(upper[i]);}
    return out;
  }
  function relatedSkills(step,focus) {
    const names=[step.selected,skill(focus.actual)];
    return [...new Set(names)].filter(name=>name&&name!==step.target?.skill).slice(0,2);
  }
  function evidenceRole(target,line,step) {
    if(target.skill===step.target?.skill) {
      if(!line.passed)return {desired:true,reason:line.probe?.castable===false?'目标行条件不成立，且受可释放性限制':'目标行条件不成立',direct:line.probe?.castable!==false};
      return {desired:true,reason:line.probe?.castable?'目标行已满足条件与可释放性':'目标受可释放性限制',direct:false};
    }
    if(step.selected===target.skill&&line.line===step.selected_line&&line.passed)
      return {desired:false,reason:'实际选中另一技能，尝试限制该行',direct:true};
    return {desired:false,reason:line.passed?'关联行条件成立，但并未在本轮抢先':'关联行条件未成立',direct:false};
  }
  function thresholds(saved,step,focus,group,related=[]) {
    const names=new Set([step.target?.skill,step.selected]);
    for(const i of group?.references || [])names.add(skill(saved.template.timeline[i]));
    for(const i of group?.actuals || [])names.add(skill(saved.actual.timeline[i]));
    const out=[],seen=new Set();
    const falseCasts=new Set(saved.alignment.rows.filter(row=>row.kind==='extra').map(row=>row.actualIndex));
    for(const page of saved.pages)for(const line of page) {
      const parsed=/^\s*(\/(?:fcast|cast))\s+(.+?)\s+(\S+)\s*$/.exec(line.text);
      if(!parsed || !names.has(parsed[3]))continue;
      const [,command,raw,target]=parsed;
      const expression=raw.startsWith('[')&&raw.endsWith(']')?raw.slice(1,-1):raw;
      const atoms=/(^|[&|()])(\s*)((?:rage|sun|energy|(?:bufftime|tbufftime|buff|skill_energy):[^&|()<>=~]+))(>=|<=|~=|>|<|=)(\d+(?:\.\d+)?)(?=$|[&|()])/g;
      for(const match of expression.matchAll(atoms)) {
        const [,prefix,space,field,op,number]=match,current=Number(number);
        const positives=saved.template.timeline.filter(e=>!e.triggered&&skill(e)===target).map(e=>value(e.state_before,field));
        if(!positives.some(Number.isFinite))continue;
        // 模板其他技能位置与实际多放该技能的状态，均参与边界取样；它们只是条件搜索样本。
        const negatives=saved.template.timeline.filter(e=>!e.triggered&&skill(e)!==target).map(e=>value(e.state_before,field))
          .concat([...falseCasts].filter(i=>skill(saved.actual.timeline[i])===target).map(i=>value(saved.actual.timeline[i].state_before,field)));
        const start=match.index+prefix.length+space.length+field.length+op.length;
        for(const threshold of boundaries(field,op,current,positives,negatives)) {
          const changed=expression.slice(0,start)+threshold+expression.slice(start+number.length);
          const after=`${command} [${changed}] ${target}`;
          const text=saved.text.slice(0,line.start)+after+saved.text.slice(line.end);
          if(seen.has(text))continue;seen.add(text);
          const direction=op.startsWith('<')?(threshold>current?'放宽':'收紧'):op.startsWith('>')?(threshold<current?'放宽':'收紧'):'调整';
          const record=[step.target,...related].filter(Boolean).find(item=>item.skill===target);
          const observed=page===saved.pages[step.page-1]?record?.lines?.find(item=>item.line===page.indexOf(line)+1):null;
          const role=observed?evidenceRole(record,observed,step):null;
          const state={rage:step.rage,block_value:step.block_value,berserk_value:step.berserk_value};
          const observedValue=value(state,field);
          const crosses=role?.direct&&observedValue!=null&&test(observedValue,op,threshold)===role.desired&&test(observedValue,op,current)!==role.desired;
          const candidate={title:`${direction} ${target} 的条件边界`,text,before:line.text,after,
            edits:[{start:line.start,end:line.end,text:after}], priority:crosses?0:role?.direct?1:2,
            evidence:{origin:`${field}${op}${current} → ${field}${op}${threshold}；边界来自模板状态及实际多放样本，需整轴验证`}};
          if(role)candidate.evidence.origin=`${role.reason}；${candidate.evidence.origin}`;
          out.push(candidate);
          const anchor=saved.pages[step.page-1]?.[step.selected_line-1];
          if(page===saved.pages[step.page-1] && anchor && anchor.start<line.start) {
            const end=line.end+(saved.text[line.end]==='\n'?1:0);
            const without=saved.text.slice(0,line.start)+saved.text.slice(end);
            const moved=without.slice(0,anchor.start)+after+'\n'+without.slice(anchor.start);
            if(!seen.has(moved)) {seen.add(moved);out.push({...candidate,edits:undefined,text:moved,title:`${direction}条件并前移 ${target}`,after:`移到第 ${step.selected_line} 行前：${after}`});}
          }
        }
      }
    }
    // 每个宏行轮流取候选，避免某一行的多个阈值吃完试跑预算。
    out.sort((a,b)=>a.priority-b.priority);
    const buckets=new Map();for(const item of out){if(!buckets.has(item.before))buckets.set(item.before,[]);buckets.get(item.before).push(item);}
    const result=[];for(let i=0;i<4;i++)for(const bucket of buckets.values())if(bucket[i])result.push(bucket[i]);
    return result.slice(0,12);
  }
  function combine(saved,candidates,limit=4) {
    const rows=new Map(),out=[],seen=new Set();
    for(const c of candidates)if(c.edits?.length===1){const key=c.edits[0].start;if(!rows.has(key))rows.set(key,[]);if(rows.get(key).length<2)rows.get(key).push(c);}
    const groups=[...rows.values()];
    for(let a=0;a<groups.length;a++)for(let b=a+1;b<groups.length;b++)for(const left of groups[a])for(const right of groups[b]) {
      const edits=[...left.edits,...right.edits].sort((a,b)=>b.start-a.start);
      if(edits[1].end>edits[0].start)continue;
      let text=saved.text;for(const edit of edits)text=text.slice(0,edit.start)+edit.text+text.slice(edit.end);
      if(seen.has(text)||text===saved.text)continue;seen.add(text);
      out.push({title:'联合调整目标与关联行条件',text,before:`${left.before}\n${right.before}`,after:`${left.after}\n${right.after}`,edits,
        evidence:{origin:'合并两个不重叠的条件改动，重新运行完整宏；不把两条单独得分相加'}});
      if(out.length>=limit)return out;
    }
    return out;
  }
  function chooseDiverse(candidates,limit=6) {
    const out=[],families=new Set(),seen=new Set();
    for(const c of candidates) {
      const family=(c.expression || '').split(/[&|]/).map(atom=>atom.replace(/(?:>=|<=|~=|>|<|=).*/, '')).sort().join('&');
      if(families.has(family))continue;
      families.add(family);seen.add(c.macro_text);out.push(c);if(out.length>=limit)return out;
    }
    for(const c of candidates)if(!seen.has(c.macro_text)){seen.add(c.macro_text);out.push(c);if(out.length>=limit)break;}
    return out;
  }
  function anchor(focus) {
    if(focus.kind==='missing'&&focus.reference)return focus.reference.state_before?.time ?? focus.reference.cast_time;
    return focus.actual?.state_before?.time ?? focus.actual?.cast_time ?? focus.reference?.cast_time ?? 0;
  }
  function decisionIndex(steps,focus) {
    if(!steps.length)return -1;
    if(focus.kind!=='missing'&&focus.actual) {
      const actual=focus.actual;
      const index=steps.findIndex(s=>s.cast_success&&s.page===actual.macro_page&&s.selected_line===actual.macro_line&&Math.abs(s.cast_time-actual.cast_time)<.0001);
      if(index>=0)return index;
    }
    const time=anchor(focus);
    return steps.reduce((best,s,i)=>Math.abs(s.time-time)<Math.abs(steps[best].time-time)?i:best,0);
  }
  return Object.freeze({value,boundaries,thresholds,chooseDiverse,anchor,decisionIndex,relatedSkills,evidenceRole,combine});
});
