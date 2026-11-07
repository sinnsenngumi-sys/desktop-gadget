// 画面の自動テスト（Playwright）。Rust 側との通信（window.__TAURI__）を偽物に差し替えて、表示と動きを確かめる。
// 実行: npm run test:ui（すべて） / node tests/ui/<名前>.test.js（これだけ）
const { chromium } = require('playwright');
const path = require('path'), fs = require('fs'), { pathToFileURL } = require('url');
const FILE = pathToFileURL(path.resolve(__dirname, '../../src/index.html')).href;
const OUT = path.resolve(__dirname, 'out');
fs.mkdirSync(OUT, { recursive: true });
function check(cond, msg) { if (!cond) { console.error('NG: ' + msg); process.exitCode = 1; } }
const mock=`(function(){
  function wall(){ // 1600x1000: 上の40%は明るい空、その下は暗い森。右下は白黒の縞（明暗が混じる）
    var c=new OffscreenCanvas(1600,1000),x=c.getContext('2d');
    x.fillStyle='#e9f1fb';x.fillRect(0,0,1600,400);
    x.fillStyle='#1d2a1f';x.fillRect(0,400,1600,600);x.fillStyle='#8f8a80';x.fillRect(0,330,1600,240);
    for(var i=0;i<40;i++){x.fillStyle=i%2?'#ffffff':'#000000';x.fillRect(i*40,780,40,220)}
    return c.convertToBlob({type:'image/png'}).then(function(b){return new Promise(function(ok){var r=new FileReader();r.onload=function(){ok(r.result)};r.readAsDataURL(b)})});
  }
  window.__TAURI__={event:{listen:function(){return Promise.resolve(function(){})},emit:function(){return Promise.resolve()}},
   window:{getCurrentWindow:function(){return{label:'main',onMoved:function(){},close:function(){}}}},
   core:{invoke:function(c,a){
    if(c==='get_wallpaper')return wall().then(function(u){return {image:u,path:'x',style:2,tile:false,bg:[0,0,0]}});
    if(c==='get_window_geometry')return Promise.resolve({mon_x:0,mon_y:0,mon_w:1600,mon_h:1000,scale:1,win_x:0,win_y:0});
    if(c==='get_usage')return Promise.resolve({cpu:20,ram:50,ram_used_gb:8,ram_total_gb:16,gpus:[],notes:[],cpu_temp_c:55,net:{rx_bps:1e5,tx_bps:1e4,name:'Wi-Fi',up:1}});
    if(c==='get_system_state')return Promise.resolve({quns:5,game:false,locked:false,on_battery:false});
    if(c==='settings_read')return Promise.resolve(null);
    return Promise.resolve(null);}}};
})();`;
(async()=>{const b=await chromium.launch();const p=await b.newPage({viewport:{width:1600,height:1000}});const errs=[];p.on('pageerror',e=>errs.push(e.message));
await p.addInitScript(mock);
await p.goto(FILE);await p.waitForTimeout(1500);
const st=()=>p.evaluate(()=>['clockCard','wxCard','usageCard','tempCard','netCard'].map(id=>{const e=document.getElementById(id);const r=e.getBoundingClientRect();return id+'@'+Math.round(r.top)+'-'+Math.round(r.bottom)+':'+getComputedStyle(e).color+(e.classList.contains('lowc-light')?'+影':'')+(e.classList.contains('lowc-dark')?'+白影':'')}));
console.log('白（既定）',JSON.stringify(await st()));
await p.evaluate(()=>{const r=document.querySelector('input[name=fg][value=auto]');r.checked=true;r.dispatchEvent(new Event('change'));});
await p.waitForTimeout(300);
console.log('自動',JSON.stringify(await st()));
const st2=()=>p.evaluate(()=>['clockCard','wxCard','usageCard','tempCard','netCard'].map(id=>{const e=document.getElementById(id);return id.replace('Card','')+':'+(getComputedStyle(e).color==='rgb(255, 255, 255)'?'白':'黒')+' 膜'+(e.style.getPropertyValue('--scrim')||'-').replace('rgba(','').replace(')','')+(e.className.includes('lowc')?' 影':'')}).join(' | '));
for(const pref of ['white','dark','none']){
  await p.evaluate(v=>{const e=document.getElementById('autoPref');e.value=v;e.dispatchEvent(new Event('change'))},pref);
  await p.waitForTimeout(150);
  const line=await st2(); console.log(pref, line);
  if(pref==='white') check(!line.includes('黒'),'白優先なのに黒い文字のカードがある');
  if(pref==='dark') check(!line.includes('白'),'黒優先なのに白い文字のカードがある');
  await p.screenshot({path:OUT+'/autofg_'+pref+'.png',clip:{x:0,y:0,width:470,height:860}});
}
console.log('ERR',JSON.stringify(errs)); check(errs.length === 0, 'ページでエラーが出た');await b.close()})();
