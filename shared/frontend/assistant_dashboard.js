/* DOM-only view; background ownership stays in the main pet renderer. */
function assistantUi(window) {
  const doc=window.document;
  if(!doc.getElementById('wt-assistant-style')){
    const style=doc.createElement('style');style.id='wt-assistant-style';
    style.textContent=`
    .wt-console,.wt-gate{--ink:#34322e;--muted:#8c857a;--line:#e9e4db;--gold:#b78643;--cream:#fff4dc;color:var(--ink);font:14px/1.6 "Segoe UI","Microsoft YaHei",sans-serif;text-align:left;letter-spacing:0;box-sizing:border-box;user-select:text}
    .wt-console *,.wt-gate *{box-sizing:border-box}
    .wt-console{position:fixed;inset:10px;z-index:2147483000;background:#f7f6f2;border:1px solid #e5dfd5;border-radius:20px;box-shadow:0 12px 50px #34261038;display:flex;flex-direction:column;overflow:hidden}
    .wt-console[data-embedded=true]{position:static;inset:auto;z-index:auto;background:transparent;border:0;border-radius:0;box-shadow:none;height:100%;min-height:0}
    .wt-console[data-embedded=true] header{padding:0 0 8px}.wt-console[data-embedded=true] nav{padding:0 0 10px}.wt-console[data-embedded=true] .wt-content{padding:12px 2px 16px}
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
    .wt-setting{padding:13px 0;display:flex;gap:20px;justify-content:space-between;align-items:center;border-bottom:1px solid #f2efe9}.wt-setting:last-child{border:0}
    .wt-slots{display:flex;flex-wrap:wrap;gap:6px;justify-content:flex-end;max-width:340px}.wt-console .wt-slot{font-size:12px;padding:5px 11px;min-height:30px;border-radius:9px;background:#f8f7f4}.wt-console .wt-slot.active{background:#efd8ac;border-color:#d9af6f;color:#654921;font-weight:650}
    .wt-console input[type=checkbox]{appearance:none;flex-shrink:0;width:38px;height:22px;background:#d7d3ca;border:0;border-radius:15px;position:relative;cursor:pointer}.wt-console input[type=checkbox]:after{content:'';display:block;background:white;box-shadow:0 1px 3px #0002;width:16px;height:16px;border-radius:50%;position:absolute;top:3px;left:3px;transition:transform .15s}.wt-console input[type=checkbox]:checked{background:#b8965a}.wt-console input[type=checkbox]:checked:after{transform:translateX(16px)}
    .wt-console input[type=number]{font:inherit;color:var(--ink);width:112px;border:1px solid var(--line);padding:6px 10px;border-radius:8px;background:#faf9f7}.wt-chart{display:flex;gap:14px;align-items:flex-end;height:180px;margin:20px 0 8px}.wt-column{flex:1;min-width:0;text-align:center;font-size:11px;color:var(--muted)}.wt-bar{margin:5px auto;background:#ddc292;border-radius:5px 5px 2px 2px;width:70%}.wt-empty{padding:28px 10px;text-align:center;color:var(--muted);font-size:12px}
    .wt-console footer{padding:10px 24px;border-top:1px solid var(--line);display:flex;justify-content:space-between;align-items:center;gap:12px;font-size:11px;color:var(--muted);background:#faf9f6;flex-shrink:0}.wt-console footer button{font-size:11px;padding:4px 10px;min-height:28px}.wt-notice{font-size:12px;color:#9c6c32;padding:6px 0;min-height:20px}
    .wt-transform-grid{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:10px 14px;margin-top:12px}.wt-transform-field{display:grid;gap:5px;font-size:12px;color:var(--muted)}.wt-transform-field input{width:100%}
    .wt-synth-log{margin-top:10px;border-top:1px solid var(--line);padding-top:8px}
    .wt-console input[type=search]{font:inherit;color:var(--ink);flex:1;min-width:0;border:1px solid var(--line);padding:7px 12px;border-radius:9px;background:#faf9f7}
    .wt-versions{margin-top:10px;border-top:1px solid var(--line)}
    .wt-notes{margin-top:12px;border-top:1px solid var(--line);padding-top:12px}
    .wt-notes h3{margin-bottom:6px}
    .wt-notes-body{white-space:pre-line;font-size:12px;line-height:1.75;color:#6f675c;max-height:210px;overflow:auto;padding-right:4px}
    .wt-notes-body::-webkit-scrollbar{width:7px}.wt-notes-body::-webkit-scrollbar-thumb{background:#ded4c2;border-radius:8px}
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
    .wt-pills{display:flex;flex-wrap:wrap;gap:6px;margin-top:12px}
    .wt-pill{font-size:11px;padding:2px 10px;border-radius:999px;background:#fffdf8;border:1px solid #e6dcc8;color:#745022}
    .wt-inline-pair{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:12px;padding:12px 0;border-bottom:1px solid #f2efe9}
    .wt-inline-pair:last-child{border:0}
    .wt-inline-field{display:flex;flex-direction:column;gap:4px;min-width:0}
    .wt-inline-field strong{font-size:13px;font-weight:650}
    .wt-inline-field input[type=number]{width:100%}
    .wt-setting-block{padding:12px 0;border-bottom:1px solid #f2efe9}
    .wt-setting-block:last-child{border:0}
    .wt-setting-block>h3{margin-bottom:2px}
    @media(max-width:680px){.wt-stats{grid-template-columns:repeat(2,1fr)}.wt-content{padding:14px}.wt-console header{padding:16px 16px 10px}.wt-console nav{padding:0 14px 10px}.wt-console footer{padding:10px 14px}.wt-progress-grid{gap:14px}.wt-speeds{gap:6px}.wt-console .wt-speed{padding:8px}.wt-inline-pair{grid-template-columns:1fr}}
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
  el('p','新版本来自本项目 GitHub 发布。更新会保留账号、图鉴缓存与设置，支持自动下载、校验、备份并重启升级。',card,'wt-muted');
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
  let closed=false,timer=null,officialTimer=null,installTimer=null,refreshing=false,tab='overview',value=null,renderView=()=>{},pending=0,saveQueue=Promise.resolve();
  function withTimeout(promise,label,ms=10000){let timeout;return Promise.race([promise,new Promise((_,reject)=>{timeout=window.setTimeout(()=>reject(new Error(label+'超时，请检查网络后重试')),ms);})]).finally(()=>window.clearTimeout(timeout));}
  const root=el('section',undefined,d.mount||doc.body,'wt-console');root.tabIndex=-1;root.setAttribute('role','dialog');root.setAttribute('aria-label','桌宠控制台');
  // 挂进官方面板的「控制台」页时改成流式布局：外框、圆角和阴影都由面板提供。
  if(d.mount)root.setAttribute('data-embedded','true');
  // 面板外观（透明度/背景图）提升到整个控制台作用域：进任何页签前就先独立应用一次，
  // 进入「外观与 Mod」时再把滑杆与按钮接上，之后由这些控件实时同步。
  let captureButtons=null,opacityInput=null,clearBgBtn=null;
  function applyDashboardStyle(){
    const c=d.atlas.capture?.value||{};
    d.atlas.applyCaptureStyle?.();
    const mode=c.mode==='green'?'green':'off';
    if(captureButtons){captureButtons.off.className=mode==='off'?'wt-primary':'';captureButtons.green.className=mode==='green'?'wt-primary':'';}
    const panelOpacity=Math.max(.35,Math.min(1,Number(c.panelOpacity)||.96));
    if(opacityInput)opacityInput.value=panelOpacity;
    if(clearBgBtn)clearBgBtn.disabled=!c.panelBackground;
    if(d.mount){root.style.opacity='';root.style.backgroundImage='';}
    else{root.style.opacity=String(panelOpacity);root.style.backgroundImage=c.panelBackground?`url("${String(c.panelBackground).replace(/"/g,'%22')}")`:'';}
  }
  applyDashboardStyle();
  const header=el('header',undefined,root);
  const eyebrow=el('div',undefined,header,'wt-row wt-between');el('div','__WUWA_BRAND__',eyebrow,'wt-eyebrow');
  button('×',eyebrow,close,'wt-close').setAttribute('aria-label','关闭控制台，后台继续运行');
  const heading=el('div',undefined,header,'wt-row wt-between');el('h2','桌宠控制台',heading);const badge=el('span','读取中',heading,'wt-badge paused');
  const nav=el('nav',undefined,root),content=el('main',undefined,root,'wt-content'),footer=el('footer',undefined,root);
  el('span','关闭面板后，后台任务继续运行',footer);const version=button('v'+d.version+' · 检查更新',footer,()=>showTab('system'));
  // 底栏版本号统一取本仓库发布号：Windows 侧的 d.version 是上游 MOD 版本（1.2），
  // 和设置页「全图鉴版本」不是同一个号，直接显示会和 Mac 对不上。
  withTimeout(d.atlas.request('/community/version',{}),'本机版本读取').then(r=>{if(r&&r.current)version.textContent='v'+r.current+' · 检查更新';else version.textContent='版本未知 · 检查更新';}).catch(()=>{version.textContent='版本未知 · 检查更新';});
  const notice=el('div','',header,'wt-notice');
  const tabs=[
    ['overview','概览'],
    ['automation','自动助手'],
    ['records','统计与记录'],
    ['rooms','联机房间'],
    ['mods','外观与 Mod'],
    ['system','系统与更新'],
  ];
  const tabAliases={settings:'automation',rewards:'records',daily:'records',update:'system'};
  const resolveTab=key=>{const mapped=tabAliases[key]||key;return tabs.some(([name])=>name===mapped)?mapped:null;};
  const navButtons={};
  for(const [key,label]of tabs)navButtons[key]=button(label,nav,()=>showTab(key));
  const fmt=n=>Number(n||0).toLocaleString('zh-CN');
  function close(){if(closed)return;closed=true;window.clearInterval(timer);window.clearInterval(officialTimer);window.clearInterval(installTimer);root.remove();d.onClose?.();}
  function checkOfficialNotice(){withTimeout(d.atlas.request('/official/update-check',{}),'官方版本检查').then(r=>{if(r.updateAvailable)notice.textContent='检测到官方 '+r.latestVersion+'，请打开「系统与更新」查看，或到官方桌宠页下载原版。';}).catch(()=>{});}
  async function change(changes){
    pending++;
    saveQueue=saveQueue.catch(()=>{}).then(async()=>{try{value=await d.configure(changes);notice.textContent='设置已保存';}catch(e){notice.textContent='保存失败：'+e.message;}finally{pending--;if(!closed)paint();}});
    await saveQueue;
  }
  function card(parent=content,cls=''){return el('section',undefined,parent,'wt-card '+cls);}
  function setBadge(){const r=value?.runtime||{},on=value?.settings?.enabled;
    const chest=!!value?.settings?.auto_chest;
    if(value?.update_required){badge.textContent='等待更新';badge.className='wt-badge paused';return;}
    if(on&&r.balance_paused){badge.textContent='点数达上限 · 待机';badge.className='wt-badge paused';return;}
    badge.textContent=on?(r.quiet===false?(chest?'点击避让 · 开箱值守':'输入中 · 点击暂停'):'运行中'):(chest?(r.logged_in?'开箱值守':'开箱待登录'):'已暂停');
    badge.className='wt-badge'+((!on&&!chest)?' paused':'');}
  function logRows(parent,items){parent.replaceChildren();if(!items.length){el('div','暂无真实开箱记录。原版账号成功领奖后会自动记录。',parent,'wt-empty');return;}
    for(const item of items){
      const row=el('div',undefined,parent,'wt-log');el('span','♧',row,'wt-log-icon');const text=el('div',undefined,row,'wt-log-main');
      el('strong',item.name||item.asset_name,text);el('div',new Date(item.time*1000).toLocaleString('zh-CN',{hour12:false})+' · '+(item.side==='left'?'宝箱1':'宝箱2')+' · '+(item.source==='automatic'?'自动领取':'手动领取')+' · 消耗 '+fmt(item.cost),text,'wt-muted');
      el('span',item.rarity||'奖励',row,'wt-rarity');
    }
  }
  function showTab(rawKey){
    const key=resolveTab(rawKey)||'overview';
    captureButtons=null;opacityInput=null;clearBgBtn=null;
    tab=key;d.onTabChange?.(key);content.replaceChildren();notice.textContent='';for(const[k,n]of Object.entries(navButtons))n.className=k===key?'active':'';
    if(key==='overview'){
      const hero=card(content,'warm');el('h3','运行状态与快捷控制',hero);el('p','自动功能与官方账号共用同一个余额与冷却；关闭面板后后台继续运行',hero,'wt-muted');
      const controls=el('div',undefined,hero,'wt-controls');button('启动后台',controls,()=>change({enabled:true}),'wt-primary');button('暂停点击',controls,()=>change({enabled:false}));button('停止全部自动任务',controls,()=>change({enabled:false,auto_chest:false,auto_emote:false}));
      const pills=el('div',undefined,hero,'wt-pills'),speedPill=el('span','',pills,'wt-pill'),chestPill=el('span','',pills,'wt-pill'),emotePill=el('span','',pills,'wt-pill');
      const stats=el('div',undefined,content,'wt-stats'),numbers=[];
      for(const label of ['可用点数','累计点数','本次发送','本次开箱']){const c=card(stats,'wt-stat');el('div',label,c,'wt-label');numbers.push(el('div','—',c,'wt-number'));el('div',label==='可用点数'?'原版账号余额':label==='累计点数'?'原版累计数据':label==='本次发送'?'关闭窗口不会清零':'真实成功领取',c,'wt-label');}
      const chests=card(),chestHead=el('div',undefined,chests,'wt-row wt-between');el('h3','宝箱进度',chestHead);button('调整自动策略',chestHead,()=>showTab('automation'));const grid=el('div',undefined,chests,'wt-progress-grid'),progress={};
      for(const side of ['left','right']){const c=el('div',undefined,grid),r=el('div',undefined,c,'wt-row wt-between');el('span',side==='left'?'宝箱1':'宝箱2',r);const text=el('span','—',r,'wt-muted'),track=el('div',undefined,c,'wt-track'),fill=el('span','',track);progress[side]={text,fill};}
      const hint=el('p','',chests,'wt-muted');hint.style.marginTop='10px';
      const recent=card(),recentHead=el('div',undefined,recent,'wt-row wt-between');el('h3','最近获得',recentHead);button('查看全部',recentHead,()=>showTab('records'));const logs=el('div',undefined,recent);
      renderView=()=>{const r=value.runtime||{},s=value.settings;
        speedPill.textContent='点击速度：'+(s.speed||5)+' 次/秒';
        chestPill.textContent='自动开箱：'+(s.auto_chest?'开启':'关闭');
        emotePill.textContent='打字表情：'+(s.auto_emote?'开启':'关闭');
        [r.balance,r.total,value.session,value.session_chests].forEach((n,i)=>numbers[i].textContent=n===undefined?'—':fmt(n));
        for(const side of ['left','right']){const c=r.chests?.[side],ms=c?.remaining||0;progress[side].text.textContent=!r.logged_in?'需要登录':c?.ready?'可开启':Math.floor(ms/60000)+':'+String(Math.ceil(ms/1000)%60).padStart(2,'0');progress[side].fill.style.width=(c?.ready?100:Math.max(0,100*(1-ms/(r.chests?.cooldown||1800000))))+'%';}
        hint.textContent=(s.auto_chest?'自动开箱已开启':'自动开箱已关闭')+' · 余额至少 '+fmt(Math.max(s.threshold,1000))+'，按钮出现约 2 秒后领取';
        logRows(logs,(value.rewards||[]).slice(0,3));};
    }else if(key==='automation'){
      const searchCard=card();searchCard.setAttribute('data-search-card','true');
      const searchRow=el('div',undefined,searchCard,'wt-row'),searchInput=el('input',undefined,searchRow);
      searchInput.type='search';searchInput.setAttribute('data-settings-search','true');searchInput.setAttribute('aria-label','搜索自动化与设置');searchInput.setAttribute('placeholder','搜索点击速度、自动开箱、表情或批量合成…');
      const clearSearch=button('清空',searchRow,()=>{searchInput.value='';applySearch();searchInput.focus();});
      const searchEmpty=el('div','未找到匹配的自动化设置',searchCard,'wt-empty');searchEmpty.hidden=true;
      const settingCards=[],searchCards=[],settingRows=[],cardRows=new Map();
      function trackRow(c,row){settingRows.push(row);cardRows.set(c,[...(cardRows.get(c)||[]),row]);return row;}
      const clickCard=card();settingCards.push(clickCard);el('h3','自动点击与余额策略',clickCard);const fields={};
      function addToggle(parent,key,title,detail){
        const label=el('label',undefined,parent,'wt-setting'),t=el('div',undefined,label);el('h3',title,t);el('p',detail,t,'wt-muted');
        const input=el('input',undefined,label);input.type='checkbox';input.setAttribute('aria-label',title);input.onchange=()=>change({[key]:input.checked});fields[key]=input;trackRow(parent,label);return input;
      }
      function applySearch(){
        const raw=String(searchInput.value||'').trim();
        clearSearch.disabled=!raw;
        if(!raw){
          for(const item of settingRows)item.hidden=false;
          for(const c of settingCards)c.hidden=false;
          for(const item of searchCards)item.hidden=false;
          searchEmpty.hidden=true;return;
        }
        const q=raw.toLowerCase();let hits=0;
        for(const c of settingCards){
          let cardHits=0;
          for(const item of (cardRows.get(c)||[])){const hit=String(item.textContent||'').toLowerCase().includes(q);item.hidden=!hit;if(hit){hits++;cardHits++;}}
          c.hidden=cardHits===0;
        }
        for(const item of searchCards){const hit=String(item.textContent||'').toLowerCase().includes(q);item.hidden=!hit;if(hit)hits++;}
        searchEmpty.hidden=hits>0;
      }
      searchInput.oninput=applySearch;
      addToggle(clickCard,'enabled','后台自动点击','使用桌宠原版计数；打字、Shift 和鼠标拖拽时暂停');
      const speedBlock=el('div',undefined,clickCard,'wt-setting-block');trackRow(clickCard,speedBlock);el('h3','点击速度',speedBlock);el('p','按真实打字与点击频率选择合适的档位',speedBlock,'wt-muted');
      const speedButtons={},speedGrid=el('div',undefined,speedBlock,'wt-speeds');
      for(const[n,label]of [[1,'轻缓 · 1 次/秒'],[5,'标准 · 5 次/秒'],[10,'快速 · 10 次/秒']]){const b=button('',speedGrid,()=>change({speed:n}),'wt-speed');el('strong',label,b);el('span','键盘或鼠标操作时自动暂停',b);speedButtons[n]=b;}
      addToggle(clickCard,'auto_click_balance','按可用点数自动启停','开启后台自动点击后生效；高于上限暂停触发点击，低于下限自动恢复（不关闭后台点击总开关）');
      const pair=el('div',undefined,clickCard,'wt-setting-block wt-inline-pair');trackRow(clickCard,pair);
      for(const[key,title,detail]of [['auto_click_below','低于此可用点数时开启','低于下限后自动恢复点击'],['auto_click_above','高于此可用点数时关闭','高于上限后自动暂停点击']]){
        const field=el('label',undefined,pair,'wt-inline-field');el('strong',title,field);el('p',detail,field,'wt-muted');
        const input=el('input',undefined,field);input.type='number';input.min=0;input.max=1000000000;input.step=1;input.setAttribute('aria-label',title);input.onchange=()=>change({[key]:Number(input.value)});fields[key]=input;
      }
      const chestCard=card();settingCards.push(chestCard);el('h3','自动开箱',chestCard);
      addToggle(chestCard,'auto_chest','自动开宝箱','不等待键鼠空闲；沿用原版登录、冷却、余额与服务器奖励');
      const limitRow=el('label',undefined,chestCard,'wt-setting'),limitText=el('div',undefined,limitRow);trackRow(chestCard,limitRow);el('h3','自动开箱余额阈值',limitText);el('p','仍需达到原版宝箱实际费用（当前至少 1,000）',limitText,'wt-muted');const limit=el('input',undefined,limitRow);limit.type='number';limit.min=1;limit.max=1000000000;limit.onchange=()=>change({threshold:Number(limit.value)});fields.threshold=limit;
      const emoteCard=card();settingCards.push(emoteCard);el('h3','表情触发与快捷键',emoteCard);
      addToggle(emoteCard,'auto_emote','打字自动表情','先在背包装备表情；跟随真实打字与点击次数弹出（支持随机或固定间隔），自动避让后台连点，无需辅助功能权限');
      const freqRow=el('div',undefined,emoteCard,'wt-setting'),freqText=el('div',undefined,freqRow);trackRow(emoteCard,freqRow);
      el('h3','自动表情间隔',freqText);const freqSub=el('p','约每 X 次真实按键或点击随机触发一次（±40% 随机波动；后台自动点击不计入）',freqText,'wt-muted');
      const freqControls=el('div',undefined,freqRow,'wt-row');
      const randBtn=button('随机',freqControls,()=>change({auto_emote_random:value?.settings?.auto_emote_random===false}),'wt-slot');
      randBtn.setAttribute('data-emote-random','true');
      const freqInput=el('input',undefined,freqControls);freqInput.type='number';freqInput.min=1;freqInput.max=1000;freqInput.step=1;freqInput.setAttribute('aria-label','自动表情平均间隔');
      freqInput.onchange=()=>change({auto_emote_interval:Math.max(1,Math.min(1000,Math.round(Number(freqInput.value)||25)))});
      fields.auto_emote_interval=freqInput;
      const slotRow=el('div',undefined,emoteCard,'wt-setting'),slotText=el('div',undefined,slotRow);trackRow(emoteCard,slotRow);
      el('h3','自动表情槽位（1～5）',slotText);el('p','对应背包已装备的 1～5 号表情；点单个号码直接只发该表情，再点其他号码可多选随机',slotText,'wt-muted');
      const slotGroup=el('div',undefined,slotRow,'wt-slots');
      const allSlotBtn=button('全部',slotGroup,()=>{
        const cur=(Number(value?.settings?.auto_emote_slots)||31)&31||31;
        if(cur!==31)return change({auto_emote_slots:31});
      },'wt-slot');
      allSlotBtn.setAttribute('data-emote-slot','all');
      const slotButtons=[];
      for(let i=0;i<5;i++){
        const bit=1<<i;
        const btn=button(String(i+1),slotGroup,()=>{
          const cur=(Number(value?.settings?.auto_emote_slots)||31)&31||31;
          const next=cur===31?bit:(cur&bit?((cur&~bit)||31):(cur|bit));
          return change({auto_emote_slots:next});
        },'wt-slot');
        btn.setAttribute('data-emote-slot',String(i+1));
        slotButtons.push(btn);
      }
      addToggle(emoteCard,'emote_hotkeys','Alt+1～5 快捷表情','全局按 Alt+1～5 直接发送对应槽位表情');
      const emoteHint=el('p','',emoteCard,'wt-muted');emoteHint.setAttribute('data-emote-hint','true');
      const synthCard=card();searchCards.push(synthCard);el('h3','背包批量合成',synthCard);el('p','只消耗官方背包中数量大于 1 的多余副本（保留本体），不会提交本地或 Mod 外观。',synthCard,'wt-muted');
      const toolRow=el('div',undefined,synthCard,'wt-controls'),synthStatus=el('p','',synthCard,'wt-notice'),synthButtons=[];
      async function runSynthesis(kind,label){
        for(const b of synthButtons)b.disabled=true;
        synthStatus.textContent='正在按官方背包重复数量执行'+label+'…';
        try{
          const r=await d.atlas.synthesizeDuplicates?.(kind);
          if(!r){synthStatus.textContent='当前客户端暂不支持批量合成';return;}
          const batches=Number(r.batches)||0,groups=Array.isArray(r.groups)?r.groups.length:Number(r.groups)||0;
          synthStatus.textContent=r.count?('已消耗 '+r.count+' 个多余副本'+(batches?' · 提交 '+batches+' 批':'')+(groups?' · '+groups+' 组':'')+'，正在刷新官方背包'):'没有可合成的多余副本，请先刷新官方背包';
        }catch(e){synthStatus.textContent='合成失败：'+e.message;}
        finally{for(const b of synthButtons)b.disabled=false;}
      }
      synthButtons.push(button('批量同阶合成（5 合 1）',toolRow,()=>runSynthesis('same-level','同阶合成'),'wt-primary'));
      synthButtons.push(button('批量升阶合成（10 合 1）',toolRow,()=>runSynthesis('upgrade','升阶合成')));
      const synthLogBtn=button('查看合成日志',toolRow,()=>toggleSynthLog());synthLogBtn.setAttribute('data-synth-log-btn','true');
      const synthLog=el('div',undefined,synthCard,'wt-synth-log');synthLog.hidden=true;
      let synthLogOpen=false;
      function synthLogTime(value){
        if(value===undefined||value===null||value==='')return '时间未知';
        const n=Number(value),date=Number.isFinite(n)?new Date(n<1e12?n*1000:n):new Date(value);
        return isNaN(date.getTime())?'时间未知':date.toLocaleString('zh-CN',{hour12:false});
      }
      async function toggleSynthLog(){
        if(synthLogOpen){synthLogOpen=false;synthLog.hidden=true;return;}
        synthLogOpen=true;synthLog.hidden=false;synthLog.replaceChildren();
        el('div','正在读取合成日志…',synthLog,'wt-muted');
        try{
          const r=await d.atlas.request('/diagnostic/synth/list',{limit:20});
          const rows=Array.isArray(r?.entries)?r.entries:Array.isArray(r?.items)?r.items:Array.isArray(r?.logs)?r.logs:[];
          synthLog.replaceChildren();
          if(!rows.length){el('div','暂无合成日志',synthLog,'wt-empty');return;}
          for(const item of rows){
            const raw=item&&typeof item==='object'?item:{},entry=raw.entry&&typeof raw.entry==='object'?raw.entry:raw;
            const kindText=String(entry.kind||'').toLowerCase().includes('up')||entry.kind==='升阶'?'升阶':'同阶';
            const ok=!entry.error;
            const logRow=el('div',undefined,synthLog,'wt-log'),main=el('div',undefined,logRow,'wt-log-main');
            el('strong',kindText+'合成 · '+(ok?'成功':'失败'),main);
            let line=synthLogTime(raw.time??entry.time??entry.startedAt)+' · 消耗 '+(Number(entry.consumed)||0)+' 个';
            if(entry.batches!==undefined)line+=' · '+entry.batches+' 批';
            if(entry.durationMs!==undefined)line+=' · '+Math.round(Number(entry.durationMs)||0)+' ms';
            el('div',line,main,'wt-muted');
            if(entry.error)el('div',String(entry.error),main,'wt-muted');
            el('span',ok?'成功':'失败',logRow,'wt-rarity');
          }
        }catch(e){
          synthLog.replaceChildren();
          el('div','合成日志读取失败：'+e.message,synthLog,'wt-empty');
        }
      }
      renderView=()=>{
        applySearch();
        for(const[k,input]of Object.entries(fields)){if(doc.activeElement===input)continue;if(input.type==='checkbox'){input.checked=!!value.settings[k];}else input.value=value.settings[k]??(k==='auto_emote_interval'?25:'');}
        for(const[n,b]of Object.entries(speedButtons))b.className='wt-speed'+(Number(n)===value.settings.speed?' active':'');
        const stats=value?.runtime?.emote||d.atlas?.assistant?.emoteStats||{};
        const isRandom=value?.settings?.auto_emote_random!==false;
        randBtn.className='wt-slot'+(isRandom?' active':'');
        randBtn.setAttribute('aria-pressed',String(isRandom));
        freqSub.textContent=isRandom?'约每 X 次真实按键或点击随机触发一次（±40% 随机波动；后台自动点击不计入）':'每 X 次真实按键或点击固定触发一次（随机波动已关闭；后台自动点击不计入）';
        const mask=(Number(value?.settings?.auto_emote_slots)||31)&31||31;
        const hasSlots=Array.isArray(stats.slots)&&stats.slots.length>0;
        allSlotBtn.className='wt-slot'+(mask===31?' active':'');
        for(let i=0;i<5;i++)slotButtons[i].className='wt-slot'+((mask&(1<<i))?' active':'');
        const selectedEquipped=hasSlots?stats.slots.some((name,i)=>String(name||'').trim()&&(mask&(1<<i))):true;
        const remainPrefix=isRandom?' · 距下次约 ':' · 距下次 ';
        emoteHint.textContent=!value.settings.auto_emote?'打字自动表情已关闭'
          :(hasSlots&&!stats.slots.some(s=>String(s||'').trim()))||(!hasSlots&&stats.reason==='no-selection')?'请先在背包装备至少一个表情'
          :(hasSlots?!selectedEquipped:stats.reason==='slot-empty')?'所选槽位暂未装备表情，请换选已装备的槽位或点「全部」'
          :stats.sent?'本次已触发 '+stats.sent+' 次'+(stats.remaining?remainPrefix+stats.remaining+' 次点击/按键':'')
          :'待命中'+(stats.remaining?remainPrefix+stats.remaining+' 次点击/按键':'：跟随真实打字与点击次数触发');};
    }else if(key==='records'){
      const weekCard=card();el('h3','最近 7 天统计',weekCard);el('p','根据原版累计点击数的增量统计；本地资源不会切换账本。首次读取仅建立基线。',weekCard,'wt-muted');const summary=el('div','',weekCard),chart=el('div',undefined,weekCard,'wt-chart');
      const reset=button('清零本次发送与开箱次数',weekCard,async()=>{try{await d.reset();await refresh();}catch(e){notice.textContent=e.message;}});
      const legacy=el('p','',weekCard,'wt-muted');legacy.style.marginTop='14px';
      const rewardCard=card();el('h3','真实开箱获得记录',rewardCard);el('p','两种图鉴模式共用记录，仅记录原版服务器成功返回的奖励。最近保留 1,000 条。',rewardCard,'wt-muted');const list=el('div',undefined,rewardCard);
      renderView=()=>{const week=value.weekly?.normal||[],max=Math.max(1,...week.map(x=>x.count));summary.textContent='今日 '+fmt(week.at(-1)?.count)+' · 昨日 '+fmt(week.at(-2)?.count);chart.replaceChildren();for(const day of week){const col=el('div',undefined,chart,'wt-column');el('div',fmt(day.count),col);const bar=el('div','',col,'wt-bar');bar.style.height=Math.max(3,120*day.count/max)+'px';el('div',day.date.slice(5),col);}legacy.textContent=value.legacy_local?.total?'旧版模拟点数 '+fmt(value.legacy_local.total)+' 已单独存档，不计入真实账号与此统计。':'';logRows(list,value.rewards||[]);};
    }else if(key==='rooms'){
      const c=card();el('h3','去过的房间',c);el('p','成功进入后记录房间号与房主，收藏后置顶。官方联机始终可用；仅本地外观只在本机显示。',c,'wt-muted');const list=el('div',undefined,c);
      const layoutCard=card();el('h3','联机角色样式与布局',layoutCard);
      el('p','只调整本机联机房间内角色的显示大小、透明度与昵称比例，不写入账号。',layoutCard,'wt-muted');
      const memberBox=el('div',undefined,layoutCard),layoutControls={},layoutValues={};
      const roomFormat={scale:v=>Number(v).toFixed(2)+'x',opacity:v=>Math.round(Number(v)*100)+'%',nameScale:v=>Number(v).toFixed(2)+'x'};
      for(const [key,label,min,max,step] of [['scale','联机角色大小',0.3,2,0.05],['opacity','联机角色透明度',0.15,1,0.05],['nameScale','联机昵称大小',0.5,2,0.05]]){
        const row=el('label',undefined,layoutCard,'wt-setting'),text=el('div',undefined,row);el('h3',label,text);
        const head=el('div',undefined,row,'wt-row'),input=el('input',undefined,head);
        input.type='range';input.min=min;input.max=max;input.step=step;input.setAttribute('aria-label',label);input.style.flex='1';input.style.minWidth='120px';
        const valueText=el('b','',head,'wt-version-value');
        input.oninput=()=>{valueText.textContent=roomFormat[key](input.value);try{d.atlas.setRoomLayout?.({[key]:Number(input.value)});}catch(e){notice.textContent=e.message;}};
        layoutControls[key]=input;layoutValues[key]=valueText;
      }
      const layoutActions=el('div',undefined,layoutCard,'wt-controls');
      button('横向排列',layoutActions,()=>{try{d.atlas.requestRoomArrange?.('horizontal');paintRoom();}catch(e){notice.textContent=e.message;}});
      button('纵向排列',layoutActions,()=>{try{d.atlas.requestRoomArrange?.('vertical');paintRoom();}catch(e){notice.textContent=e.message;}});
      button('重置位置',layoutActions,()=>{try{d.atlas.clearRoomPositions?.();paintRoom();}catch(e){notice.textContent=e.message;}});
      let roomSignature=null;
      function paintRoom(){
        const members=(Array.isArray(d.atlas.panelState?.value?.roomMembers)?d.atlas.panelState.value.roomMembers:[]).filter(member=>member&&typeof member==='object');
        const signature=members.map(member=>String(member.id)+'|'+String(member.nickname||member.name||'')+'|'+(d.atlas.isRoomMemberHidden?.(member.id)?1:0)).join('~');
        if(signature!==roomSignature){
          roomSignature=signature;memberBox.replaceChildren();
          if(!members.length)el('div','当前未在联机房间或暂未同步成员',memberBox,'wt-empty');
          for(const member of members){
            const row=el('div',undefined,memberBox,'wt-log'),text=el('div',undefined,row,'wt-log-main');
            el('strong',String(member.nickname||member.name||member.id||'未知成员'),text);
            el('div','ID '+String(member.id??'—'),text,'wt-muted');
            const hidden=!!d.atlas.isRoomMemberHidden?.(member.id);
            button(hidden?'显示':'隐藏',row,()=>{try{d.atlas.toggleRoomMemberHidden?.(member.id);roomSignature=null;paintRoom();}catch(e){notice.textContent=e.message;}});
          }
        }
        const layout=d.atlas.roomLayout?.value||{};
        for(const [key,input] of Object.entries(layoutControls)){
          const next=Number(layout[key]),val=Number.isFinite(next)?next:1;
          input.value=val;if(layoutValues[key])layoutValues[key].textContent=roomFormat[key](val);
        }
      }
      paintRoom();
      renderView=()=>{list.replaceChildren();if(!value.rooms?.length)el('div','还没有房间历史',list,'wt-empty');for(const room of value.rooms||[]){const row=el('div',undefined,list,'wt-log');button(room.favorite?'★':'☆',row,async()=>{try{await d.favorite(room.code,!room.favorite);await refresh();}catch(e){notice.textContent=e.message;}});const join=button(room.code+' · '+(room.host||'未知房主'),row,()=>d.join(room.code).catch(e=>{notice.textContent=e.message;}));join.disabled=value.update_required;button('删除记录',row,async()=>{try{await d.deleteRoom(room.code);await refresh();}catch(e){notice.textContent=e.message;}});}paintRoom();};
    }else if(key==='mods'){
      const xform=card();el('h3','挂饰位置 / 大小 / 角度',xform);
      el('p','只修改本机显示，不影响账号联机外观。选择本地挂饰后拖动滑杆即可。',xform,'wt-muted');
      const activeLabel=el('p','',xform,'wt-muted');
      let editId=null;
      const xformControls={},xformValues={};const xformRow=el('div',undefined,xform,'wt-transform-grid');
      const formatTransform=(key,raw)=>{const n=Number(raw)||0;
        if(key==='scale')return n.toFixed(2)+'x';
        if(key==='rotate')return (n>0?'+':'')+Math.round(n)+'°';
        return (n>0?'+':'')+Math.round(n)+'px';};
      for(const [key,label,min,max,step] of [['dx','水平位置',-400,400,1],['dy','垂直位置',-400,400,1],['scale','大小',0.3,3,0.01],['rotate','角度',-180,180,1]]){
        const field=el('label',undefined,xformRow,'wt-transform-field');
        const head=el('div',undefined,field,'wt-row wt-between');el('span',label,head);
        const valueText=el('b','',head,'wt-version-value');
        const input=el('input',undefined,field);input.type='range';input.min=min;input.max=max;input.step=step;
        input.oninput=()=>{try{d.atlas.setAccessoryTransform?.({[key]:Number(input.value)},editId||undefined);}catch(e){notice.textContent=e.message;}valueText.textContent=formatTransform(key,input.value);};
        xformControls[key]=input;xformValues[key]=valueText;
      }
      const xformActions=el('div',undefined,xform,'wt-controls');
      button('恢复默认',xformActions,()=>{try{d.atlas.resetAccessoryTransform?.(editId||undefined);paintTransform();paintSaved();}catch(e){notice.textContent=e.message;}});
      button('跟随当前挂饰',xformActions,()=>{editId=null;paintTransform();paintSaved();});
      const savedSection=el('div',undefined,xform,'wt-notes');el('h3','已保存的挂饰微调',savedSection);const savedList=el('div',undefined,savedSection);
      const savedRows=()=>d.atlas.listAccessoryTransforms?.()||[];
      function accessoryName(item){return item?String(item.asset_name||item.name||item.label||item.id||''):'';}
      function paintTransform(){
        const active=d.atlas.selection?.value?.accessory||d.atlas.officialUser?.value?.accessory||null;
        const rows=savedRows();
        const editRow=editId?rows.find(r=>String(r.id)===String(editId)):null;
        const activeId=active&&active.id!==undefined?String(active.id):'';
        const activeRow=activeId?rows.find(r=>String(r.id)===activeId):null;
        const target=editId?editRow:(activeRow||d.atlas.accessoryTransform?.(null)||null);
        const row=target?{dx:Number(target.dx)||0,dy:Number(target.dy)||0,scale:Number(target.scale)||1,rotate:Number(target.rotate)||0}:{dx:0,dy:0,scale:1,rotate:0};
        for(const key of Object.keys(xformControls)){xformControls[key].value=row[key];if(xformValues[key])xformValues[key].textContent=formatTransform(key,row[key]);}
        const name=accessoryName(active)||'未选择挂饰';
        activeLabel.textContent='当前挂饰：'+name+(editId?' · 编辑中：'+((editRow&&editRow.name)||('#'+editId)):'');
      }
      let savedSignature=null;
      function paintSaved(){
        const rows=savedRows();
        const signature=rows.map(row=>[row.id,row.name,row.dx,row.dy,row.scale,row.rotate].join(':')).join('|');
        if(signature===savedSignature)return;
        savedSignature=signature;savedList.replaceChildren();
        if(!rows.length){el('div','还没有保存过挂饰微调',savedList,'wt-empty');return;}
        for(const row of rows){
          const item=el('div',undefined,savedList,'wt-version'),text=el('div',undefined,item);
          el('strong',row.name||('#'+row.id),text);
          el('div','水平 '+formatTransform('dx',row.dx)+' · 垂直 '+formatTransform('dy',row.dy)+' · 大小 '+formatTransform('scale',row.scale)+' · 角度 '+formatTransform('rotate',row.rotate),text,'wt-muted');
          button('加载编辑',item,()=>{editId=String(row.id);paintTransform();paintSaved();notice.textContent='正在编辑 '+(row.name||('#'+row.id));});
          button('删除',item,()=>{try{d.atlas.deleteAccessoryTransform?.(row.id);if(String(editId)===String(row.id))editId=null;notice.textContent='已删除 '+(row.name||('#'+row.id));savedSignature=null;paintTransform();paintSaved();}catch(e){notice.textContent=e.message;}});
        }
      }
      const tools=card();el('h3','已安装的资源包',tools);const summary=el('p','正在读取…',tools,'wt-muted');
      const actions=el('div',undefined,tools,'wt-controls');
      const importPack=button('导入 Mod ZIP',actions,importArchive,'wt-primary');button('重新扫描',actions,()=>refreshPacks());
      const list=el('div',undefined,tools);
      function paintMode(){paintTransform();paintSaved();}
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
          summary.textContent=packs.length?('共 '+packs.length+' 个包 · '+r.atlas_count+' 项 · '+r.asset_count+' 张图片'):'还没有导入资源包；本地资源为空时直接使用官方图鉴。';
          if(r.error)el('p','Mod 叠加失败，已回退到官方图鉴：'+r.error,list,'wt-muted');
          if(!packs.length)el('div','点上方「导入 Mod ZIP」选择资源包；格式见 docs/MODS.md',list,'wt-empty');
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
      const captureCard=card();el('h3','OBS 桌宠绿幕',captureCard);el('p','仅在桌宠周围绘制绿幕背景，方便 OBS 色键抠像，不影响桌面其他区域。',captureCard,'wt-muted');
      const captureRow=el('div',undefined,captureCard,'wt-controls'),cb={};
      for(const [mode,label] of [['off','关闭'],['green','开启绿幕']])cb[mode]=button(label,captureRow,()=>{d.atlas.setCapture?.({mode});applyDashboardStyle();});
      captureButtons=cb;
      const lookCard=card();el('h3','面板外观',lookCard);el('p','面板背景图与透明度只保存在本机，会实时作用于控制台与官方面板。',lookCard,'wt-muted');
      const bgRow=el('div',undefined,lookCard,'wt-setting'),bgText=el('div',undefined,bgRow);el('h3','面板背景图',bgText);el('p','选择一张本地图片作为控制台与面板背景',bgText,'wt-muted');
      const bgControls=el('div',undefined,bgRow,'wt-row'),bgInput=el('input',undefined,bgControls);bgInput.type='file';bgInput.accept='image/*';bgInput.setAttribute('aria-label','面板背景图');
      const clearBg=button('清除背景图',bgControls,()=>{try{d.atlas.setCapture?.({panelBackground:''});bgInput.value='';applyDashboardStyle();}catch(e){notice.textContent=e.message;}});
      bgInput.onchange=()=>{const file=bgInput.files?.[0];if(!file)return;if(file.size>2*1024*1024){notice.textContent='背景图片过大，请选择 2 MB 以内的图片';return;}const reader=new FileReader();reader.onload=()=>{try{d.atlas.setCapture?.({panelBackground:String(reader.result||'')});applyDashboardStyle();}catch(e){notice.textContent=e.message;}};reader.readAsDataURL(file);};
      const opacityRow=el('label',undefined,lookCard,'wt-setting'),opacityText=el('div',undefined,opacityRow);el('h3','面板透明度',opacityText);
      const opacity=el('input',undefined,opacityRow);opacity.type='range';opacity.min=.35;opacity.max=1;opacity.step=.01;opacity.setAttribute('aria-label','面板透明度');opacity.oninput=()=>{d.atlas.setCapture?.({panelOpacity:Number(opacity.value)});applyDashboardStyle();};
      opacityInput=opacity;clearBgBtn=clearBg;
      applyDashboardStyle();
      renderView=()=>{applyDashboardStyle();paintMode();};
    }else if(key==='system'){
      const perfCard=card();
      el('h3','性能与省内存渲染',perfCard);
      el('p','只影响本机桌面渲染；低配或长期挂机时更省电、省内存。',perfCard,'wt-muted');
      const perfButtons={},perfFpsRow=el('div',undefined,perfCard,'wt-controls');
      for(const [fpsCap,label] of [[30,'30 FPS（省电）'],[60,'60 FPS（均衡）'],[0,'跟随屏幕']]){
        perfButtons[fpsCap]=button(label,perfFpsRow,()=>{try{d.atlas.setPerf?.({fpsCap});paintPerf();}catch(e){notice.textContent=e.message;}});
      }
      const perfFields={};
      for(const [name,title,detail] of [['idlePause','空闲停帧','动作静止且菜单收起时暂停桌面动画循环，显著降低闲置功耗'],['reduceMotion','精简动画效果','跳过气泡/窗口过渡动画，减少高频重绘'],['hideMainPet','停止渲染桌面主角','隐藏主桌宠图层但保留后台点击与助手统计，大幅节省内存']]){
        const row=el('label',undefined,perfCard,'wt-setting'),text=el('div',undefined,row);el('h3',title,text);el('p',detail,text,'wt-muted');
        const input=el('input',undefined,row);input.type='checkbox';input.setAttribute('data-perf-setting','true');input.setAttribute('aria-label',title);
        input.onchange=()=>{try{d.atlas.setPerf?.({[name]:input.checked});paintPerf();}catch(e){notice.textContent=e.message;}};
        perfFields[name]=input;
      }
      function paintPerf(){
        const perf=d.atlas.perf?.value||{};
        const cap=[0,30,60].includes(Number(perf.fpsCap))?Number(perf.fpsCap):60;
        for(const [fps,btn] of Object.entries(perfButtons))btn.className=Number(fps)===cap?'wt-primary':'';
        for(const [name,input] of Object.entries(perfFields)){if(doc.activeElement===input)continue;input.checked=perf[name]===undefined?(name==='idlePause'):!!perf[name];}
      }
      paintPerf();
      const u=card();el('h3','版本与更新',u);const fields={};
      const versions=el('div',undefined,u,'wt-versions');
      const baseRow=el('div',undefined,versions,'wt-version'),baseText=el('div',undefined,baseRow);
      el('strong','当前版本（官方基底）',baseText);el('span','Kuro 官方客户端版本',baseText,'wt-muted');
      const baseValue=el('b','检测中…',baseRow,'wt-version-value');
      const modRow=el('div',undefined,versions,'wt-version'),modText=el('div',undefined,modRow);
      el('strong','当前安装版本',modText);el('span','本整合包已安装的发布版本',modText,'wt-muted');
      const modValue=el('b','检测中…',modRow,'wt-version-value');
      const remoteRow=el('div',undefined,versions,'wt-version'),remoteText=el('div',undefined,remoteRow);
      el('strong','GitHub 最新修改版',remoteText);el('span','本整合包能升级到的版本',remoteText,'wt-muted');
      const remoteValue=el('b','检测中…',remoteRow,'wt-version-value');
      const preRow=el('label',undefined,u,'wt-setting');const preText=el('div',undefined,preRow);
      el('h3','包含预发布版本',preText);el('p','关闭时只提醒正式版；打开后 alpha/beta 也算更新',preText,'wt-muted');
      const preBox=el('input',undefined,preRow);preBox.type='checkbox';preBox.setAttribute('aria-label','包含预发布版本');preBox.onchange=async()=>{await change({prerelease_updates:preBox.checked});installChecked=false;await checkCommunity();};fields.prerelease_updates=preBox;
      const updates=el('p','',u,'wt-notice');updates.style.margin='6px 0 0';
      // 下载/校验/解压/替换的实时进度：桌宠还活着时由本机服务写，重启后照样能读回来。
      const progress=el('div',undefined,u,'wt-progress');progress.hidden=true;
      const progressText=el('p','',progress,'wt-notice');
      const track=el('div',undefined,progress,'wt-track'),fill=el('span',undefined,track);
      const installActions=el('div',undefined,progress,'wt-controls');installActions.hidden=true;installActions.style.marginTop='8px';
      const installFallback=button('前往 GitHub 手动下载',installActions,openReleases);installFallback.setAttribute('data-install-github-fallback','true');
      progress.style.margin='4px 0 10px';
      const notes=el('section',undefined,u,'wt-notes');notes.hidden=true;
      el('h3','这次更新了什么',notes);
      const notesBody=el('div','',notes,'wt-notes-body');
      const actions=el('div',undefined,u,'wt-controls');
      button('打开 GitHub Releases',actions,openReleases);
      button('打开官方桌宠下载页',actions,openPetSite,'wt-primary');
      const check=button('检查更新',actions,checkCommunity);
      // 拿不到发布号就说「未知」，不要一直挂着「检测中」——那看着像还在等网络。
      withTimeout(d.atlas.request('/community/version',{}),'本机版本读取').then(r=>{modValue.textContent=r.current||'未知';}).catch(()=>{modValue.textContent=d.version||'未知';});
      withTimeout(d.atlas.request('/official/update-check',{}),'官方版本检查').then(paintOfficial).catch(e=>{baseValue.textContent='未知';updates.textContent='官方版本检查失败：'+e.message+'；不影响本机资源同步。';});
      // 官方清单偶尔会落后于我们已整合的基底（CDN 缓存或发布节奏差），
      // 这时说「已是最新（旧号）」会让人以为检查坏了，改成把两个号都摆出来。
      function versionParts(value){return String(value||'').split(/[.+-]/).map(n=>parseInt(n,10)||0).slice(0,3);}
      function compareVersion(a,b){const x=versionParts(a),y=versionParts(b);for(let i=0;i<3;i++){if(x[i]!==y[i])return x[i]-y[i];}return 0;}
      function paintOfficial(r){
        const remote=r.latestVersion||r.remote||'';
        const current=r.currentVersion||r.local||'未知';
        baseValue.textContent=current;
        const newer=r.updateAvailable===undefined?!!r.newer:!!r.updateAvailable;
        const older=r.remoteOlder===undefined?(!!remote&&compareVersion(remote,current)<0):!!r.remoteOlder;
        if(newer)updates.textContent='官方已发布 '+remote+'，当前基底 '+current+'。';
        else if(older)updates.textContent='官方清单当前是 '+remote+'，本包基底已跟进到 '+current+'。';
        else updates.textContent=remote?('官方基底已是最新（'+current+'）。'):'暂时无法确认官方最新版本。';
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
        check.disabled=true;
        try{
          updates.textContent='正在对比 GitHub 最新发布…';
          const r=await withTimeout(d.atlas.request('/community/update-check',{force:true}),'GitHub 版本检查');
          if(r.current)modValue.textContent=r.current;
          if(r.error){
            remoteValue.textContent='检查失败';
            updates.textContent=r.message||('检查更新失败：'+r.error);
          }else if(r.latest){
            remoteValue.textContent=r.latest;
            if(r.updateAvailable){updates.textContent='发现新版本 '+r.latest+'（当前 '+r.current+'）。';confirmUpdate(r);}
            else updates.textContent='已是最新版本 '+r.current;
          }else{
            remoteValue.textContent='暂无发布';
            updates.textContent=r.message||'GitHub 上还没有已发布的安装包。';
          }
          // 检查更新的响应里也带说明正文（Windows 是 /update/status，macOS 与社区检查走 update-check），
          // 两种命名都认，免得两端各写一套字段名。
          paintNotes(r.release_notes||r.releaseNotes);
        }catch(e){updates.textContent='检查更新失败：'+e.message;}
        finally{check.disabled=false;}
      }
      function paintNotes(text){
        const body=String(text||'').trim();
        if(!body||body===notesBody.textContent){return;}
        notesBody.textContent=body;notes.hidden=false;
      }
      function paintInstall(v){
        if(!v||!v.phase){progress.hidden=true;installActions.hidden=true;return;}
        progress.hidden=false;
        const failed=v.phase==='failed';
        const pct=failed?0:Math.max(0,Math.min(100,Number(v.percent)||0));
        fill.style.width=pct+'%';
        progressText.textContent=failed?(v.message||'自动更新失败，可前往 GitHub 手动下载覆盖安装。'):((v.message||'正在更新…')+(v.phase==='download'&&pct<=2?'':' · '+pct.toFixed(0)+'%'));
        installActions.hidden=!failed;
        if(v.phase==='complete'||failed){window.clearInterval(installTimer);installTimer=null;}
      }
      let installChecked=false;
      async function pollInstall(){
        if(closed||tab!=='system'){window.clearInterval(installTimer);installTimer=null;return;}
        try{
          const r=await d.atlas.request('/update/status',{});
          paintInstall(r.installation);
          if(r.github_version){remoteValue.textContent=r.github_version;modValue.textContent=r.current||modValue.textContent;}
          else if(r.error){
            if(remoteValue.textContent==='检测中…'||remoteValue.textContent==='未检查')remoteValue.textContent='检查失败';
            if(!updates.textContent||updates.textContent==='正在对比 GitHub 最新发布…')updates.textContent=r.message||('检查更新失败：'+r.error);
          }else if(r.checked_at){
            if(remoteValue.textContent==='检测中…'||remoteValue.textContent==='未检查')remoteValue.textContent='暂无发布';
          }else if(remoteValue.textContent==='检测中…')remoteValue.textContent='未检查';
          paintNotes(r.release_notes);
          // 打开更新页时补一次联网检查，否则 release_notes 和远端版本号都是空的。
          if(!r.checked_at&&!installChecked){
            installChecked=true;
            d.atlas.request('/update/check',{}).then(pollInstall).catch(()=>{
              if(remoteValue.textContent==='检测中…')remoteValue.textContent='未检查';
            });
          }
        }catch{}
      }
      function confirmUpdate(r){
        const gate=el('section',undefined,doc.body,'wt-gate');gate.setAttribute('role','alertdialog');gate.setAttribute('aria-modal','true');
        const box=el('article',undefined,gate);el('h2','发现新版本',box);
        el('p','当前整合版 '+r.current+'，GitHub 最新发布 '+r.latest+'。',box);
        el('p','更新会保留账号、图鉴缓存与已导入的 Mod 资源包。',box,'wt-muted');
        const row=el('div',undefined,box,'wt-controls'),status=el('p','',box,'wt-notice');
        const github=()=>{gate.remove();openReleases();};
        // 只有服务端明确说这个发布带原地升级包时才给「一键更新」；
        // 其余情况（macOS 版、发布里没 zip、下载校验失败）都退回跳 GitHub 下载安装包。
        if(r.installable){
          const install=button('一键更新',row,async()=>{
            install.disabled=true;status.textContent='正在启动更新任务…';
            try{
              const res=await d.atlas.request('/update/install',{});
              gate.remove();
              if(tab!=='system')showTab('system');
              paintInstall({phase:'download',percent:2,message:res.message||'已开始后台下载并校验安装包…'});
              window.clearInterval(installTimer);
              installTimer=window.setInterval(pollInstall,800);
              pollInstall();
            }catch(e){
              if(String(e?.message||'').includes('更新正在进行中')){
                gate.remove();
                if(tab!=='system')showTab('system');
                window.clearInterval(installTimer);
                installTimer=window.setInterval(pollInstall,800);
                pollInstall();
                return;
              }
              status.textContent='程序内更新失败：'+e.message+'。可以改为下载安装包覆盖安装。';
              install.disabled=false;
              if(!row.querySelector('[data-github-fallback]')){const fallback=button('前往 GitHub 下载',row,github);fallback.setAttribute('data-github-fallback','true');}
            }
          },'wt-primary');
          button('前往 GitHub 下载',row,github);
        }else{
          el('p',r.installableReason||'当前系统或这个发布不支持程序内更新，将打开 GitHub Releases 下载安装包。',box,'wt-muted');
          button('前往 GitHub 下载',row,github,'wt-primary');
        }
        button('稍后',row,()=>gate.remove());
      }
      const sync=card();el('h3','本机素材资源',sync);el('p','本地素材只影响本机显示与装扮；官方背包、联机、登录、点数、开箱和版本检查仍使用原账号。',sync,'wt-muted');button('检查并同步素材缓存',sync,async()=>{try{const r=await d.atlas.request('/sync',{force:true});notice.textContent=r.status==='running'?'素材缓存正在后台同步':'同步请求已提交';}catch(e){notice.textContent=e.message;}});
      renderView=()=>{
        paintPerf();
        if(doc.activeElement!==preBox&&value?.settings)preBox.checked=!!value.settings.prerelease_updates;
        pollInstall();
        if(!installTimer)installTimer=window.setInterval(pollInstall,2000);
      };
    }
    if(value)paint();
  }
  function paint(){if(!value||closed||pending)return;setBadge();renderView();}
  async function refresh(){if(closed||refreshing||pending)return;refreshing=true;try{const next=await d.getState();if(!pending){value=next;paint();}}catch(e){notice.textContent='连接失败：'+e.message;}finally{refreshing=false;}}
  showTab(resolveTab(d.initialTab)||'overview');refresh();checkOfficialNotice();officialTimer=window.setInterval(checkOfficialNotice,8*3600*1000);timer=window.setInterval(refresh,2000);root.focus();
  return {close,focus:()=>root.focus(),showTab:key=>{const next=resolveTab(key);if(next)showTab(next);},currentTab:()=>tab};
}
