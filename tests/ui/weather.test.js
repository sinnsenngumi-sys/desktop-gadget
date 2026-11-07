// 画面の自動テスト（Playwright）。Rust 側との通信（window.__TAURI__）を偽物に差し替えて、表示と動きを確かめる。
// 実行: npm run test:ui（すべて） / node tests/ui/<名前>.test.js（これだけ）
const { chromium } = require('playwright');
const path = require('path'), fs = require('fs'), { pathToFileURL } = require('url');
const FILE = pathToFileURL(path.resolve(__dirname, '../../src/index.html')).href;
const OUT = path.resolve(__dirname, 'out');
fs.mkdirSync(OUT, { recursive: true });
function check(cond, msg) { if (!cond) { console.error('NG: ' + msg); process.exitCode = 1; } }
// フェーズ4の画面確認（Tauriの呼び出しを、本物と同じ形式の偽データに差し替える）
const file = FILE;
const out = OUT;

const mock = `
(function(){
  var M=window.__mock={calls:{},fail:{},warnItems:[
    {code:'49',name:'レベル4土砂災害危険警報',kind:'danger',rank:1},
    {code:'03',name:'レベル3大雨警報',kind:'warning',rank:2},
    {code:'14',name:'雷注意報',kind:'advisory',rank:3}]};
  function stamp(d){var p=function(n){return String(n).padStart(2,'0')};return d.getUTCFullYear()+p(d.getUTCMonth()+1)+p(d.getUTCDate())+p(d.getUTCHours())+p(d.getUTCMinutes())+'00'}
  function times(){
    var now=new Date();now.setUTCSeconds(0,0);now.setUTCMinutes(Math.floor(now.getUTCMinutes()/5)*5-5);
    var past=[],fc=[];
    for(var i=0;i<37;i++){var d=new Date(now.getTime()-i*300000);past.push({basetime:stamp(d),validtime:stamp(d)})}
    for(var j=1;j<=12;j++){var e=new Date(now.getTime()+j*300000);fc.push({basetime:stamp(now),validtime:stamp(e)})}
    return {past:past.reverse(),forecast:fc,zooms:[4,6,8,10],note:null};
  }
  var JMA=[[242,242,255],[160,210,255],[33,140,255],[0,65,255],[250,245,0],[255,153,0],[255,40,0],[180,0,104]];
  function png(draw){var c=new OffscreenCanvas(256,256),x=c.getContext('2d');draw(x);return c.convertToBlob({type:'image/png'}).then(function(b){return b.arrayBuffer()})}
  function radarTile(a){
    var t=parseInt(a.validtime.slice(8,12),10),sh=(Math.floor(t/100)*60+t%100)%600;
    return png(function(x){
      // 雨のかたまり（気象庁の色で、強さの段を同心円に）
      for(var k=0;k<3;k++){
        var cx=((a.x*97+k*83+sh*1.3)%300)-20,cy=((a.y*61+k*131)%256);
        for(var lv=0;lv<8;lv++){x.fillStyle='rgb('+JMA[lv].join(',')+')';x.beginPath();x.arc(cx,cy,60-lv*7,0,7);x.fill()}
      }
    });
  }
  function baseTile(a){
    return png(function(x){
      x.fillStyle='#f5f5f2';x.fillRect(0,0,256,256);
      x.fillStyle='#bfd9ef';x.beginPath();x.moveTo(170,0);x.lineTo(256,0);x.lineTo(256,256);x.lineTo(200,256);x.closePath();x.fill();
      x.strokeStyle='#d8d8d0';x.lineWidth=2;for(var i=0;i<256;i+=48){x.beginPath();x.moveTo(0,i+(a.x%3)*8);x.lineTo(256,i+30);x.stroke()}
      x.strokeStyle='#b9b9c8';x.lineWidth=1;x.strokeRect(0.5,0.5,255,255);
    });
  }
  var area={code:'1310100',name:'千代田区',office:'130000',office_name:'東京都',note:null,
    options:[{code:'1310100',name:'千代田区'},{code:'1310200',name:'中央区'},{code:'1310300',name:'港区'}]};
  window.__TAURI__={core:{invoke:function(c,a){
    M.calls[c]=(M.calls[c]||0)+1;
    if(M.fail[c])return Promise.reject(M.fail[c]);
    switch(c){
      case 'wx_sources':return Promise.resolve({path:'C:\\\\Users\\\\me\\\\AppData\\\\Roaming\\\\local.desktopgadget\\\\sources.json',note:null});
      case 'wx_geocode':return Promise.resolve([{name:'千代田区',detail:'東京都',lat:35.694,lon:139.7536,country_code:'JP'},{name:'千代田町',detail:'群馬県',lat:36.21,lon:139.44,country_code:'JP'}]);
      case 'wx_find_area':return Promise.resolve(area);
      case 'wx_area_info':return Promise.resolve(Object.assign({},area,{code:a.code}));
      case 'wx_current':return Promise.resolve({temp_c:14.6,humidity:72,code:61,is_day:false,time:'2026-11-04T01:15'});
      case 'wx_warnings':return Promise.resolve({items:M.warnItems,report_time:'2026-11-04T00:45:00+09:00',office_name:'東京都',area_name:'千代田区'});
      case 'wx_radar_times':return Promise.resolve(times());
      case 'wx_radar_tile':return radarTile(a);
      case 'wx_base_tile':return baseTile(a);
      case 'get_usage':return Promise.resolve({cpu:20,ram:50,ram_used_gb:8,ram_total_gb:16,gpus:[],notes:[]});
      case 'list_gpus':return Promise.resolve([]);
      case 'get_system_state':return Promise.resolve(Object.assign({on_battery:false,battery_saver:false,battery_percent:null,quns:5,fullscreen_window:false,foreground_exe:null,game:false,locked:false},M.state||{}));
      case 'get_wallpaper':return Promise.reject('テストでは壁紙なし');
      case 'get_window_geometry':return Promise.resolve({mon_x:0,mon_y:0,mon_w:1920,mon_h:1080,scale:1,win_x:0,win_y:0});
    }
    return Promise.resolve('ok');
  }},window:{getCurrentWindow:function(){return{onMoved:function(){},close:function(){}}}}};
})();`;

