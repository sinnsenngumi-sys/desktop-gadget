// 画面の自動テスト（Playwright）。Rust 側との通信（window.__TAURI__）を偽物に差し替えて、表示と動きを確かめる。
// 実行: npm run test:ui（すべて） / node tests/ui/<名前>.test.js（これだけ）
const { chromium } = require('playwright');
const path = require('path'), fs = require('fs'), { pathToFileURL } = require('url');
const FILE = pathToFileURL(path.resolve(__dirname, '../../src/index.html')).href;
const OUT = path.resolve(__dirname, 'out');
fs.mkdirSync(OUT, { recursive: true });
function check(cond, msg) { if (!cond) { console.error('NG: ' + msg); process.exitCode = 1; } }
// フェーズ5の確認: ガジェット(main)と設定画面(settings)の2つの窓を、同じ保存領域を持つ2ページで再現する
const file = FILE;
const out = OUT;

const mock = (label) => `
(function(){
  var bus=new BroadcastChannel('tauri-events'),handlers={};
  bus.onmessage=function(m){(handlers[m.data.ev]||[]).forEach(function(h){h({event:m.data.ev,payload:m.data.payload})})};
  function emitAll(ev,payload){(handlers[ev]||[]).forEach(function(h){h({event:ev,payload:payload})});bus.postMessage({ev:ev,payload:payload})}
  var M=window.__mock={label:'${label}',calls:{},log:[],emitAll:emitAll};
  window.__TAURI__={
    window:{getCurrentWindow:function(){return{label:'${label}',onMoved:function(){},close:function(){}}}},
    event:{
      listen:function(ev,h){(handlers[ev]=handlers[ev]||[]).push(h);return Promise.resolve(function(){})},
      emit:function(ev,payload){emitAll(ev,payload);return Promise.resolve()}
    },
    core:{invoke:function(c,a){
      M.calls[c]=(M.calls[c]||0)+1;M.log.push(c+' '+JSON.stringify(a||{}));
      switch(c){
        case 'settings_read':return Promise.resolve(localStorage.getItem('__file'));
        case 'settings_write':localStorage.setItem('__file',a.json);return Promise.resolve();
        case 'settings_dir':return Promise.resolve('C:\\\\Users\\\\me\\\\AppData\\\\Roaming\\\\local.desktopgadget');
        case 'settings_import':return Promise.resolve(window.__importJson||null);
        case 'settings_export':return Promise.resolve('C:\\\\Users\\\\me\\\\Documents\\\\desktop-gadget-settings.json');
        case 'autostart_get':return Promise.resolve({on:false,other_exe:false,exe:'"C:\\\\app\\\\desktop-gadget.exe"'});
        case 'autostart_set':return Promise.resolve({on:a.on,other_exe:false,exe:'"C:\\\\app\\\\desktop-gadget.exe"'});
        case 'list_monitors':return Promise.resolve([{index:0,name:'\\\\\\\\.\\\\DISPLAY1',x:0,y:0,width:1920,height:1080,scale:1.5,primary:true,current:true},{index:1,name:'\\\\\\\\.\\\\DISPLAY2',x:1920,y:0,width:2560,height:1440,scale:1,primary:false,current:false}]);
        case 'wx_geocode':return Promise.resolve([{name:'札幌',detail:'北海道',lat:43.06,lon:141.35,country_code:'JP'}]);
        case 'wx_find_area':return Promise.resolve({code:'0110000',name:'札幌市',office:'016000',office_name:'石狩・空知・後志地方',note:null,options:[{code:'0110000',name:'札幌市'},{code:'0120200',name:'函館市'}]});
        case 'wx_area_info':return Promise.resolve({code:a.code,name:'札幌市',office:'016000',office_name:'石狩・空知・後志地方',note:null,options:[{code:'0110000',name:'札幌市'},{code:'0120200',name:'函館市'}]});
        case 'wx_current':return Promise.resolve({temp_c:3.2,humidity:80,code:71,is_day:false,time:'x'});
        case 'wx_warnings':return Promise.resolve({items:[{code:'12',name:'大雪注意報',kind:'advisory',rank:3}],report_time:null,office_name:'x',area_name:'札幌市'});
        case 'wx_sources':return Promise.resolve({path:'C:\\\\x\\\\sources.json',note:null});
        case 'get_usage':return Promise.resolve({cpu:20,ram:50,ram_used_gb:8,ram_total_gb:16,gpus:[],notes:[]});
        case 'list_gpus':return Promise.resolve([]);
        case 'get_system_state':return Promise.resolve({on_battery:false,battery_saver:false,battery_percent:null,quns:5,fullscreen_window:false,foreground_exe:null,game:false,locked:false});
        case 'get_wallpaper':return Promise.reject('なし');
        case 'get_window_geometry':return Promise.resolve({mon_x:0,mon_y:0,mon_w:1920,mon_h:1080,scale:1,win_x:0,win_y:0});
      }
      return Promise.resolve(null);
    }}
  };
})();`;

