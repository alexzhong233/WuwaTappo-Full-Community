const assert=require('node:assert/strict'),vm=require('node:vm');
const {renderFrontend}=require('../../../shared/tests/render-frontend.cjs');
const source=renderFrontend('desktop_assistant.js','win');
let now=100000, count=0,pressed=0,released=0,clicks=0,allowed=true,quiet=true,historyFails=false;
const queue=[], requests=[];
const settings={enabled:true,speed:10,auto_chest:true,threshold:1000};
const value={settings,local:{total:0,balance:0,chests:{},opened:0},monitor:{name:'missing'},rooms:[]};
const button={disabled:false,isConnected:true,getClientRects:()=>[{}],click:()=>{clicks++;}};
const atlas={enabled:{value:false},ready:Promise.resolve(),request:async(path,body)=>{
 requests.push({path,body});
 if(path.endsWith('/reward')&&historyFails)throw Error('temporary history save failure');
 if(path.endsWith('/tick'))return {allowed,quiet,settings,session:pressed,confirmed:released};
 return value;
}};
const window={document:{querySelector:selector=>selector.includes('left')?button:null},addEventListener(){},
 setTimeout:(fn,delay)=>{queue.push({fn,at:now+delay});return queue.length;},clearTimeout(){},setInterval(){},clearInterval(){}};
const FakeDate=class extends Date {static now(){return now;}};
const context=vm.createContext({window,Date:FakeDate,console,Map,Math,Number,Promise});
vm.runInContext(source+';globalThis.make=createDesktopAssistant;',context);
const assistant=context.make({window,ref:value=>({value}),atlas});
(async()=>{
 let roomListener;
 await assistant.startMain({total:()=>count,balance:()=>2000,account:()=> 'test',loggedIn:()=>true,
 blocked:()=>false,press:key=>{assert.equal(key,'F24');pressed++;},release:key=>{released++;count++;},
 chestCost:1000,
 currentMonitor:async()=>({name:'available'}),listenRoom:callback=>{roomListener=callback;},listenJoin:async()=>{},join(){},toast(){}});
 const step=async()=>{assert(queue.length);queue.sort((a,b)=>a.at-b.at);const task=queue.shift();now=Math.max(now,task.at);await task.fn();};
 await step();assert.equal(count,1);assert.equal(clicks,0);
 now+=1900;await step();assert.equal(clicks,0);
 now+=101;await step();assert.equal(clicks,1);
 assert.equal(pressed,released);
 allowed=false;quiet=false;
 const before=count;now+=100;await step();assert.equal(count,before);
 assert.equal(requests.filter(x=>x.path.endsWith('/tick')).at(-1).body.confirmed,1);
 // Display mode switches and closing the dashboard must not stop the driver.
 allowed=true;quiet=true;atlas.enabled.value=true;assistant.closePanel();
 // Hybrid mode: atlas is enabled, room recording still POSTs /assistant/room.
 await roomListener('room-42','host');
 const roomRequest=requests.filter(x=>x.path==='/assistant/room').at(-1);
 assert.equal(roomRequest.body.code,'room-42');
 assert.equal(roomRequest.body.host,'host');
 const normal=count;now+=100;await step();assert.equal(count,normal+1);
 const attempt=assistant.beginChest('left');assistant.chestSucceeded(attempt,{asset_id:1,asset_type:'emote',asset_name:'real.png',click_consume:1000});
 now+=1100;await step();assert(requests.some(x=>x.path==='/assistant/reward'&&x.body.reward.confirmed));
 historyFails=true;assistant.chestSucceeded(assistant.beginChest('right'),{asset_id:2,asset_type:'emote',asset_name:'real2.png',click_consume:1000});
 const running=count;now+=1100;await step();assert.equal(count,running+1);historyFails=false;
 now+=1100;await step();assert(requests.filter(x=>x.path==='/assistant/reward'&&x.body.reward.asset_id===2).length>=2);
 assert(requests.some(x=>x.path==='/update/healthy'));
 assert.equal(assistant.releasePanel,undefined);
 console.log('assistant renderer passed: paired signals, physical pause, real chest delay, hybrid room recording, shared counters after mode switch/close, confirmed history delivery');
})().catch(error=>{console.error(error);process.exitCode=1;});