(async () => {
  const b = await chromium.launch();
  const errs = [];
  const ctx = await b.newContext({ viewport: { width: 1000, height: 1400 } });
  await ctx.addInitScript(mock);
  const p = await ctx.newPage();
  p.on('pageerror', e => errs.push(e.message));
  p.on('console', m => { if (m.type() === 'error') errs.push('console: ' + m.text()) });
  await p.goto(file);
  await p.waitForTimeout(1000);
  const card = () => p.evaluate(() => ({ place: wxPlace.textContent, temp: temp.textContent, hum: hum.textContent, upd: upd.textContent,
    warns: [...document.querySelectorAll('#warns .w')].map(e => e.className.replace('w ', '') + ':' + e.textContent),
    area: areaSel.value, areaOpts: areaSel.options.length, chip: rlabel.textContent, rmsg: rmsg.textContent, rcap: rcap.textContent,
    frames: GadgetWx._state.frames.length, built: GadgetWx._state.frames.filter(f => f.canvas).length, note: wxNote.textContent, calls: Object.assign({}, __mock.calls) }));
  console.log('1 未設定', JSON.stringify(await card()));

  // 地名検索 → 選ぶ
  await p.evaluate(()=>{const e=document.getElementById('panel');e.style.display=e.style.display==='none'?'':'none';window.dispatchEvent(new Event('resize'));});
  await p.fill('#locQuery', '千代田');
  await p.press('#locQuery', 'Enter');
  await p.waitForTimeout(300);
  console.log('results', JSON.stringify(await p.evaluate(() => [...document.querySelectorAll('#locResults button')].map(b => b.textContent))));
  await p.click('#locResults button');
  await p.waitForTimeout(800);
  console.log('2 場所設定', JSON.stringify(await card()), await p.evaluate(() => locCur.textContent));

  // レーダーをオン
  await p.check('#radarOn');
  await p.waitForTimeout(3500);
  console.log('3 レーダー', JSON.stringify(await card()));
  await p.evaluate(()=>{const e=document.getElementById('panel');e.style.display=e.style.display==='none'?'':'none';window.dispatchEvent(new Event('resize'));}); // 閉じる
  await p.waitForTimeout(400);
  await (await p.$('#wxCard')).screenshot({ path: out + '/wx_dark.png' });
  // アニメが進むか
  const c1 = await p.evaluate(() => rlabel.textContent);
  await p.waitForTimeout(2200);
  const c2 = await p.evaluate(() => rlabel.textContent);
  console.log('anim', c1, '→', c2);

  // ライト地図、詳細
  await p.evaluate(()=>{const e=document.getElementById('panel');e.style.display=e.style.display==='none'?'':'none';window.dispatchEvent(new Event('resize'));});
  await p.selectOption('#mapSel', 'light');
  await p.selectOption('#radarZoom', '10');
  await p.selectOption('#tempUnit', 'f');
  await p.waitForTimeout(3000);
  await p.evaluate(()=>{const e=document.getElementById('panel');e.style.display=e.style.display==='none'?'':'none';window.dispatchEvent(new Event('resize'));});
  await p.waitForTimeout(400);
  await (await p.$('#wxCard')).screenshot({ path: out + '/wx_light.png' });
  console.log('4 ライト・詳細・℉', JSON.stringify(await card()));

  // 古いデータの表示: 取得時刻を古くし、取得を失敗させる
  await p.evaluate(() => { __mock.fail.wx_current = '接続できません（オフラインかもしれません）'; GadgetWx._state.wxAt = Date.now() - 3600e3; GadgetWx._state.timesAt = Date.now() - 3600e3; });
  await p.evaluate(() => { document.getElementById('tempUnit').dispatchEvent(new Event('change')); GadgetWx.draw(); });
  console.log('5 古いデータ', JSON.stringify(await card()));

  // ゲーム中: 天気の取得が止まり、アニメも止まる
  await p.evaluate(() => { __mock.state = { game: true, quns: 3, game_reason: 'Direct3Dの全画面' } });
  await p.waitForTimeout(2600);
  const g1 = await p.evaluate(() => rlabel.textContent);
  await p.waitForTimeout(2000);
  const g2 = await p.evaluate(() => rlabel.textContent);
  console.log('6 ゲーム中 アニメ停止', g1 === g2, g1);
  await p.evaluate(() => { __mock.state = {}; delete __mock.fail.wx_current; });
  await p.waitForTimeout(2600);
  console.log('7 ゲーム後', JSON.stringify(await card()));

  // 試験の表示（特別警報）と、警報の地域の変更
  await p.evaluate(()=>{const e=document.getElementById('panel');e.style.display=e.style.display==='none'?'':'none';window.dispatchEvent(new Event('resize'));});
  await p.selectOption('#warnSel', 'sp');
  console.log('8 試験 特別警報', JSON.stringify((await card()).warns));
  await p.selectOption('#warnSel', 'real');
  await p.evaluate(() => { __mock.warnItems = []; });
  await p.selectOption('#areaSel', '1310200');
  await p.waitForTimeout(400);
  console.log('9 地域変更・発表なし', JSON.stringify(await card()));

  // 読み直しで残るか
  await p.reload();
  await p.waitForTimeout(3500);
  console.log('10 読み直し', JSON.stringify(await card()), await p.evaluate(() => [locName.value, locLat.value, radarZoom.value, mapSel.value, tempUnit.value]));

  // 国外
  await p.evaluate(()=>{const e=document.getElementById('panel');e.style.display=e.style.display==='none'?'':'none';window.dispatchEvent(new Event('resize'));});
  await p.evaluate(() => { __mock.fail.wx_find_area = 'この地点の市区町村がわかりません（海の上や日本の外かもしれません）' });
  await p.fill('#latIn', '48.85'); await p.fill('#lonIn', '2.35');
  await p.click('#latlonSet');
  await p.waitForTimeout(1200);
  console.log('11 国外', JSON.stringify(await card()));
  await p.screenshot({ path: out + '/wx_full.png' });
  console.log('ERRORS', JSON.stringify(errs)); check(errs.length === 0, 'ページでエラーが出た');
  await b.close();
})();