(async () => {
  const b = await chromium.launch();
  const ctx = await b.newContext({ viewport: { width: 1000, height: 1000 } });
  const errs = [];
  const main = await ctx.newPage();
  await main.addInitScript(mock('main'));
  main.on('pageerror', e => errs.push('main: ' + e.message));
  await main.goto(file);
  await main.waitForTimeout(800);
  const st = (p) => p.evaluate(() => ({
    settingsMode: document.documentElement.classList.contains('settings-mode'),
    mode: (document.querySelector('input[name=mode]:checked') || {}).value,
    analogShown: getComputedStyle(document.getElementById('analogWrap')).display !== 'none',
    sec: document.getElementById('sec').checked,
    locked: document.documentElement.classList.contains('locked'),
    place: document.getElementById('wxPlace').textContent,
    clockOff: document.getElementById('clockCard').classList.contains('off'),
    calls: Object.assign({}, __mock.calls)
  }));
  console.log('1 main 起動', JSON.stringify(await st(main)), 'file?', await main.evaluate(() => !!localStorage.getItem('__file')));
  // 上のバーはマウスを乗せたときだけ
  const op0 = await main.evaluate(() => getComputedStyle(document.querySelector('.topbar')).opacity);
  await main.hover('#clockCard');
  await main.waitForTimeout(300);
  const op1 = await main.evaluate(() => getComputedStyle(document.querySelector('.topbar')).opacity);
  console.log('topbar opacity', op0, '→', op1);
  await main.click('#togglePanel');
  console.log('open_settings 呼び出し', await main.evaluate(() => __mock.calls.open_settings));

  // 設定画面の窓
  const set = await ctx.newPage();
  await set.addInitScript(mock('settings'));
  set.on('pageerror', e => errs.push('settings: ' + e.message));
  await set.setViewportSize({ width: 460, height: 820 });
  await set.goto(file);
  await set.waitForTimeout(800);
  console.log('2 settings 起動', JSON.stringify(await st(set)), 'panelVisible', await set.isVisible('#panel'), 'cardsVisible', await set.isVisible('#clockCard'));
  await set.screenshot({ path: out + '/p5_settings.png' });

  const w0 = await main.evaluate(() => __mock.calls.settings_write || 0);
  // 3) 設定画面で、アナログにする・秒を消す → ガジェットに反映
  await set.click('label:has(> input[name=mode][value=analog])');
  await set.uncheck('#sec');
  await main.waitForTimeout(700);
  console.log('3 main に反映', JSON.stringify(await st(main)));
  await main.waitForTimeout(800);
  const wm = await main.evaluate(() => __mock.calls.settings_write || 0), ws = await set.evaluate(() => __mock.calls.settings_write || 0);
  console.log('  保存回数 main', wm - w0, 'settings', ws, '（往復していなければ少ない）');

  // 4) 設定画面で場所を決める → ガジェットが天気を取る
  const wc0 = await main.evaluate(() => __mock.calls.wx_current || 0);
  await set.fill('#locQuery', '札幌'); await set.click('#locSearch'); await set.waitForTimeout(300);
  await set.click('#locResults button'); await set.waitForTimeout(1200);
  const s4 = await st(main);
  console.log('4 場所', s4.place, 'main wx_current +', s4.calls.wx_current - wc0, 'settings wx_current', (await st(set)).calls.wx_current || 0,
    'main areaSel', await main.inputValue('#areaSel'));

  // 5) 置き方と位置の固定
  await set.selectOption('#deskMode', 'normal');
  await set.check('#lockPos');
  await main.waitForTimeout(700);
  console.log('5 set_desktop_mode', JSON.stringify(await main.evaluate(() => __mock.log.filter(l => l.startsWith('set_desktop_mode')))), 'settings側の呼び出し', await set.evaluate(() => __mock.calls.set_desktop_mode || 0), 'locked', (await st(main)).locked);

  // 6) トレイ: デジタル → 両方に反映、固定の切替
  await main.evaluate(() => __mock.emitAll('tray', 'digital'));
  await main.evaluate(() => __mock.emitAll('tray', 'lock'));
  await set.waitForTimeout(800);
  console.log('6 トレイ main', JSON.stringify((({ mode, locked }) => ({ mode, locked }))(await st(main))), 'settings', await set.evaluate(() => [document.querySelector('input[name=mode]:checked').value, document.getElementById('lockPos').checked]));

  // 7) 動作状態の試験（ゲーム中）を設定画面から
  await set.click('label:has(> input[name=power][value=game])');
  await main.waitForTimeout(400);
  console.log('7 試験 ゲーム中 main off', (await st(main)).clockOff);
  await set.click('label:has(> input[name=power][value=auto])');
  await main.waitForTimeout(400);
  console.log('  自動に戻す main off', (await st(main)).clockOff);

  // 8) 読み込み
  await set.evaluate(() => { window.__importJson = JSON.stringify({ v: { sec: true, analogSize: '50' }, r: { mode: 'analog' }, tc: {} }) });
  await set.click('#cfgImport');
  await main.waitForTimeout(800);
  console.log('8 読み込み main', JSON.stringify((({ mode, sec }) => ({ mode, sec }))(await st(main))), 'settings msg', await set.textContent('#sysOk'));

  // 9) 起動時はファイルを正とする
  await main.evaluate(() => { const o = JSON.parse(localStorage.getItem('__file')); o.r.mode = 'digital'; o.v.sec = false; localStorage.setItem('__file', JSON.stringify(o)); });
  await main.reload(); await main.waitForTimeout(1200);
  console.log('9 ファイル優先 main', JSON.stringify((({ mode, sec }) => ({ mode, sec }))(await st(main))));

  // 10) 自動起動・モニター
  await set.check('#autoStart'); await set.waitForTimeout(200);
  console.log('10 自動起動', await set.textContent('#sysOk'), '| モニター', JSON.stringify(await set.evaluate(() => [...monSel.options].map(o => o.text))));
  await set.selectOption('#monSel', '1'); await set.click('#monMove'); await set.waitForTimeout(100);
  console.log('   移動', JSON.stringify(await set.evaluate(() => __mock.log.filter(l => l.startsWith('move_')))));
  await set.screenshot({ path: out + '/p5_settings2.png' });
  console.log('ERRORS', JSON.stringify(errs)); check(errs.length === 0, 'ページでエラーが出た');
  await b.close();
})();
