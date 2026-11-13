/* Main-renderer driver. No signal enters the macOS input queue. */
function resolveBalanceAutoClick(settings,balance) {
  if(!settings.auto_click_balance||!Number.isFinite(balance))return null;
  if(balance<settings.auto_click_below)return true;
  if(balance>settings.auto_click_above)return false;
  return null;
}

function createDesktopAssistant({ref, window, atlas, displayName=(type,name)=>name}) {
  const VERSION='1.3.1', updateRequired=ref(false);
  const state=ref({settings:{"enabled":false,"speed":5,"auto_chest":false,"threshold":1000,"auto_click_balance":false,"auto_click_below":10000,"auto_click_above":40000,"remember_monitor":true,"prerelease_updates":false,"auto_emote":false}});
  const request=(action,body={})=>atlas.request('/assistant/'+action,body);
  const ready=atlas.ready.then(()=>request('state')).then(v=>{state.value=v;updateRequired.value=!!v.update_required;});
  let started=false,disposed=false,timer=null,mainDependencies=null,sent=0,confirmed=0,lastObserve=0;
  let dashboard=null,updateDialog=null,autoSide=null,autoUntil=0,flushing=null;
  const readyButtons=new Map(),retryAfter=new Map();
  // 打字自动表情（跟上游 1.3.4）：只认真实物理按键，虚拟补点（F24）不会触发。
  // 焦点判定放在宿主侧 —— Win 看前台窗口的输入焦点，mac 走 AX 焦点 ——
  // 这样用 WASD 走路不会弹表情，只有真的在输入文字才弹。
  const emoteStats={physical:0,eligible:0,sent:0,reason:'waiting-input'};
  const physicalHeld=new Set();
  let emoteBusy=false,nextEmoteAt=0,typedKeys=0,lastTypedAt=0;
  const TYPING_KEYS=new Set(['Space','Spacebar',' ','Comma','Dot','Period','Semicolon','Quote','Slash',
    'Backslash','Minus','Equal','LeftBracket','RightBracket',',','.',';',"'",'/','\\','-','=','[',']','`']);
  const MODIFIER=/control|ctrl|alt|option|meta|win|super|command|shift/i;
  function physicalRelease(key){physicalHeld.delete(String(key));}
  function typingKey(key){
    // mac 的键名是小写字母/单个符号，Windows 给的是 KeyA/Digit1 这类码名，两边都收。
    return /^[a-z0-9]$/i.test(key) || /^Key[A-Z]$/i.test(key) || /^Digit[0-9]$/.test(key) || TYPING_KEYS.has(key);
  }
  async function physicalKey(raw){
    const key=String(raw),now=Date.now();
    if(physicalHeld.has(key))return;
    physicalHeld.add(key);
    emoteStats.physical++;
    if(!started||disposed||!state.value.settings.auto_emote||updateRequired.value||mainDependencies?.blocked?.())return;
    if([...physicalHeld].some(k=>MODIFIER.test(k)))return;
    if(!typingKey(key))return;
    emoteStats.eligible++;
    if(now-lastTypedAt>2000)typedKeys=0;
    lastTypedAt=now;typedKeys++;
    if(typedKeys<2||now<nextEmoteAt||emoteBusy)return;
    if(!(mainDependencies.emotes?.()||[]).some(x=>x.id&&x.src)){emoteStats.reason='no-selection';return;}
    emoteBusy=true;
    try{
      const context=await request('typing-context');
      emoteStats.reason=context.editable?'ready'
        :context.reason==='accessibility-denied'?'no-access'
        :context.reason==='disabled'?'disabled':'not-editable';
      if(!context.editable){nextEmoteAt=Date.now()+500;return;}
      // 焦点判定和真正发出去之间有几十毫秒：这期间停了手、关了开关或锁了更新，就放弃这次。
      if(disposed||!state.value.settings.auto_emote||updateRequired.value||mainDependencies.blocked()||Date.now()-lastTypedAt>500)return;
      const current=(mainDependencies.emotes?.()||[]).filter(x=>x.id&&x.src);
      if(!current.length)return;
      mainDependencies.sendEmote(current[Math.floor(Math.random()*current.length)]);
      emoteStats.sent++;emoteStats.reason='sent';typedKeys=0;
      nextEmoteAt=Date.now()+1200+Math.floor(Math.random()*1200);
    }catch(error){emoteStats.reason='error';console.warn('[assistant] typing emote skipped',error);}
    finally{emoteBusy=false;}
  }
  const OUTBOX='wuwatappo-confirmed-rewards-v2';
  let outbox=[];
  try{outbox=JSON.parse(window.localStorage?.getItem(OUTBOX)||'[]');if(!Array.isArray(outbox))outbox=[];}catch{}
  function saveOutbox(){try{window.localStorage?.setItem(OUTBOX,JSON.stringify(outbox));}catch{}}
  async function drainRewards(){
    while(outbox.length){
      try{await request('reward',{reward:outbox[0]});outbox.shift();saveOutbox();}
      catch(error){state.value={...state.value,history_error:'奖励记录待补存：'+error.message};return;}
    }
  }
  function beginChest(side){
    const source=autoSide===side&&Date.now()<autoUntil?'automatic':'manual';autoSide=null;
    return {id:window.crypto?.randomUUID?.()||Date.now()+'-'+Math.random().toString(36).slice(2),side,source};
  }
  function chestSucceeded(attempt,result){
    try{
      if(!result?.asset_name||!result.asset_type||!(Number(result.click_consume)>0))return;
      const reward={...attempt,confirmed:true,asset_id:result.asset_id,asset_type:result.asset_type,
        asset_name:String(result.asset_name),name:displayName(result.asset_type,result.asset_name),
        rarity:String(result.rarity_type||''),cost:Number(result.click_consume)};
      outbox.push(reward);saveOutbox();
    }catch(error){console.warn('[assistant] reward history pending',error);}
  }
  function observation(d){return {total:Number(d.total()),balance:Number(d.balance()),account:d.account(),
    logged_in:!!d.loggedIn(),chests:d.chests?.()||{}};}
  async function flush(){
    if(!mainDependencies)return;
    if(flushing)return flushing;
    flushing=(async()=>{
      await mainDependencies.persistOriginal?.();
      const body={...observation(mainDependencies),flush:true,sent,confirmed};sent=confirmed=0;
      await request('observe',body);await drainRewards();
    })().finally(()=>{flushing=null;});
    return flushing;
  }
  async function flushBeforeSwitch(){
    if(started)return flush();
    const value=await request('flush-request'),generation=value.flush_generation;
    for(let i=0;i<20;i++){
      await new Promise(resolve=>window.setTimeout(resolve,100));
      if((await request('state')).flush_done>=generation)return;
    }
  }
  function autoChest(d){
    const settings=state.value.settings;
    // Physical activity pauses virtual tapping only. This is an in-renderer
    // button click, so it needs no mouse movement, focus or OS input event.
    if(!settings.auto_chest||!d.loggedIn()||updateRequired.value||d.blocked()){readyButtons.clear();return;}
    for(const side of ['left','right']){
      const button=window.document.querySelector('.reward-anchor--'+side+' button.reward-btn'),now=Date.now();
      if(!button||button.disabled||!button.isConnected||button.getClientRects().length===0||
        Number(d.balance())<Math.max(settings.threshold,d.chestCost)){readyButtons.delete(side);continue;}
      if(!readyButtons.has(side))readyButtons.set(side,now);
      if(now-readyButtons.get(side)<2000||now<(retryAfter.get(side)||0))continue;
      retryAfter.set(side,now+30000);readyButtons.delete(side);autoSide=side;autoUntil=now+1000;
      button.click();break;
    }
  }
  function showRequiredUpdate(result){
    updateRequired.value=result.required===true;
    if(!updateRequired.value)return;
    if(updateDialog){updateDialog.update(result);return;}
    updateDialog=createAssistantUpdateGate({window,result,
      retry:async()=>{const r=await atlas.request('/update/check',{});showRequiredUpdate(r);return r;},
      install:async()=>atlas.request('/update/install',{}),
      quit:async()=>{if(started){await flush();await mainDependencies.quit();}else await request('quit');}});
  }
  async function startMain(d){
    if(started)return;started=true;mainDependencies=d;
    await ready;state.value=await request('renderer-start');
    async function loop(){
      if(disposed)return;
      let delay=1000;
      try{
        const autoClick=resolveBalanceAutoClick(state.value.settings,Number(d.balance()));
        if(autoClick!==null&&autoClick!==state.value.settings.enabled)
          state.value=await request('configure',{settings:{enabled:autoClick}});
        const now=Date.now(),body={sent,confirmed};sent=confirmed=0;
        if(now-lastObserve>=1000){Object.assign(body,observation(d));lastObserve=now;await drainRewards();}
        const result=await request('tick',body);state.value={...state.value,...result};
        if(result.update_required&&!updateRequired.value)showRequiredUpdate(await atlas.request('/update/status',{}));
        if(result.flush_generation){await flush();await request('flush-ack',{generation:result.flush_generation});}
        if(result.quit){await flush();await d.quit();return;}
        if(typeof result.open_settings==='string')await d.openView?.(result.open_settings);
        else if(result.open_settings)await d.openSettings?.();
        if(result.settings.enabled&&!updateRequired.value)delay=50;
        if(result.allowed&&!updateRequired.value&&!d.blocked()){
          const before=Number(d.total());
          try{d.press('F24');}finally{d.release('F24');}
          sent=1;confirmed=Number(d.total())>before?1:0;
        }
        autoChest(d);
      }catch(error){state.value={...state.value,error:String(error.message||error)};delay=1500;}
      timer=window.setTimeout(loop,delay);
    }
    timer=window.setTimeout(loop,1000);
    const saved=state.value.monitor;
    if(state.value.settings.remember_monitor&&saved?.name){
      try{const monitors=await d.monitors();if(monitors.filter(m=>m.name===saved.name).length===1)await d.restoreMonitor(saved.name);}
      catch(error){state.value={...state.value,error:'屏幕恢复：'+error.message};}
    }
    let monitorName=saved?.name;
    async function rememberMonitor(){
      if(!state.value.settings.remember_monitor)return;
      try{const m=await d.currentMonitor();if(m?.name&&m.name!==monitorName){await request('monitor',{monitor:{name:m.name,position:m.position,size:m.size}});monitorName=m.name;}}catch{}
    }
    await rememberMonitor();window.setInterval(rememberMonitor,30000);
    await atlas.request('/update/healthy',{version:VERSION});
    let observedRoom=null;
    d.listenRoom(async(code,host)=>{const new_visit=String(code||'')!==observedRoom;observedRoom=String(code||'');if(!atlas.enabled.value&&code)await request('room',{code,host,new_visit});});
    await d.listenJoin(async code=>{if(atlas.enabled.value){d.toast('本地全图鉴开启时无法进入联机');return;}if(!updateRequired.value)await d.join(code);});
    window.addEventListener('pagehide',()=>{disposed=true;window.clearTimeout(timer);});
  }
  let dashGen=0,requestedTab='overview',lastTab='overview';
  // 关面板时记下停在哪一页：右键「控制台」再打开时回到那一页，而不是永远回概览。
  function closePanel(){dashGen++;if(dashboard)lastTab=dashboard.currentTab?.()||lastTab;dashboard?.close();dashboard=null;}
  async function deleteRoom(code){await ready;return (await request('delete-room',{code})).rooms||[];}
  // 「控制台」在支持面板页签的壳里是面板的一页（联机下方、设置上方）；
  // 壳没提供就退回桌宠窗口内的浮层，两种都拿到同一套界面。
  async function openPanel(initialTab){
    // 浮层已经开着就直接切页聚焦，别关了重建（重建会闪一下，还会丢掉正在看的页）。
    // 面板壳（macOS）那条路不能这么走：它要靠 openTab 把页签叫到前台。
    if(!window.__WUWA_OPEN_PANEL_TAB__&&dashboard){
      dashboard.showTab(initialTab||dashboard.currentTab?.()||lastTab);
      dashboard.focus();
      return;
    }
    // 先关再取：面板正开着时，「当前页」比「上次关面板那页」更准。
    closePanel();
    requestedTab=initialTab=initialTab||lastTab;
    await ready;
    const openTab=window.__WUWA_OPEN_PANEL_TAB__;
    if(openTab){
      try{if(await openTab('console')!==false)return;}
      catch(error){console.warn('[console] panel tab unavailable',error);}
    }
    openFloating(initialTab);
  }
  function openFloating(initialTab){
    if(dashboard){dashboard.showTab(initialTab);dashboard.focus();return;}
    dashboard=createAssistantDashboard({window,version:VERSION,atlas,initialTab,
      getState:async()=>{const v=await request('state');state.value=v;return v;},
      configure:async changes=>{state.value=await request('configure',{settings:changes});return state.value;},
      reset:()=>request('reset'),join:code=>window.__TAURI_INTERNALS__.invoke('plugin:event|emit',{event:'local-assistant-join',payload:code}),
      favorite:(code,favorite)=>request('favorite',{code,favorite}),deleteRoom,onClose:()=>{dashboard=null;}});
  }
  // 面板「控制台」页挂载点：把同一套界面内嵌进去，不再叠一层浮层。
  async function mountConsole(mount){
    closePanel();
    const generation=dashGen;
    await ready;
    if(generation!==dashGen||!mount?.isConnected)return;
    dashboard=createAssistantDashboard({window,version:VERSION,atlas,mount,initialTab:requestedTab,
      getState:async()=>{const v=await request('state');state.value=v;return v;},
      configure:async changes=>{state.value=await request('configure',{settings:changes});return state.value;},
      reset:()=>request('reset'),join:code=>window.__TAURI_INTERNALS__.invoke('plugin:event|emit',{event:'local-assistant-join',payload:code}),
      favorite:(code,favorite)=>request('favorite',{code,favorite}),deleteRoom,onClose:()=>{dashboard=null;}});
  }
  async function pollPolicy(){
    if(disposed)return;
    try{const result=await atlas.request('/update/status',{});showRequiredUpdate(result);}catch{}
    if(!disposed)window.setTimeout(pollPolicy,15000);
  }
  ready.then(pollPolicy).catch(()=>{});
  window.addEventListener('pagehide',()=>{disposed=true;closePanel();});
  return {ready,state,updateRequired,startMain,openPanel,mountConsole,closePanel,flush,flushBeforeSwitch,beginChest,chestSucceeded,
    physicalKey,physicalRelease,emoteStats};
}
