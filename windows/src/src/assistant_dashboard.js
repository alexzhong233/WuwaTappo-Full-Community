/* DOM-only view; background ownership stays in the main pet renderer. */
function assistantUi(window) {
  const doc=window.document;
  if(!doc.getElementById('wt-assistant-style')){
    const style=doc.createElement('style');style.id='wt-assistant-style';
    style.textContent=`
    .wt-console,.wt-gate{--ink:#34322e;--muted:#8c857a;--line:#e9e4db;--gold:#b78643;--cream:#fff4dc;color:var(--ink);font:14px/1.6 "Segoe UI","Microsoft YaHei",sans-serif;text-align:left;letter-spacing:0;box-sizing:border-box;user-select:text}
    .wt-console *,.wt-gate *{box-sizing:border-box}
    .wt-console{position:fixed;inset:10px;z-index:2147483000;background:#f7f6f2;border:1px solid #e5dfd5;border-radius:20px;box-shadow:0 12px 50px #34261038;display:flex;flex-direction:column;overflow:hidden}
    .wt-console h2,.wt-console h3,.wt-console p,.wt-gate h2,.wt-gate p{margin:0}
    .wt-console h2{font-size:24px;line-height:1.4;font-weight:750}.wt-console h3{font-size:14px;font-weight:700}
    .wt-console button,.wt-gate button{font:inherit;color:inherit;cursor:pointer;background:white;border:1px solid var(--line);border-radius:11px;padding:8px 16px;line-height:1.5;transition:background .15s;box-shadow:none;min-height:36px;outline-offset:3px}
    .wt-console button:hover,.wt-gate button:hover{background:#fff7e6;border-color:#dbb77b}.wt-console button:disabled,.wt-gate button:disabled{opacity:.5;cursor:default}
    .wt-console .wt-primary,.wt-gate .wt-primary{background:#efd8ac;border-color:#efd8ac;color:#654921;font-weight:650}
    .wt-console header{padding:20px 24px 10px}.wt-console .wt-eyebrow{font-size:10px;font-weight:700;letter-spacing:1.2px;color:#a08a6a;margin-bottom:5px}
    .wt-row{display:flex;align-items:center;gap:10px}.wt-between{justify-content:space-between}.wt-wrap{flex-wrap:wrap}.wt-muted{color:var(--muted);font-size:12px}
    .wt-console .wt-close{border:0;padding:2px 9px;background:transparent;font-size:25px;color:#82786b;min-height:32px}.wt-badge{font-size:11px;background:#e6eddd;color:#53683f;border-radius:20px;padding:3px 11px;white-space:nowrap}.wt-badge.paused{background:#ece8e0;color:#84745e}
    .wt-console nav{display:flex;gap:6px;padding:0 24px 12px;border-bottom:1px solid var(--line);overflow:auto;flex-shrink:0}.wt-console nav button{font-size:12px;border:0;background:transparent;white-space:nowrap;padding:7px 14px}.wt-console nav button.active{background:#ebe3d5;color:#745022;font-weight:650}
    .wt-content{padding:18px 24px 20px;overflow:auto;flex:1;min-height:0}.wt-card{border:1px solid var(--line);background:white;border-radius:15px;padding:16px;margin-bottom:14px}.wt-card.warm{background:#f0ece3}
    .wt-controls{margin-top:14px;display:flex;gap:10px;flex-wrap:wrap}.wt-controls button{min-width:100px}.wt-stats{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:12px}.wt-stat{padding:14px 16px}.wt-stat:first-child{background:var(--cream);border-color:#f0dfbe}.wt-number{font-size:29px;font-weight:750;line-height:1.5;font-variant-numeric:tabular-nums}.wt-label{color:var(--muted);font-size:11px}
    .wt-progress-grid{display:grid;grid-template-columns:1fr 1fr;gap:24px;margin-top:12px}.wt-track{height:6px;border-radius:10px;background:#eee9df;overflow:hidden;margin-top:7px}.wt-track span{display:block;height:100%;background:linear-gradient(90deg,#d9b574,#edcc87);border-radius:10px}
    .wt-speeds{display:grid;grid-template-columns:repeat(3,1fr);gap:10px;margin-top:12px}.wt-console .wt-speed{text-align:left;background:#f8f7f4;padding:10px 14px}.wt-console .wt-speed.active{border-color:#d9af6f;background:#fff4df}.wt-speed strong{font-size:13px;display:block}.wt-speed span{font-size:11px;color:var(--muted)}
    .wt-log{display:flex;gap:12px;padding:10px 0;border-bottom:1px solid #f1eee8;align-items:center}.wt-log:last-child{border:0}.wt-log-icon{background:#fff2d8;border-radius:12px;width:40px;height:40px;flex-shrink:0;display:grid;place-items:center;font-size:20px}.wt-log-main{flex:1;min-width:0}.wt-log strong{font-size:13px;font-weight:600}.wt-rarity{font-size:10px;background:#f1e9f6;color:#8b60a0;border-radius:6px;padding:2px 7px}
    .wt-setting{padding:13px 0;display:flex;gap:20px;justify-content:space-between;align-items:center;border-bottom:1px solid #f2efe9}.wt-setting:last-child{border:0}.wt-console input[type=checkbox]{appearance:none;flex-shrink:0;width:38px;height:22px;background:#d7d3ca;border:0;border-radius:15px;position:relative;cursor:pointer}.wt-console input[type=checkbox]:after{content:'';display:block;background:white;box-shadow:0 1px 3px #0002;width:16px;height:16px;border-radius:50%;position:absolute;top:3px;left:3px;transition:transform .15s}.wt-console input[type=checkbox]:checked{background:#b8965a}.wt-console input[type=checkbox]:checked:after{transform:translateX(16px)}
    .wt-console input[type=number]{font:inherit;color:var(--ink);width:112px;border:1px solid var(--line);padding:6px 10px;border-radius:8px;background:#faf9f7}.wt-chart{display:flex;gap:14px;align-items:flex-end;height:180px;margin:20px 0 8px}.wt-column{flex:1;min-width:0;text-align:center;font-size:11px;color:var(--muted)}.wt-bar{margin:5px auto;background:#ddc292;border-radius:5px 5px 2px 2px;width:70%}.wt-empty{padding:28px 10px;text-align:center;color:var(--muted);font-size:12px}
    .wt-console footer{padding:10px 24px;border-top:1px solid var(--line);display:flex;justify-content:space-between;align-items:center;gap:12px;font-size:11px;color:var(--muted);background:#faf9f6;flex-shrink:0}.wt-console footer button{font-size:11px;padding:4px 10px;min-height:28px}.wt-notice{font-size:12px;color:#9c6c32;padding:6px 0;min-height:20px}
    .wt-versions{margin-top:10px;border-top:1px solid var(--line)}
    .wt-version{display:flex;align-items:center;gap:12px;padding:11px 0;border-bottom:1px solid #f2efe9}
    .wt-version:last-child{border:0}
    .wt-version>div{flex:1;min-width:0;display:flex;flex-direction:column;gap:1px}
    .wt-version strong{font-size:13px;font-weight:650}
    .wt-version-value{font-size:13px;font-weight:650;font-variant-numeric:tabular-nums;color:#745022;white-space:nowrap}
    .wt-version button{font-size:11px;padding:4px 12px;min-height:28px;white-space:nowrap}
    .wt-gate article p{font-size:13px}
    .wt-mod-pack{border:1px solid #efe9df;border-radius:13px;padding:12px 14px;margin-top:12px;background:#fdfcfa}
    .wt-mod-pack strong{font-size:13px;font-weight:650}
    .wt-console .wt-mod-remove{font-size:11px;padding:4px 10px;min-height:28px}
    .wt-mod-grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(78px,1fr));gap:10px;margin-top:12px}
    .wt-mod-item{text-align:center;min-width:0}
    .wt-mod-thumb{width:100%;aspect-ratio:1;object-fit:contain;background:#f4f1ea;border:1px solid #ece6da;border-radius:10px;display:block}
    .wt-mod-item span{display:block;font-size:11px;margin-top:5px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
    .wt-mod-type{font-style:normal;font-size:10px;color:var(--muted)}
    .wt-gate{position:fixed;inset:0;z-index:2147483500;background:#f7f4ee;display:grid;place-items:center;padding:16px;overflow:auto}.wt-gate article{max-width:480px;width:100%;padding:24px;background:white;border:1px solid var(--line);border-radius:18px;box-shadow:0 10px 30px #0001}.wt-gate h2{font-size:22px}.wt-gate p{margin:12px 0;font-size:13px}.wt-gate .wt-controls{display:flex;flex-wrap:wrap;gap:8px}
    @media(max-width:680px){.wt-stats{grid-template-columns:repeat(2,1fr)}.wt-content{padding:14px}.wt-console header{padding:16px 16px 10px}.wt-console nav{padding:0 14px 10px}.wt-console footer{padding:10px 14px}.wt-progress-grid{gap:14px}.wt-speeds{gap:6px}.wt-console .wt-speed{padding:8px}}
    `;
    doc.head.append(style);
  }
  function el(tag,text,parent,cls){const n=doc.createElement(tag);if(text!==undefined)n.textContent=text;if(cls)n.className=cls;parent?.append(n);return n;}
  function button(text,parent,action,cls){const n=el('button',text,parent,cls);n.type='button';n.onclick=action;return n;}
  return {el,button};
}

