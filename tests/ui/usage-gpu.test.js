// 画面の自動テスト（Playwright）。Rust 側との通信（window.__TAURI__）を偽物に差し替えて、表示と動きを確かめる。
// 実行: npm run test:ui（すべて） / node tests/ui/<名前>.test.js（これだけ）
const { chromium } = require('playwright');
const path = require('path'), fs = require('fs'), { pathToFileURL } = require('url');
const FILE = pathToFileURL(path.resolve(__dirname, '../../src/index.html')).href;
const OUT = path.resolve(__dirname, 'out');
fs.mkdirSync(OUT, { recursive: true });
function check(cond, msg) { if (!cond) { console.error('NG: ' + msg); process.exitCode = 1; } }
const file = FILE;
const mock=(gpus)=>`window.__G=${JSON.stringify(gpus)};window.__TAURI__={core:{invoke:function(c,a){
 if(c==='get_usage')return Promise.resolve({cpu:37,ram:61,ram_used_gb:9.8,ram_total_gb:15.7,gpus:window.__G,notes:[]});
 if(c==='list_gpus')return Promise.resolve(window.__G);
 if(c==='get_system_state')return Promise.resolve({on_battery:false,battery_saver:false,battery_percent:80,quns:5,fullscreen_window:false,foreground_exe:'explorer.exe',game:false,locked:false});
 if(c==='get_wallpaper')return Promise.reject('x');
 if(c==='get_window_geometry')return Promise.resolve({mon_x:0,mon_y:0,mon_w:1920,mon_h:1080,scale:1,win_x:0,win_y:0});
 return Promise.resolve('ok')}},window:{getCurrentWindow:function(){return{onMoved:function(){},close:function(){}}}}};`;
const two=[{id:'A',name:'Iris Xe',full_name:'Intel(R) Iris(R) Xe Graphics',usage:12},{id:'B',name:'GTX 1650 Ti',full_name:'NVIDIA GeForce GTX 1650 Ti',usage:64}];
const rows=p=>p.evaluate(()=>[...document.querySelectorAll('#gpuRows .u')].map((r,i)=>r.querySelector('b').textContent+' '+document.querySelectorAll('#gpuRows .cap')[i].textContent));
(async()=>{const b=await chromium.launch();const errs=[];
 // 新規: 既定で2行
 let ctx=await b.newContext({viewport:{width:900,height:1300}});let p=await ctx.newPage();p.on('pageerror',e=>errs.push(e.message));
 await p.addInitScript(mock(two));await p.goto(file);await p.waitForTimeout(2000);
 console.log('default',await p.evaluate(()=>gpuSel.value),JSON.stringify(await rows(p)));
 await (await p.$('#usageCard')).screenshot({path:OUT+'/usage2.png'});
 await p.evaluate(()=>{const e=document.getElementById('panel');e.style.display=e.style.display==='none'?'':'none';window.dispatchEvent(new Event('resize'));});await p.waitForTimeout(300);
 console.log('opts',JSON.stringify(await p.evaluate(()=>[...gpuSel.options].map(o=>o.value+'='+o.text))));
 await p.selectOption('#gpuSel','auto');await p.waitForTimeout(1200);console.log('auto',JSON.stringify(await rows(p)));
 await p.selectOption('#gpuSel','A');await p.waitForTimeout(1200);console.log('fixed A',JSON.stringify(await rows(p)));
 await p.selectOption('#gpuSel','all');await p.waitForTimeout(1200);console.log('all',JSON.stringify(await rows(p)));
 await ctx.close();
 // 前の版の保存（auto）→ すべてに移行。2回目は移行しない
 ctx=await b.newContext({viewport:{width:900,height:1300}});p=await ctx.newPage();p.on('pageerror',e=>errs.push(e.message));
 await p.addInitScript(mock(two));
 await p.goto(file);await p.evaluate(()=>{localStorage.clear();localStorage.setItem('desktopGadget.v1',JSON.stringify({v:{gpuSel:'auto'},r:{},tc:{}}))});
 await p.reload();await p.waitForTimeout(1500);console.log('migrated',await p.evaluate(()=>gpuSel.value),JSON.stringify(await rows(p)));
 await p.evaluate(()=>{const e=document.getElementById('panel');e.style.display=e.style.display==='none'?'':'none';window.dispatchEvent(new Event('resize'));});await p.selectOption('#gpuSel','auto');await p.reload();await p.waitForTimeout(1500);console.log('kept auto after migration',await p.evaluate(()=>gpuSel.value));
 await ctx.close();
 for(const [n,g] of [['one',[two[0]]],['none',[]]]){ctx=await b.newContext();p=await ctx.newPage();p.on('pageerror',e=>errs.push(e.message));
  await p.addInitScript(mock(g));await p.goto(file);await p.waitForTimeout(1800);console.log(n,JSON.stringify(await rows(p)));await ctx.close();}
 console.log('ERRORS',JSON.stringify(errs)); check(errs.length === 0, 'ページでエラーが出た');await b.close()})();
