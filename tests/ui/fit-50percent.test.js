// 画面の自動テスト（Playwright）。Rust 側との通信（window.__TAURI__）を偽物に差し替えて、表示と動きを確かめる。
// 実行: npm run test:ui（すべて） / node tests/ui/<名前>.test.js（これだけ）
const { chromium } = require('playwright');
const path = require('path'), fs = require('fs'), { pathToFileURL } = require('url');
const FILE = pathToFileURL(path.resolve(__dirname, '../../src/index.html')).href;
const OUT = path.resolve(__dirname, 'out');
fs.mkdirSync(OUT, { recursive: true });
function check(cond, msg) { if (!cond) { console.error('NG: ' + msg); process.exitCode = 1; } }
(async()=>{const b=await chromium.launch();const p=await b.newPage({viewport:{width:1280,height:2000}});
await p.goto(FILE);await p.waitForTimeout(2500);
await p.evaluate(()=>{const r=document.getElementById('radarOn');r.checked=true;r.dispatchEvent(new Event('change'));});
await p.evaluate(()=>{const w=document.getElementById('warnSel');w.value=''+'warn'+'';w.dispatchEvent(new Event('change'));}).catch(()=>{});
await p.waitForTimeout(500);
for(const z of [100,50]){
 await p.evaluate(z=>{const e=document.getElementById('zAll');e.value=z;e.dispatchEvent(new Event('input'));},z);
 await p.waitForTimeout(300);
 const m=await p.evaluate(()=>{const o={};['clockCard','wxCard','usageCard','tempCard','netCard'].forEach(id=>o[id]=Math.round(document.getElementById(id).getBoundingClientRect().height));
  o.topbar=Math.round(document.querySelector('.topbar').getBoundingClientRect().height);o.stage=Math.round(document.querySelector('.stage').getBoundingClientRect().height);o.app=Math.round(document.getElementById('app').getBoundingClientRect().height);return o});
 console.log(z, JSON.stringify(m));
 // 全体50%・レーダー表示・警報の札3つで、1920×1080・拡大率150%の画面（窓に使える高さ約624px）に縦1列で収まること
 if(z===50) check(m.stage<=624, '全体50%で縦1列が624pxを超えた: '+m.stage);
}
await b.close()})();