function createAssistantUpdateGate({window,result,retry,install,quit}){
  const {el,button}=assistantUi(window),root=el('section',undefined,window.document.body,'wt-gate');
  root.setAttribute('role','alertdialog');root.setAttribute('aria-modal','true');root.tabIndex=-1;
  const card=el('article',undefined,root),title=el('h2','需要更新',card),message=el('p','',card);
  el('p','新版本来自 Averionx/WuwaTappo-FullAtlas。更新会保留账号、图鉴缓存与设置；确认后自动下载、校验、备份并重启。',card,'wt-muted');
  const actions=el('div',undefined,card,'wt-controls'),status=el('p','',card,'wt-notice');
  let busy=false;
  const update=button('一键更新',actions,async()=>{
    if(busy)return;busy=true;update.disabled=true;status.textContent='正在下载并校验，请稍候…';
    try{const r=await install();status.textContent=r.message;}catch(e){status.textContent=e.message;busy=false;update.disabled=false;}
  },'wt-primary');
  button('重试检测',actions,async()=>{try{const r=await retry();status.textContent=r.message;}catch(e){status.textContent=e.message;}});
  button('退出桌宠',actions,()=>quit().catch(e=>{status.textContent=e.message;}));
  const apply=r=>{title.textContent='请更新至 '+r.version;message.textContent=r.message;};apply(result);root.focus();
  return {update:apply};
}

