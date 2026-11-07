// 画面の自動テスト（Playwright）。Rust 側との通信（window.__TAURI__）を偽物に差し替えて、表示と動きを確かめる。
// 実行: npm run test:ui（すべて） / node tests/ui/<名前>.test.js（これだけ）
const { chromium } = require('playwright');
const path = require('path'), fs = require('fs'), { pathToFileURL } = require('url');
const FILE = pathToFileURL(path.resolve(__dirname, '../../src/index.html')).href;
const OUT = path.resolve(__dirname, 'out');
fs.mkdirSync(OUT, { recursive: true });
function check(cond, msg) { if (!cond) { console.error('NG: ' + msg); process.exitCode = 1; } }
(async()=>{const b=await chromium.launch();const p=await b.newPage({viewport:{width:1000,height:1500}});const errs=[];p.on('pageerror',e=>errs.push(e.message));
await p.goto(FILE);
await p.addStyleTag({content:'body{background:linear-gradient(160deg,#2b2d6e,#c95d9b)} .topbar{display:none}'});
await p.waitForTimeout(6500);
const info=()=>p.evaluate(()=>({rx:vrx.textContent,tx:vtx.textContent,net:netName.textContent,temps:[...document.querySelectorAll('#tempRows .u')].map((r,i)=>r.textContent+' / '+document.querySelectorAll('#tempRows .cap')[i].textContent),note:tempNote.textContent,hist:GadgetNet._h.rx.length}));
console.log(JSON.stringify(await info()));
await (await p.$('.stack')).screenshot({path:OUT+'/cards_one.png'});
// 横2列・単位・℉
await p.evaluate(()=>{panel.style.display='';});
await p.selectOption('#layoutSel','two');await p.selectOption('#netUnit','byte');await p.selectOption('#tempUnit','f');
await p.evaluate(()=>{panel.style.display='none';GadgetLayout.set('two');});
await p.waitForTimeout(1500);
console.log(JSON.stringify(await info()), await p.evaluate(()=>[...document.querySelectorAll('.stack .col')].map(c=>[...c.children].map(x=>x.id).join(','))));
await (await p.$('.stack')).screenshot({path:OUT+'/cards_two.png'});
// 1列に戻すと順番が戻るか、カードを隠せるか
await p.evaluate(()=>{panel.style.display='';});
await p.evaluate(()=>GadgetLayout.set('one'));await p.uncheck('#showNet');
console.log(await p.evaluate(()=>[...document.querySelector('.stack').children].map(x=>x.id+(x.style.display==='none'?'(隠)':'')).join(',')));
console.log('ERR',JSON.stringify(errs)); check(errs.length === 0, 'ページでエラーが出た');await b.close()})();
