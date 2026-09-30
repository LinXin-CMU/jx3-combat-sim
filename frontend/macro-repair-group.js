/* 连续差异段的多行候选：状态取自模板，修改保持页边界，整轴验证由面板完成。 */
(function(root,factory){const api=factory();if(typeof module==='object'&&module.exports)module.exports=api;if(root)root.Jx3MacroRepairGroup=api;})(typeof window!=='undefined'?window:null,function(){
  'use strict';
  const name=e=>(e?.name || '').replace(/·(?!雾海).*/, '');
  const lineSkill=line=>line.text.trim().split(/\s+/).at(-1);
  function scope(saved,index,states=false) {
    const rows=saved.alignment.rows;
    const colored=r=>r && (r.kind==='missing'||r.kind==='extra'||(states&&r.kind==='changed'));
    let start=index,end=index;
    while(start>0&&colored(rows[start-1]))start--;
    while(end+1<rows.length&&colored(rows[end+1]))end++;
    const selected=rows.slice(start,end+1);
    return {start,end,references:selected.flatMap(r=>r.referenceIndex==null?[]:[r.referenceIndex]),actuals:selected.flatMap(r=>r.actualIndex==null?[]:[r.actualIndex])};
  }
  function assemble(saved,group,lines,removedSkills=[],fallbackPage=1) {
    const pages=new Map(), wanted=new Map();
    const all=saved.pages.flatMap((page,i)=>page.map(line=>({line,page:i+1})));
    for(const rule of lines) {
      const existing=all.filter(item=>lineSkill(item.line)===rule.skill);
      const pageIds=[...new Set(existing.map(item=>item.page))];
      let page=pageIds[0];
      if(pageIds.length>1)throw new Error(`${rule.skill} 分布在多个宏页，无法自动确定整段修改页。`);
      if(!page) {
        const event=group.references.map(i=>saved.template.timeline[i]).find(e=>name(e)===rule.skill);
        const stance=String(event?.state_before?.stance || '').toLowerCase();
        const headers=[...saved.text.matchAll(/^\s*#page\s*(\S+)\s*$/gm)];
        const i=headers.findIndex(h=>h[1].toLowerCase()===stance);
        page=i>=0?i+1:fallbackPage;
      }
      if(!saved.pages[page-1]?.length)throw new Error('目标宏页没有可定位的语句，无法安全插入整段候选。');
      const commands=new Set(existing.map(item=>/^\s*\/fcast\b/.test(item.line.text)?'/fcast':'/cast'));
      if(commands.size>1)throw new Error(`${rule.skill} 同时使用 cast 和 fcast，需先确认释放方式。`);
      const command=[...commands][0] || '/cast';
      const text=`${command} ${rule.expression?`[${rule.expression}] `:''}${rule.skill}`;
      if(!pages.has(page))pages.set(page,[]);
      if(!pages.get(page).includes(text))pages.get(page).push(text);
      wanted.set(rule.skill,page);
    }
    const removals=new Set(removedSkills), edits=[], before=[],after=[];
    for(let page=1;page<=saved.pages.length;page++) {
      const affected=saved.pages[page-1].filter(line=>wanted.get(lineSkill(line))===page || removals.has(lineSkill(line)));
      const replacements=pages.get(page) || [];
      if(!affected.length&&!replacements.length)continue;
      const anchor=affected[0] || saved.pages[page-1][0];
      if(!anchor)continue;
      for(const line of affected) {
        const end=line.end+(saved.text[line.end]==='\n'?1:0);
        edits.push({start:line.start,end,text:line===anchor&&replacements.length?replacements.join('\n')+'\n':''});
        before.push(`第 ${page} 页：${line.text}`);
      }
      if(!affected.length)edits.push({start:anchor.start,end:anchor.start,text:replacements.join('\n')+'\n'});
      after.push(...replacements.map(text=>`第 ${page} 页：${text}`));
    }
    let text=saved.text;
    for(const edit of edits.sort((a,b)=>b.start-a.start))text=text.slice(0,edit.start)+edit.text+text.slice(edit.end);
    return {title:'联合调整连续差异段的多行宏',text,before:before.join('\n')||'（补充缺失语句）',after:after.join('\n')||'（删除该段多放技能语句）',evidence:{origin:`连续差异段：模板 ${group.references.length} 次释放 / 实际 ${group.actuals.length} 次释放；联合条件仍以整轴验证为准`}};
  }
  async function generate(saved,group,request,page=1) {
    const timeline=saved.template.timeline;
    const input=saved.repairInput || timeline;
    const active=timeline.flatMap((e,i)=>e.triggered?[]:[i]);
    const refs=group.references.length?group.references:[];
    let variants=[[]];
    // 长差异段按最多八步分段取样，限制单次联合搜索耗时，合成的宏仍覆盖整个差异段，不截断验证范围。
    for(let offset=0;offset<refs.length;offset+=8) {
      const chunk=refs.slice(offset,offset+8),start=active.indexOf(chunk[0]),end=active.indexOf(chunk.at(-1));
      const body={timeline:input,selection_start:start,selection_end:end,step:0,options:{max_terms:2,max_candidates:120}};
      let choices;
      if(start===end) {
        const data=await request('/api/macro/assist',body);
        choices=(data.candidates||[]).slice(0,3).map(c=>[{skill:name(timeline[chunk[0]]),expression:c.expression}]);
      } else {
        const data=await request('/api/macro/assist/program',body);
        choices=(data.programs||[]).slice(0,3).map(p=>p.lines);
      }
      if(!choices.length)throw new Error('该段未找到联合条件候选。');
      variants=Array.from({length:3},(_,i)=>[...variants[i%variants.length],...choices[i%choices.length]]);
    }
    const targets=new Set(refs.map(i=>name(timeline[i]))), removed=[];
    const extras=[...new Set(group.actuals.map(i=>name(saved.actual.timeline[i])))].filter(s=>!targets.has(s));
    for(const skill of extras) {
      const index=timeline.findIndex(e=>!e.triggered&&name(e)===skill);
      if(index<0){removed.push(skill);continue;}
      const at=active.indexOf(index);
      const data=await request('/api/macro/assist',{timeline:input,selection_start:at,selection_end:at,step:0,options:{max_terms:1,max_candidates:120}});
      const choices=(data.candidates||[]).slice(0,3);
      if(!choices.length)throw new Error(`${skill} 缺少可用的限制条件。`);
      variants=variants.map((lines,i)=>[...lines,{skill,expression:choices[i%choices.length].expression}]);
    }
    return variants.map(lines=>assemble(saved,group,lines,removed,page));
  }
  return Object.freeze({scope,assemble,generate});
});
