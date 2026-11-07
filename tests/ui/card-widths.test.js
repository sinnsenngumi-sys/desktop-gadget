// 画面の自動テスト（Playwright）。Rust 側との通信（window.__TAURI__）を偽物に差し替えて、表示と動きを確かめる。
// 実行: npm run test:ui（すべて） / node tests/ui/<名前>.test.js（これだけ）
const { chromium } = require('playwright');
const path = require('path'), fs = require('fs'), { pathToFileURL } = require('url');
const FILE = pathToFileURL(path.resolve(__dirname, '../../src/index.html')).href;
const OUT = path.resolve(__dirname, 'out');
fs.mkdirSync(OUT, { recursive: true });
function check(cond, msg) { if (!cond) { console.error('NG: ' + msg); process.exitCode = 1; } }
(async()=>{const b=await chromium.launch();const p=await b.newPage({viewport:{width:2400,height:3200}});const errs=[];p.on('pageerror',e=>errs.push(e.message));
await p.goto(FILE);
await p.addStyleTag({content:'body{background:#555}'});
await p.evaluate(()=>{const set=(id,v)=>{const e=document.getElementById(id);if(e.type==='checkbox'){e.checked=v;e.dispatchEvent(new Event('change'))}else{e.value=v;e.dispatchEvent(new Event(e.tagName==='SELECT'?'change':'input'))}};
  set('radarOn',true);set('warnSel','sp');set('ramGb',true);});
await p.waitForTimeout(2500);
const ids=['clockCard','wxCard','usageCard','tempCard','netCard'],sl={clockCard:'zClock',wxCard:'zWx',usageCard:'zUsage',tempCard:'zTemp',netCard:'zNet'};
const combos=[];const vals=[50,75,100,125,150];
for(const all of [50,100,150])for(let i=0;i<6;i++){const c={};ids.forEach((id,j)=>c[id]=i===0?100:vals[(i*3+j*2+all)%5]);combos.push([all,c])}
let bad=0,n=0;
for(const mode of ['digital','analog'])for(const layout of ['one','two'])for(const [all,c] of combos){
  const r=await p.evaluate(({all,c,sl,ids,mode,layout})=>{
    const set=(id,v)=>{const e=document.getElementById(id);e.value=v;e.dispatchEvent(new Event('input'))};
    const m=document.querySelector('input[name=mode][value='+mode+']');m.checked=true;m.dispatchEvent(new Event('change'));
    set('zAll',all);ids.forEach(id=>set(sl[id],c[id]));GadgetLayout.set(layout);
    const w=ids.map(id=>document.getElementById(id).getBoundingClientRect().width);
    const over=ids.filter(id=>{const e=document.getElementById(id);return e.scrollWidth>e.clientWidth+1});
    // 中身が枠からはみ出していないか（バッジ以外の子孫）
    const out=ids.filter(id=>{const e=document.getElementById(id),R=e.getBoundingClientRect();return [...e.querySelectorAll('*')].some(x=>{if(x.closest('.badge'))return false;const q=x.getBoundingClientRect();return q.width>0&&q.right>R.right+1})});
    return {w:w.map(x=>Math.round(x)),over,out};
  },{all,c,sl,ids,mode,layout});
  n++;
  const spread=Math.max(...r.w)-Math.min(...r.w);
  if(spread>1||r.over.length||r.out.length){bad++;console.log('NG',mode,layout,all,JSON.stringify(c),JSON.stringify(r))}
}
console.log('combos',n,'bad',bad); check(bad===0,'カードの幅がそろわない、またははみ出す組み合わせがある');
// 見本: 写真と近い設定（全体50%、カードごとにばらばら）
await p.evaluate(()=>{const set=(id,v)=>{const e=document.getElementById(id);e.value=v;e.dispatchEvent(new Event('input'))};
  const m=document.querySelector('input[name=mode][value=analog]');m.checked=true;m.dispatchEvent(new Event('change'));
  set('zAll',50);set('zClock',110);set('zWx',90);set('zUsage',120);set('zTemp',90);set('zNet',100);GadgetLayout.set('one');});
await p.addStyleTag({content:'body{background:linear-gradient(160deg,#2b2d6e,#c95d9b)}'});
await p.waitForTimeout(500);
await (await p.$('.stack')).screenshot({path:OUT+'/widths_sample.png'});
console.log('ERR',JSON.stringify(errs)); check(errs.length === 0, 'ページでエラーが出た');await b.close()})();