function createAssistantDashboard(d){
  const {window}=d,{el,button}=assistantUi(window),doc=window.document;
  let closed=false,timer=null,officialTimer=null,refreshing=false,tab='overview',value=null,renderView=()=>{},pending=0,saveQueue=Promise.resolve();
  const root=el('section',undefined,doc.body,'wt-console');root.tabIndex=-1;root.setAttribute('role','dialog');root.setAttribute('aria-label','桌宠控制台');
  const header=el('header',undefined,root);
  const eyebrow=el('div',undefined,header,'wt-row wt-between');el('div','WUWA TAPPO / 后台助手',eyebrow,'wt-eyebrow');
  button('×',eyebrow,close,'wt-close').setAttribute('aria-label','关闭控制台，后台继续运行');
  const heading=el('div',undefined,header,'wt-row wt-between');el('h2','桌宠控制台',heading);const badge=el('span','读取中',heading,'wt-badge paused');
  const nav=el('nav',undefined,root),content=el('main',undefined,root,'wt-content'),footer=el('footer',undefined,root);
  el('span','关闭面板后，后台任务继续运行',footer);const version=button('v'+d.version+' · 检查更新',footer,()=>showTab('settings'));
  // 底栏版本号统一取本仓库发布号：Windows 侧的 d.version 是上游 MOD 版本（1.2），
  // 和设置页「全图鉴版本」不是同一个号，直接显示会和 Mac 对不上。
  d.atlas.request('/community/version',{}).then(r=>{if(r&&r.current)version.textContent='v'+r.current+' · 检查更新';}).catch(()=>{});
  const notice=el('div','',header,'wt-notice');
  const tabs=[['overview','概览'],['rewards','开箱记录'],['daily','每日统计'],['rooms','房间历史'],['mods','Mod 管理'],['settings','设置与更新']],navButtons={};
  for(const [key,label]of tabs)navButtons[key]=button(label,nav,()=>showTab(key));
  const fmt=n=>Number(n||0).toLocaleString('zh-CN');
  function close(){if(closed)return;closed=true;window.clearInterval(timer);window.clearInterval(officialTimer);root.remove();d.onClose?.();}
  function checkOfficialNotice(){d.atlas.request('/official/update-check',{}).then(r=>{if(r.updateAvailable)notice.textContent='检测到官方 '+r.latestVersion+'，请打开「设置与更新」查看更新，或到官方桌宠页下载原版。';}).catch(()=>{});}
  async function change(changes){
    pending++;
    saveQueue=saveQueue.catch(()=>{}).then(async()=>{try{value=await d.configure(changes);notice.textContent='设置已保存';}catch(e){notice.textContent='保存失败：'+e.message;}finally{pending--;if(!closed)paint();}});
    await saveQueue;
  }
  function card(parent=content,cls=''){return el('section',undefined,parent,'wt-card '+cls);}
  function setBadge(){const r=value?.runtime||{},on=value?.settings?.enabled;
    const chest=!!value?.settings?.auto_chest;
    badge.textContent=value?.update_required?'等待更新':on?(r.quiet===false?(chest?'点击避让 · 开箱值守':'输入中 · 点击暂停'):'运行中'):(chest?(r.logged_in?'开箱值守':'开箱待登录'):'已暂停');
    badge.className='wt-badge'+((!on&&!chest)||value?.update_required?' paused':'');}
  function logRows(parent,items){parent.replaceChildren();if(!items.length){el('div','暂无真实开箱记录。原版账号成功领奖后会自动记录。',parent,'wt-empty');return;}
    for(const item of items){
      const row=el('div',undefined,parent,'wt-log');el('span','♧',row,'wt-log-icon');const text=el('div',undefined,row,'wt-log-main');
      el('strong',item.name||item.asset_name,text);el('div',new Date(item.time*1000).toLocaleString('zh-CN',{hour12:false})+' · '+(item.side==='left'?'宝箱1':'宝箱2')+' · '+(item.source==='automatic'?'自动领取':'手动领取')+' · 消耗 '+fmt(item.cost),text,'wt-muted');
      el('span',item.rarity||'奖励',row,'wt-rarity');
    }
  }
  function showTab(key){
    tab=key;content.replaceChildren();notice.textContent='';for(const[k,n]of Object.entries(navButtons))n.className=k===key?'active':'';
    if(key==='overview'){
      const hero=card(content,'warm');el('h3','后台陪伴，安心打字',hero);const description=el('p','',hero,'wt-muted');
      const controls=el('div',undefined,hero,'wt-controls');button('启动后台',controls,()=>change({enabled:true}),'wt-primary');button('暂停点击',controls,()=>change({enabled:false}));button('停止全部自动任务',controls,()=>change({enabled:false,auto_chest:false}));
      const stats=el('div',undefined,content,'wt-stats'),numbers=[];
      for(const label of ['可用点数','累计点数','本次发送','本次开箱']){const c=card(stats,'wt-stat');el('div',label,c,'wt-label');numbers.push(el('div','—',c,'wt-number'));el('div',label==='可用点数'?'原版账号余额':label==='累计点数'?'原版累计数据':label==='本次发送'?'关闭窗口不会清零':'真实成功领取',c,'wt-label');}
      const chests=card();el('h3','宝箱进度',chests);const grid=el('div',undefined,chests,'wt-progress-grid'),progress={};
      for(const side of ['left','right']){const c=el('div',undefined,grid),r=el('div',undefined,c,'wt-row wt-between');el('span',side==='left'?'宝箱1':'宝箱2',r);const text=el('span','—',r,'wt-muted'),track=el('div',undefined,c,'wt-track'),fill=el('span','',track);progress[side]={text,fill};}
      const hint=el('p','',chests,'wt-muted');hint.style.marginTop='10px';
      const speeds=card();el('h3','点击速度',speeds);const speedButtons={},speedGrid=el('div',undefined,speeds,'wt-speeds');
      for(const[n,label]of [[1,'轻缓'],[5,'标准'],[10,'快速']]){const b=button('',speedGrid,()=>change({speed:n}),'wt-speed');el('strong',label+' · '+n+' 次/秒',b);el('span','键盘或鼠标操作时自动暂停',b);speedButtons[n]=b;}
      const recent=card(),r=el('div',undefined,recent,'wt-row wt-between');el('h3','最近获得',r);button('查看全部',r,()=>showTab('rewards'));const logs=el('div',undefined,recent);
      renderView=()=>{const r=value.runtime||{},s=value.settings;
        description.textContent=(r.local_atlas?'全图鉴 + Mod 已开启':'官方模式（账号背包）')+' · 点数与开箱始终使用同一个真实账号';
        [r.balance,r.total,value.session,value.session_chests].forEach((n,i)=>numbers[i].textContent=n===undefined?'—':fmt(n));
        for(const side of ['left','right']){const c=r.chests?.[side],ms=c?.remaining||0;progress[side].text.textContent=!r.logged_in?'需要登录':c?.ready?'可开启':Math.floor(ms/60000)+':'+String(Math.ceil(ms/1000)%60).padStart(2,'0');progress[side].fill.style.width=(c?.ready?100:Math.max(0,100*(1-ms/(r.chests?.cooldown||1800000))))+'%';}
        hint.textContent=(s.auto_chest?'自动开箱已开启':'自动开箱已关闭')+' · 余额至少 '+fmt(Math.max(s.threshold,1000))+'，按钮出现约 2 秒后领取';
        for(const[n,b]of Object.entries(speedButtons))b.className='wt-speed'+(Number(n)===s.speed?' active':'');logRows(logs,(value.rewards||[]).slice(0,3));};
    }else if(key==='rewards'){
      const c=card();el('h3','真实开箱获得记录',c);el('p','两种图鉴模式共用记录，仅记录原版服务器成功返回的奖励。最近保留 1,000 条。',c,'wt-muted');const list=el('div',undefined,c);renderView=()=>logRows(list,value.rewards||[]);
    }else if(key==='daily'){
      const c=card();el('h3','最近 7 天',c);el('p','根据原版累计点击数的增量统计；切换本地图鉴不会切换账本。首次读取仅建立基线。',c,'wt-muted');const summary=el('div','',c),chart=el('div',undefined,c,'wt-chart');
      const reset=button('清零本次发送与开箱次数',c,async()=>{try{await d.reset();await refresh();}catch(e){notice.textContent=e.message;}});
      const legacy=el('p','',c,'wt-muted');legacy.style.marginTop='14px';
      renderView=()=>{const week=value.weekly?.normal||[],max=Math.max(1,...week.map(x=>x.count));summary.textContent='今日 '+fmt(week.at(-1)?.count)+' · 昨日 '+fmt(week.at(-2)?.count);chart.replaceChildren();for(const day of week){const col=el('div',undefined,chart,'wt-column');el('div',fmt(day.count),col);const bar=el('div','',col,'wt-bar');bar.style.height=Math.max(3,120*day.count/max)+'px';el('div',day.date.slice(5),col);}legacy.textContent=value.legacy_local?.total?'旧版模拟点数 '+fmt(value.legacy_local.total)+' 已单独存档，不计入真实账号与此统计。':'';};
    }else if(key==='rooms'){
      const c=card();el('h3','去过的房间',c);el('p','成功进入后记录房间号与房主，收藏后置顶。本地全图鉴开启时不能进入联机。',c,'wt-muted');const list=el('div',undefined,c);
      renderView=()=>{list.replaceChildren();if(!value.rooms?.length)el('div','还没有房间历史',list,'wt-empty');for(const room of value.rooms||[]){const row=el('div',undefined,list,'wt-log');button(room.favorite?'★':'☆',row,async()=>{try{await d.favorite(room.code,!room.favorite);await refresh();}catch(e){notice.textContent=e.message;}});const join=button(room.code+' · '+(room.host||'未知房主'),row,()=>d.join(room.code).catch(e=>{notice.textContent=e.message;}));join.disabled=d.atlas.enabled.value||value.update_required;button('删除记录',row,async()=>{try{await d.deleteRoom(room.code);await refresh();}catch(e){notice.textContent=e.message;}});}};
    }else if(key==='mods'){
      const hero=card(content,'warm');el('h3','外观模式');const modeText=el('p','正在读取外观模式…',hero,'wt-muted');
      const modeActions=el('div',undefined,hero,'wt-controls');
      const toOfficial=button('官方（只用账号背包）',modeActions,()=>switchMode(false)),toCustom=button('全图鉴 + Mod',modeActions,()=>switchMode(true),'wt-primary');
      const tools=card();el('h3','已安装的资源包',tools);const summary=el('p','正在读取…',tools,'wt-muted');
      const actions=el('div',undefined,tools,'wt-controls');
      const importPack=button('导入 Mod ZIP',actions,importArchive,'wt-primary');button('重新扫描',actions,()=>refreshPacks());
      const list=el('div',undefined,tools);
      async function switchMode(on){
        toOfficial.disabled=toCustom.disabled=true;
        try{await d.atlas.setEnabled(on);notice.textContent=on?'已切换到全图鉴 + Mod':'已切换到官方模式';}
        catch(e){notice.textContent='切换失败：'+e.message;}
        finally{paintMode();}
      }
      function paintMode(){
        const on=d.atlas.enabled.value;
        modeText.textContent=on?'当前：全图鉴 + Mod —— 本机资源包会出现在背包和图鉴里':'当前：官方模式 —— 背包只显示账号已拥有的物品，本机 Mod 不参与';
        toOfficial.disabled=d.atlas.busy.value||!on;
        toCustom.disabled=d.atlas.busy.value||on||!d.atlas.cacheReady.value;
      }
      async function importArchive(){
        importPack.disabled=true;
        try{
          const path=await window.__TAURI_INTERNALS__.invoke('plugin:dialog|open',{options:{multiple:false,directory:false,filters:[{name:'Mod 资源包',extensions:['zip']}]}});
          if(!path)return;
          const result=await d.atlas.importCustomResources(path);
          notice.textContent='已导入 '+result.name+' · '+result.item_count+' 项';
          await refreshPacks();
        }catch(e){notice.textContent='导入失败：'+e.message;}
        finally{importPack.disabled=false;}
      }
      async function refreshPacks(){
        try{
          await d.atlas.prepare().catch(()=>{});
          const r=await d.atlas.request('/custom/status');
          const packs=r.packs||[];
          list.replaceChildren();
          summary.textContent=packs.length?('共 '+packs.length+' 个包 · '+r.atlas_count+' 项 · '+r.asset_count+' 张图片'):'还没有导入资源包。没有 Mod 时「全图鉴 + Mod」等同于原有全图鉴。';
          if(r.error)el('p','Mod 叠加失败，已回退到官方图鉴：'+r.error,list,'wt-muted');
          if(!packs.length)el('div','点上方「导入 Mod ZIP」选择资源包；格式见 macos/docs/MODS.md',list,'wt-empty');
          for(const pack of packs){
            const box=el('div',undefined,list,'wt-mod-pack'),head=el('div',undefined,box,'wt-row wt-between');
            const title=el('div',undefined,head);el('strong',pack.name,title);
            el('div',(pack.item_count||0)+' 项 · '+(pack.asset_count||0)+' 张图片'+(pack.region&&pack.region!=='ANY'?' · '+pack.region:''),title,'wt-muted');
            button('移除',head,async()=>{try{await d.atlas.removeCustomResource(pack.id);notice.textContent='已移除 '+pack.name;await refreshPacks();}catch(e){notice.textContent='移除失败：'+e.message;}},'wt-mod-remove');
            const grid=el('div',undefined,box,'wt-mod-grid');
            for(const item of pack.items||[]){
              const cell=el('div',undefined,grid,'wt-mod-item'),src=d.atlas.assetUrl(item.icon);
              if(src){const img=el('img',undefined,cell,'wt-mod-thumb');img.src=src;img.alt=item.name;img.loading='lazy';}
              else el('div',undefined,cell,'wt-mod-thumb');
              el('span',item.name,cell);el('em',item.type==='character'?'角色':item.type==='accessory'?'挂饰':'表情',cell,'wt-mod-type');
            }
          }
          paintMode();
        }catch(e){notice.textContent='读取 Mod 列表失败：'+e.message;}
      }
      paintMode();refreshPacks();
      renderView=paintMode;
    }else{
      const c=card();el('h3','自动化设置',c);const fields={};
      for(const[key,title,detail]of [['enabled','后台自动点击','使用桌宠原版计数；打字、Shift 和鼠标拖拽时暂停'],['auto_chest','自动开宝箱','不等待键鼠空闲；沿用原版登录、冷却、余额与服务器奖励'],['remember_monitor','记住显示器','下次启动恢复；找不到原显示器时保持原位']]){
        const label=el('label',undefined,c,'wt-setting'),t=el('div',undefined,label);el('h3',title,t);el('p',detail,t,'wt-muted');const input=el('input',undefined,label);input.type='checkbox';input.setAttribute('aria-label',title);input.onchange=()=>change({[key]:input.checked});fields[key]=input;
      }
      const row=el('label',undefined,c,'wt-setting');const t=el('div',undefined,row);el('h3','自动开箱余额阈值',t);el('p','仍需达到原版宝箱实际费用（当前至少 1,000）',t,'wt-muted');const limit=el('input',undefined,row);limit.type='number';limit.min=1;limit.max=1000000000;limit.onchange=()=>change({threshold:Number(limit.value)});fields.threshold=limit;
      const u=card();el('h3','版本与更新',u);
      const versions=el('div',undefined,u,'wt-versions');
      const baseRow=el('div',undefined,versions,'wt-version'),baseText=el('div',undefined,baseRow);
      el('strong','当前版本（官方基底）',baseText);el('span','Kuro 官方客户端版本',baseText,'wt-muted');
      const baseValue=el('b','检测中…',baseRow,'wt-version-value');
      const modRow=el('div',undefined,versions,'wt-version'),modText=el('div',undefined,modRow);
      el('strong','全图鉴版本',modText);el('span','本整合包的发布版本',modText,'wt-muted');
      const modValue=el('b','检测中…',modRow,'wt-version-value');
      const check=button('检查更新',modRow,checkCommunity);
      const preRow=el('label',undefined,u,'wt-setting');const preText=el('div',undefined,preRow);
      el('h3','包含预发布版本',preText);el('p','关闭时只提醒正式版；打开后 alpha/beta 也算更新',preText,'wt-muted');
      const preBox=el('input',undefined,preRow);preBox.type='checkbox';preBox.setAttribute('aria-label','包含预发布版本');preBox.onchange=()=>change({prerelease_updates:preBox.checked});fields.prerelease_updates=preBox;
      const updates=el('p','',u,'wt-notice');updates.style.margin='6px 0 0';
      const actions=el('div',undefined,u,'wt-controls');
      button('打开 GitHub Releases',actions,openReleases);
      button('打开官方桌宠下载页',actions,openPetSite,'wt-primary');
      d.atlas.request('/community/version',{}).then(r=>{if(r.current)modValue.textContent=r.current;}).catch(()=>{modValue.textContent=d.version||'未知';});
      d.atlas.request('/official/update-check',{}).then(paintOfficial).catch(()=>{baseValue.textContent='未知';updates.textContent='无法读取官方基底版本；不影响全图鉴版本检查。';});
      function paintOfficial(r){
        const remote=r.latestVersion||r.remote;
        baseValue.textContent=r.currentVersion||r.local||'未知';
        const newer=r.updateAvailable===undefined?!!r.newer:!!r.updateAvailable;
        if(newer)updates.textContent='官方已发布 '+remote+'，当前基底 '+baseValue.textContent+'。';
        else updates.textContent=remote?'官方基底已是最新（'+remote+'）。':'暂时无法确认官方最新版本。';
      }
      async function openReleases(){
        try{const r=await d.atlas.request('/official/releases',{});if(r&&r.ok===false)throw new Error(r.error||'打开失败');}
        catch(e){updates.textContent='打开 Releases 失败：'+e.message;}
      }
      async function openPetSite(){
        updates.textContent='正在打开官方桌宠下载页…';
        try{const r=await d.atlas.request('/official/pet-site',{});if(r&&r.ok===false)throw new Error(r.error||'打开失败');updates.textContent='已打开官方桌宠下载页：'+(r.url||'官方站点');}
        catch(e){updates.textContent='打开失败：'+e.message;}
      }
      async function checkCommunity(){
        check.disabled=true;updates.textContent='正在对比 GitHub 最新发布…';
        try{
          const r=await d.atlas.request('/community/update-check',{});
          if(r.current)modValue.textContent=r.current;
          if(r.updateAvailable){updates.textContent='发现新版本 '+r.latest+'（当前 '+r.current+'）。';confirmUpdate(r);}
          else updates.textContent=r.latest?'已是最新版本 '+r.current:'GitHub 上还没有已发布的安装包。';
        }catch(e){updates.textContent='检查更新失败：'+e.message;}
        finally{check.disabled=false;}
      }
      function confirmUpdate(r){
        const gate=el('section',undefined,doc.body,'wt-gate');gate.setAttribute('role','alertdialog');gate.setAttribute('aria-modal','true');
        const box=el('article',undefined,gate);el('h2','发现新版本',box);
        el('p','当前全图鉴版本 '+r.current+'，GitHub 最新发布 '+r.latest+'。',box);
        el('p','更新会保留账号、图鉴缓存与已导入的 Mod 资源包。',box,'wt-muted');
        const row=el('div',undefined,box,'wt-controls');
        button('打开下载页',row,()=>{gate.remove();openReleases();},'wt-primary');
        button('稍后',row,()=>gate.remove());
      }
      const sync=card();el('h3','本地图鉴资源',sync);el('p','本地开关只影响物品展示与本机装扮，登录、点数、开箱和版本检查仍使用原账号。',sync,'wt-muted');button('检查并同步官方图鉴',sync,async()=>{try{const r=await d.atlas.request('/sync',{force:true});notice.textContent=r.status==='running'?'官方图鉴资源正在后台同步':'同步请求已提交';}catch(e){notice.textContent=e.message;}});
      const custom=card();el('h3','外观模式与 Mod',custom);
      el('p','「全图鉴 + Mod」使用完整离线图鉴并叠加本机导入的角色、挂饰和表情；官方模式只用账号背包，不会显示任何本机资源。',custom,'wt-muted');
      el('p','导入、移除与缩略图预览都在「Mod 管理」里。ZIP 只需名称、类型和图片路径，ID 由应用管理。',custom,'wt-muted');
      button('打开 Mod 管理',custom,()=>showTab('mods'));
      renderView=()=>{for(const[k,input]of Object.entries(fields)){if(doc.activeElement===input)continue;if(input.type==='checkbox')input.checked=!!value.settings[k];else input.value=value.settings[k];}};
    }
    if(value)paint();
  }
  function paint(){if(!value||closed||pending)return;setBadge();renderView();}
  async function refresh(){if(closed||refreshing||pending)return;refreshing=true;try{const next=await d.getState();if(!pending){value=next;paint();}}catch(e){notice.textContent='连接失败：'+e.message;}finally{refreshing=false;}}
  showTab(tabs.some(([key])=>key===d.initialTab)?d.initialTab:'overview');refresh();checkOfficialNotice();officialTimer=window.setInterval(checkOfficialNotice,8*3600*1000);timer=window.setInterval(refresh,2000);root.focus();
  return {close,focus:()=>root.focus(),showTab:key=>{if(tabs.some(([name])=>name===key))showTab(key);}};
}
