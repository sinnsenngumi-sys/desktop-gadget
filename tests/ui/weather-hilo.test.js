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
  const p = await b.newPage({ viewport: { width: 1000, height: 1400 } });
  await p.addInitScript(mock);
  p.on('pageerror', e => errs.push(e.message));
  await p.goto(file);
  await p.waitForTimeout(800);
  await p.evaluate(()=>{panel.style.display='';});
  await p.fill('#locQuery','千代田');await p.press('#locQuery','Enter');await p.waitForTimeout(300);
  await p.click('#locResults button');await p.waitForTimeout(800);
  await p.evaluate(()=>{panel.style.display='none';});
  const hs=[];
  const cases=[[14.6,18.2,9.4,'c','real'],[-12.4,-3.1,-18.7,'c','sp'],[38.5,40.2,29.9,'f','sp'],[null,null,null,'c','real']];
  for(const [t,mx,mn,u,w] of cases){
    const r=await p.evaluate(({t,mx,mn,u,w})=>{
      GadgetWx._state.wx={temp_c:t,humidity:72,code:61,is_day:true,temp_max_c:mx,temp_min_c:mn};GadgetWx._state.wxAt=Date.now();
      const un=document.getElementById('tempUnit');un.value=u;un.dispatchEvent(new Event('change'));
      const ws=document.getElementById('warnSel');ws.value=w;ws.dispatchEvent(new Event('change'));
      const c=document.getElementById('wxCard'),R=c.getBoundingClientRect();
      const out=[...c.querySelectorAll('.wx *, .rc *, .hilo *')].filter(x=>{const q=x.getBoundingClientRect();return q.width>0&&q.right>R.right-20});
      const hl=document.getElementById('hilo').getBoundingClientRect(),rc=c.querySelector('.rc').getBoundingClientRect();
      return {temp:document.getElementById('temp').textContent,hi:tmax.textContent,lo:tmin.textContent,overlap:hl.right>rc.left+1,out:out.length,h:Math.round(R.height)};
    },{t,mx,mn,u,w});
    console.log(JSON.stringify(r)); check(r.out===0,'天気カードの中身がはみ出した'); hs.push(r.h);
    await (await p.$('#wxCard')).screenshot({path:out+'/hilo_'+u+'_'+String(t)+'.png'});
  }
  check(new Set(hs).size===1,'気温の値によってカードの高さが変わる: '+hs.join(','));
  await p.evaluate(()=>{const e=document.getElementById('hiloOn');e.checked=false;e.dispatchEvent(new Event('change'));});
  console.log('オフ', await p.evaluate(()=>getComputedStyle(document.getElementById('hilo')).display));
  console.log('ERRORS', JSON.stringify(errs)); check(errs.length === 0, 'ページでエラーが出た');
  await b.close();
})();
