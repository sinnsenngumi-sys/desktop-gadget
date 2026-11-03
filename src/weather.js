/* フェーズ4：天気・警報・雨雲レーダー（仕様書§4）
 *
 * 通信はすべてRust側（weather.rs）で行い、ここは結果を受け取って描くだけ。
 * - 気温・湿度・天気: Open-Meteo（CC BY 4.0。出典をカードに表示）
 * - 警報・注意報: 気象庁（令和8年体系）。特別警報・危険警報・警報・注意報の順に2件まで、残りは「ほか○件」
 * - 雨雲レーダー: 気象庁の高解像度降水ナウキャストを、地理院タイル（淡色地図）に重ねる。
 *   雨の色は地図の色（ダーク／ライト）に合わせて塗り直す
 * 更新間隔は§7に従う（バッテリー時は倍、ゲーム中・ロック中は停止）。
 */
(function(){
  'use strict';
  var T=window.__TAURI__, C=window.GadgetCore, S=C.S;
  /* 設定画面の窓では、場所の検索と地域の選択だけを動かす（天気の取得とレーダーは、ガジェットの窓が行う） */
  var SET=!!window.GADGET_SETTINGS;
  var $=function(s){return document.querySelector(s)};
  var pad=function(n){return String(n).padStart(2,'0')};
  var inv=function(cmd,args){return T.core.invoke(cmd,args||{})};

  var W={wx:null,wxAt:0,wxErr:'',warn:null,warnAt:0,warnErr:'',area:null,
         times:null,timesAt:0,radarErr:'',frames:[],idx:0,hold:0,base:null,geoKey:'',busy:false};

  function hm(d){return pad(d.getHours())+':'+pad(d.getMinutes())}
  function errText(e){return String(e&&e.message||e)}
  function note(){
    var l=[];
    if(W.wxErr)l.push('天気: '+W.wxErr);
    if(W.warnErr)l.push('警報: '+W.warnErr);
    if(W.radarErr)l.push('レーダー: '+W.radarErr);
    $('#wxNote').textContent=l.join('\n');
  }
  function save(){if(window.GadgetSettings)window.GadgetSettings.save()}

  /* ---------- 場所 ---------- */
  function loc(){
    var lat=parseFloat($('#locLat').value),lon=parseFloat($('#locLon').value);
    if(!isFinite(lat)||!isFinite(lon))return null;
    return {name:$('#locName').value||(lat.toFixed(3)+', '+lon.toFixed(3)),lat:lat,lon:lon};
  }
  function inJmaDomain(p){return p&&p.lat>=20&&p.lat<=48&&p.lon>=118&&p.lon<=150}
  function renderLoc(){
    var p=loc(),a=$('#areaSel'),an=a.selectedIndex>=0&&a.value?a.options[a.selectedIndex].text:'';
    $('#locCur').textContent=p?('場所：'+p.name+'（'+p.lat.toFixed(3)+', '+p.lon.toFixed(3)+'）'+(an?'\n警報の地域：'+an:'')):'場所：未設定（地名で探すか、緯度・経度を入れてください）';
    $('#locCur').style.whiteSpace='pre-line';
  }
  function fillAreas(info){
    var sel=$('#areaSel');
    sel.innerHTML='';
    info.options.forEach(function(o){sel.add(new Option(o.name,o.code))});
    sel.value=info.code;
    W.area=info;
    renderLoc();
  }
  function setLocation(name,lat,lon){
    $('#locName').value=name||'';$('#locLat').value=String(lat);$('#locLon').value=String(lon);
    $('#locResults').innerHTML='';
    W.wx=null;W.wxAt=0;W.warn=null;W.frames=[];W.geoKey='';
    renderLoc();save();
    if(!T){if(!SET)refreshAll(true);return}
    inv('wx_find_area',{lat:lat,lon:lon}).then(function(info){
      if(!name)$('#locName').value=info.name;
      fillAreas(info);W.warnErr=info.note||'';
    }).catch(function(e){
      var sel=$('#areaSel');sel.innerHTML='';sel.add(new Option('（この場所では選べません）',''));
      W.warnErr='警報の地域を決められませんでした: '+errText(e);
    }).then(function(){save();renderLoc();note();if(!SET)refreshAll(true)});
  }
  $('#locSearch').onclick=function(){
    var q=$('#locQuery').value.trim(),box=$('#locResults');
    if(!q)return;
    if(!T){box.textContent='ブラウザでは検索できません';return}
    box.textContent='検索中…';
    inv('wx_geocode',{query:q}).then(function(list){
      box.innerHTML='';
      if(!list.length){box.textContent='見つかりませんでした。別の書き方（「札幌市」→「札幌」など）や、緯度・経度で試してください';return}
      list.forEach(function(p){
        var b=document.createElement('button');b.className='btn';b.type='button';
        b.textContent=p.name+(p.detail?'（'+p.detail+'）':'')+'  '+p.lat.toFixed(2)+', '+p.lon.toFixed(2);
        b.onclick=function(){setLocation(p.name,p.lat,p.lon)};
        box.appendChild(b);
      });
    }).catch(function(e){box.textContent='検索できませんでした: '+errText(e)});
  };
  $('#locQuery').addEventListener('keydown',function(e){if(e.key==='Enter')$('#locSearch').click()});
  $('#latlonSet').onclick=function(){
    var lat=parseFloat($('#latIn').value),lon=parseFloat($('#lonIn').value);
    if(!(lat>=-90&&lat<=90&&lon>=-180&&lon<=180)){$('#locResults').textContent='緯度は-90〜90、経度は-180〜180の数で入れてください';return}
    setLocation('',lat,lon);
  };
  $('#areaSel').addEventListener('change',function(){renderLoc();W.warn=null;fetchWarnings()});

  /* ---------- 気温・湿度・天気 ---------- */
  /* WMOの天気コード → 文言とアイコンの種類 */
  function wmo(c){
    var t={0:['快晴','clear'],1:['晴れ','clear'],2:['晴れ時々くもり','partly'],3:['くもり','cloud'],45:['霧','fog'],48:['霧（着氷）','fog'],
      51:['弱い霧雨','rain'],53:['霧雨','rain'],55:['強い霧雨','rain'],56:['着氷性の霧雨','rain'],57:['着氷性の霧雨','rain'],
      61:['弱い雨','rain'],63:['雨','rain'],65:['強い雨','rain'],66:['着氷性の雨','rain'],67:['着氷性の強い雨','rain'],
      71:['弱い雪','snow'],73:['雪','snow'],75:['強い雪','snow'],77:['霧雪','snow'],80:['弱いにわか雨','rain'],81:['にわか雨','rain'],
      82:['激しいにわか雨','rain'],85:['にわか雪','snow'],86:['強いにわか雪','snow'],95:['雷雨','thunder'],96:['ひょうを伴う雷雨','thunder'],99:['ひょうを伴う雷雨','thunder']};
    return t[c]||['—','cloud'];
  }
  function icon(kind,day){
    var sun='<circle cx="24" cy="24" r="10" fill="#FFD36B"/><g stroke="#FFD36B" stroke-width="3" stroke-linecap="round"><path d="M24 6v5M24 37v5M6 24h5M37 24h5M11 11l3.5 3.5M33.5 33.5L37 37M37 11l-3.5 3.5M14.5 33.5L11 37"/></g>';
    var moon='<path d="M30 8a16 16 0 1 0 14 24A13 13 0 0 1 30 8z" fill="#FFE9A8"/>';
    var cloud='<path d="M24 50a9 9 0 0 1 1-18 12 12 0 0 1 23 3 8 8 0 0 1-1 15z" fill="#fff" opacity=".92"/>';
    var bigCloud='<path d="M18 46a11 11 0 0 1 1-22 15 15 0 0 1 28 4 9 9 0 0 1-1 18z" fill="#fff" opacity=".92"/>';
    var drops='<g stroke="#8db3e3" stroke-width="3" stroke-linecap="round"><path d="M22 52l-3 7M32 52l-3 7M42 52l-3 7"/></g>';
    var flakes='<g fill="#e8f0ff"><circle cx="21" cy="55" r="2.6"/><circle cx="32" cy="58" r="2.6"/><circle cx="43" cy="55" r="2.6"/></g>';
    var bolt='<path d="M33 46l-7 10h6l-3 8 9-12h-6l3-6z" fill="#FFD36B"/>';
    var fog='<g stroke="#e5e7eb" stroke-width="3.5" stroke-linecap="round" opacity=".9"><path d="M12 26h40M8 36h44M14 46h36"/></g>';
    var orb=day?sun:moon;
    return {clear:orb,partly:orb+cloud,cloud:bigCloud,fog:fog,rain:'<g transform="translate(0,-6)">'+bigCloud+'</g>'+drops,
            snow:'<g transform="translate(0,-6)">'+bigCloud+'</g>'+flakes,thunder:'<g transform="translate(0,-8)">'+bigCloud+'</g>'+bolt}[kind]||bigCloud;
  }
  function fmtTemp(c){
    if(c==null||!isFinite(c))return '--°';
    var f=$('#tempUnit').value==='f';
    return Math.round(f?c*9/5+32:c)+(f?'°F':'°');
  }
  function intervalMin(){
    var m=+$('#wxMin').value||10,o=C.opt();
    return S.power==='battery'?m*o.mult:m;
  }
  function renderWeather(){
    var p=loc(),w=W.wx;
    if(!p){
      $('#wxPlace').textContent='場所が未設定です';$('#temp').textContent='--°';$('#hum').textContent='--%';
      $('#wxIcon').innerHTML=icon('partly',true);$('#wxIcon').setAttribute('aria-label','天気');
      $('#upd').textContent='「設定」→「天気」で場所を決めてください';$('#upd').classList.remove('stale');
      return;
    }
    var k=wmo(w?w.code:null);
    $('#wxPlace').textContent=p.name+(w?'・'+k[0]:'');
    $('#wxPlace').title=p.name+(w?'・'+k[0]:'');
    $('#temp').textContent=fmtTemp(w?w.temp_c:null);
    $('#hum').textContent=w&&w.humidity!=null?Math.round(w.humidity)+'%':'--%';
    $('#wxIcon').innerHTML=icon(w?k[1]:'cloud',w?w.is_day:true);
    $('#wxIcon').setAttribute('aria-label',w?k[0]:'天気');
    var u=$('#upd'),src=T?'天気: Open-Meteo.com（CC BY 4.0）':'（ブラウザ表示：ダミー値）';
    if(!W.wxAt){u.textContent=(W.wxErr?'天気を取得できません':'天気を取得中…')+' ／ '+src;u.classList.toggle('stale',!!W.wxErr);return}
    var at=new Date(W.wxAt),old=Date.now()-W.wxAt>(2*intervalMin()+1)*60000;
    u.textContent=(old||W.wxErr?'古いデータ（最終取得 '+hm(at)+'）':'最終更新 '+hm(at))+' ／ '+src;
    u.classList.toggle('stale',old||!!W.wxErr);
  }
  var wxBusy=false;
  function fetchWeather(){
    var p=loc();
    if(!p){renderWeather();return}
    if(!T){W.wx={temp_c:22,humidity:58,code:2,is_day:true};W.wxAt=Date.now();renderWeather();return}
    if(wxBusy)return;
    wxBusy=true;
    inv('wx_current',{lat:p.lat,lon:p.lon}).then(function(w){W.wx=w;W.wxAt=Date.now();W.wxErr=''})
      .catch(function(e){W.wxErr=errText(e)})
      .then(function(){wxBusy=false;renderWeather();note()});
  }

  /* ---------- 警報・注意報 ---------- */
  var KIND={special:'sp',danger:'danger',warning:'warn',advisory:'adv'};
  var TEST={
    none:[],
    adv:[{k:'adv',t:'強風注意報'},{k:'adv',t:'雷注意報'}],
    warn:[{k:'adv',t:'レベル2氾濫注意報'},{k:'warn',t:'レベル3大雨警報'},{k:'adv',t:'雷注意報'}],
    danger:[{k:'warn',t:'暴風警報'},{k:'danger',t:'レベル4土砂災害危険警報'},{k:'adv',t:'雷注意報'},{k:'adv',t:'波浪注意報'}],
    sp:[{k:'warn',t:'レベル3氾濫警報'},{k:'sp',t:'レベル5大雨特別警報'},{k:'danger',t:'レベル4土砂災害危険警報'}]
  };
  var RANK={sp:0,danger:1,warn:2,adv:3};
  function esc(s){return String(s).replace(/[&<>"]/g,function(c){return {'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;'}[c]})}
  function renderWarns(){
    var box=$('#warns');
    box.style.display=S.warnOn?'':'none';
    if(!S.warnOn)return;
    var set=$('#warnSel').value,list,h='';
    if(set!=='real'){list=TEST[set]||[]}
    else if(!T){list=TEST.adv}
    else if(!W.warn){
      box.innerHTML='<span class="w none">'+(!$('#areaSel').value?'警報の地域が未設定':(W.warnErr?'警報を取得できません':'警報を取得中…'))+'</span>';
      return;
    }else{list=W.warn.items.map(function(i){return {k:KIND[i.kind]||'adv',t:i.name}})}
    list=list.slice().sort(function(a,b){return RANK[a.k]-RANK[b.k]});
    if(!list.length){h='<span class="w none">警報・注意報なし</span>'}
    else{
      list.slice(0,2).forEach(function(x){h+='<span class="w '+x.k+'">'+esc(x.t)+'</span>'});
      if(list.length>2)h+='<span class="w more" title="'+esc(list.slice(2).map(function(x){return x.t}).join('、'))+'">ほか'+(list.length-2)+'件</span>';
    }
    if(set==='real'&&T&&W.warn&&(W.warnErr||Date.now()-W.warnAt>(2*intervalMin()+1)*60000))h+='<span class="w none">（古いデータ）</span>';
    box.innerHTML=h;
    if(set==='real'&&W.warn){
      var r=W.warn.report_time?new Date(W.warn.report_time):null;
      box.title=W.warn.area_name+'（'+W.warn.office_name+'）'+(r&&!isNaN(r)?' '+hm(r)+' 発表':'')+'。出典：気象庁ホームページ';
    }else box.title='';
  }
  var warnBusy=false;
  function fetchWarnings(){
    var code=$('#areaSel').value;
    if(!T||!S.warnOn||!code){renderWarns();return}
    if(warnBusy)return;
    warnBusy=true;
    inv('wx_warnings',{code:code}).then(function(w){W.warn=w;W.warnAt=Date.now();W.warnErr=''})
      .catch(function(e){W.warnErr=errText(e)})
      .then(function(){warnBusy=false;renderWarns();note()});
  }

  /* ---------- 雨雲レーダー ---------- */
  var rc=$('#radar'),rx=rc.getContext('2d'),Wd=0,Ht=0,bw=0,bh=0;
  /* 気象庁タイルの色（強さの順。0.25〜1、1〜5、5〜10、10〜20、20〜30、30〜50、50〜80、80以上 mm/h） */
  var JMA=[[242,242,255],[160,210,255],[33,140,255],[0,65,255],[250,245,0],[255,153,0],[255,40,0],[180,0,104]];
  /* 地図の色ごとの雨の色（ダークでは明るめ、ライトでは濃いめ。弱い雨も地図に埋もれないようにする） */
  var PAL={
    dark:[[150,170,215,190],[141,179,227,225],[79,134,224,240],[47,95,224,250],[250,204,21,255],[251,146,60,255],[239,68,68,255],[217,70,239,255]],
    light:[[150,175,230,200],[111,155,230,230],[47,107,219,245],[29,63,191,250],[234,179,8,255],[249,115,22,255],[220,38,38,255],[162,28,175,255]]
  };
  function legend(){
    var p=PAL[S.map]||PAL.dark;
    /* 1行に収まる長さにする。加工した旨は規約に従って明記する */
    $('#rattr').textContent=S.map==='light'?'出典：気象庁ホームページ（色を加工）、地理院タイル':'出典：気象庁ホームページ、地理院タイル（いずれも色を加工）';
    $('#rlegend').style.background='linear-gradient(90deg,'+p.map(function(c){return 'rgb('+c[0]+','+c[1]+','+c[2]+')'}).join(',')+')';
  }
  function zoom(){return +$('#radarZoom').value||8}
  /* 緯度経度 → ズームzの世界座標（px） */
  function world(lat,lon,z){
    var n=256*Math.pow(2,z),s=Math.sin(Math.max(-85,Math.min(85,lat))*Math.PI/180);
    return {x:(lon+180)/360*n,y:(0.5-Math.log((1+s)/(1-s))/(4*Math.PI))*n};
  }
  function resize(){
    Wd=rc.clientWidth;Ht=rc.clientHeight;
    if(!Wd||!Ht)return false;
    var r=rc.getBoundingClientRect(),dpr=window.devicePixelRatio||1;
    var nw=Math.max(1,Math.round(r.width*dpr)),nh=Math.max(1,Math.round(r.height*dpr));
    if(nw!==bw||nh!==bh){bw=nw;bh=nh;rc.width=bw;rc.height=bh}
    var key=geoKey();
    if(key!==W.geoKey){W.geoKey=key;W.base=null;W.frames.forEach(function(f){f.canvas=null});rebuild()}
    else draw();
    return true;
  }
  function geoKey(){var p=loc();return p?[p.lat,p.lon,zoom(),Wd,Ht,bw,bh,S.map].join('|'):''}
  function view(){
    var p=loc(),z=zoom(),R=z+1,c=world(p.lat,p.lon,R);
    return {z:z,R:R,left:c.x-Wd/2,top:c.y-Ht/2};
  }
  function bitmapFrom(buf){
    var blob=new Blob([buf instanceof ArrayBuffer?buf:new Uint8Array(buf)],{type:'image/png'});
    return createImageBitmap(blob);
  }
  /* 同時に取りに行くタイルは4枚まで */
  var q=[],active=0;
  function limited(fn){
    return new Promise(function(ok,ng){
      q.push(function(){active++;fn().then(ok,ng).then(function(){active--;next()})});
      next();
    });
  }
  function next(){while(active<4&&q.length)q.shift()()}

  /* 背景の地図（地理院タイル、淡色地図）。ダークでは明暗を反転して暗い地図にする */
  function buildBase(v){
    var c=document.createElement('canvas');c.width=bw;c.height=bh;
    var x=c.getContext('2d'),sx=bw/Wd,sy=bh/Ht;
    x.fillStyle=S.map==='light'?'#eef0ee':'#1d1f22';x.fillRect(0,0,bw,bh);
    var t0x=Math.floor(v.left/256),t1x=Math.floor((v.left+Wd-1)/256),t0y=Math.floor(v.top/256),t1y=Math.floor((v.top+Ht-1)/256);
    var n=Math.pow(2,v.R),jobs=[],fail=0;
    for(var tx=t0x;tx<=t1x;tx++)for(var ty=t0y;ty<=t1y;ty++){
      (function(tx,ty){
        if(ty<0||ty>=n)return;
        var wx=((tx%n)+n)%n;
        jobs.push(limited(function(){return inv('wx_base_tile',{z:v.R,x:wx,y:ty})}).then(bitmapFrom).then(function(bm){
          x.save();
          if(S.map!=='light')x.filter='invert(1) hue-rotate(180deg) brightness(.82) contrast(.9) saturate(.55)';
          x.drawImage(bm,(tx*256-v.left)*sx,(ty*256-v.top)*sy,256*sx,256*sy);
          x.restore();
        }).catch(function(){fail++}));
      })(tx,ty);
    }
    return Promise.all(jobs).then(function(){c._fail=fail;return c});
  }
  /* 雨のタイルを、地図の色に合わせて塗り直す */
  function recolor(bm){
    var c=document.createElement('canvas');c.width=bm.width;c.height=bm.height;
    var x=c.getContext('2d',{willReadFrequently:true});x.drawImage(bm,0,0);
    var d=x.getImageData(0,0,c.width,c.height),a=d.data,P=PAL[S.map]||PAL.dark,memo={};
    for(var i=0;i<a.length;i+=4){
      if(a[i+3]<16){a[i+3]=0;continue}
      var key=(a[i]<<16)|(a[i+1]<<8)|a[i+2],lv=memo[key];
      if(lv===undefined){
        lv=-1;var best=1e9;
        for(var j=0;j<JMA.length;j++){
          var dr=a[i]-JMA[j][0],dg=a[i+1]-JMA[j][1],db=a[i+2]-JMA[j][2],dd=dr*dr+dg*dg+db*db;
          if(dd<best){best=dd;lv=j}
        }
        if(best>8000)lv=-1;
        memo[key]=lv;
      }
      if(lv<0){a[i+3]=0;continue}
      a[i]=P[lv][0];a[i+1]=P[lv][1];a[i+2]=P[lv][2];a[i+3]=P[lv][3];
    }
    x.putImageData(d,0,0);
    return c;
  }
  /* 1コマ分の雨の絵（画面の論理サイズ。粒が見やすいよう、拡大はぼかさない） */
  function buildFrame(f,v){
    var c=document.createElement('canvas');c.width=Wd;c.height=Ht;
    var x=c.getContext('2d');x.imageSmoothingEnabled=false;
    var S2=512;   /* ズームzのタイルを、1つ上のズームの大きさで描く */
    var t0x=Math.floor(v.left/S2),t1x=Math.floor((v.left+Wd-1)/S2),t0y=Math.floor(v.top/S2),t1y=Math.floor((v.top+Ht-1)/S2);
    var n=Math.pow(2,v.z),jobs=[],fail=0;
    for(var tx=t0x;tx<=t1x;tx++)for(var ty=t0y;ty<=t1y;ty++){
      (function(tx,ty){
        if(ty<0||ty>=n||tx<0||tx>=n)return;
        jobs.push(limited(function(){return inv('wx_radar_tile',{basetime:f.basetime,validtime:f.validtime,z:v.z,x:tx,y:ty})})
          .then(bitmapFrom).then(function(bm){x.drawImage(recolor(bm),tx*S2-v.left,ty*S2-v.top,S2,S2)})
          .catch(function(){fail++}));
      })(tx,ty);
    }
    return Promise.all(jobs).then(function(){f.fail=fail;f.total=jobs.length;return c});
  }
  function utc(s){return new Date(Date.UTC(+s.slice(0,4),+s.slice(4,6)-1,+s.slice(6,8),+s.slice(8,10),+s.slice(10,12),+s.slice(12,14)))}
  /* 表示するコマ: 実況は過去2時間を10分ごと、予測は60分先まで10分ごと */
  function pickFrames(t){
    if(!t||!t.past.length)return [];
    var past=t.past,last=utc(past[past.length-1].validtime).getTime(),out=[];
    past.forEach(function(f){
      var d=(last-utc(f.validtime).getTime())/60000;
      if(d<=120&&d%10===0)out.push({basetime:f.basetime,validtime:f.validtime,fc:false});
    });
    t.forecast.forEach(function(f){
      var d=(utc(f.validtime).getTime()-last)/60000;
      if(d>0&&d<=60&&d%10===0)out.push({basetime:f.basetime,validtime:f.validtime,fc:true,ahead:d});
    });
    return out;
  }
  function msg(t){$('#rmsg').textContent=t||''}
  /* 地図とコマを作り直す（場所・範囲・地図の色・大きさが変わったとき、新しいコマが来たとき） */
  var gen=0;
  function rebuild(){
    if(SET||!S.radar)return;
    var p=loc();
    if(!Wd){return}
    if(!p){msg('場所を設定すると表示します');draw();return}
    if(!inJmaDomain(p)){msg('雨雲レーダーは日本付近だけ表示できます');W.frames=[];draw();return}
    if(!T){msg('ブラウザでは表示できません（アプリで表示します）');draw();return}
    if(!W.times){draw();return}
    var my=++gen,v=view();
    legend();
    var want=pickFrames(W.times),old={};
    W.frames.forEach(function(f){old[f.basetime+f.validtime]=f});
    W.frames=want.map(function(f){var o=old[f.basetime+f.validtime];return o&&o.canvas?o:f});
    if(W.idx>=W.frames.length)W.idx=0;
    var pBase=W.base?Promise.resolve(W.base):buildBase(v).then(function(c){if(my===gen){W.base=c}return c});
    /* 最新の実況を先に作り、すぐ見えるようにする。次に予測、最後に古い実況 */
    var order=W.frames.map(function(f,i){return i});
    var liveIdx=W.frames.filter(function(f){return !f.fc}).length-1;
    order.sort(function(a,b){var ka=a===liveIdx?-1:(W.frames[a].fc?0:1),kb=b===liveIdx?-1:(W.frames[b].fc?0:1);return ka-kb||b-a});
    if(liveIdx>=0)W.idx=liveIdx;
    pBase.then(function(c){
      if(my!==gen)return;
      if(c._fail)W.radarErr='地図の一部を取得できませんでした（'+c._fail+'枚）';
      draw();
      return order.reduce(function(pr,i){
        return pr.then(function(){
          if(my!==gen)return;
          var f=W.frames[i];
          if(!f||f.canvas)return;
          return buildFrame(f,v).then(function(cv){if(my===gen){f.canvas=cv;if(i===W.idx)draw()}});
        });
      },Promise.resolve());
    }).then(function(){
      if(my!==gen)return;
      var bad=W.frames.filter(function(f){return f.total&&f.fail===f.total}).length;
      if(bad&&bad===W.frames.length){W.radarErr='雨雲のタイルを取得できませんでした';msg('雨雲を取得できません')}
      else if(bad){W.radarErr='雨雲のタイルを '+bad+'コマ分 取得できませんでした'}
      note();draw();
    });
  }
  function draw(){
    if(!bw)return;
    rx.setTransform(1,0,0,1,0,0);
    rx.clearRect(0,0,bw,bh);
    if(W.base)rx.drawImage(W.base,0,0);
    else{rx.fillStyle=S.map==='light'?'#eef0ee':'#1d1f22';rx.fillRect(0,0,bw,bh)}
    var f=W.frames[W.idx];
    rx.setTransform(bw/Wd,0,0,bh/Ht,0,0);
    if(f&&f.canvas){rx.imageSmoothingEnabled=false;rx.drawImage(f.canvas,0,0,Wd,Ht);rx.imageSmoothingEnabled=true}
    if(loc()&&inJmaDomain(loc())){
      var mx=Wd/2,my=Ht/2;
      rx.lineWidth=6;rx.strokeStyle='rgba(0,0,0,.65)';rx.beginPath();rx.arc(mx,my,8,0,6.2832);rx.stroke();
      rx.lineWidth=3;rx.strokeStyle='#ffffff';rx.beginPath();rx.arc(mx,my,8,0,6.2832);rx.stroke();
    }
    var chip=$('#rlabel'),n=W.frames.length;
    if(f){
      var t=utc(f.validtime);
      chip.textContent=hm(t)+(f.fc?'　予測 +'+f.ahead+'分':'　実況');
      $('#rprog').style.width=(n>1?W.idx/(n-1)*100:100)+'%';
    }else{chip.textContent='--:--';$('#rprog').style.width='0'}
    var stale=W.timesAt&&(W.radarErr&&!W.frames.length||Date.now()-W.timesAt>(2*5*(S.power==='battery'?C.opt().mult:1)+1)*60000);
    chip.classList.toggle('stale',!!stale);
    $('#rcap').textContent=stale?'古いデータ（'+hm(new Date(W.timesAt))+' 取得）':'雨雲レーダー（○は現在地）';
  }
  /* アニメ: 実況の最新と最後のコマで少し止まる */
  function step(){
    var n=W.frames.length;
    if(n<2)return;
    if(W.hold>0){W.hold--;return}
    var tries=0;
    do{W.idx=(W.idx+1)%n;tries++}while(!W.frames[W.idx].canvas&&tries<n);
    var f=W.frames[W.idx],nx=W.frames[W.idx+1];
    if(!f.fc&&(!nx||nx.fc))W.hold=2;
    if(W.idx===n-1)W.hold=3;
    draw();
  }
  var timesBusy=false;
  function fetchRadar(force){
    if(!S.radar||!T)return rebuild();
    var p=loc();
    if(!p||!inJmaDomain(p))return rebuild();
    /* 気象庁への負荷に配慮し、5分より短い間隔では取りに行かない（§4） */
    if(!force&&W.times&&Date.now()-W.timesAt<4.5*60000)return rebuild();
    if(timesBusy)return;
    timesBusy=true;
    inv('wx_radar_times').then(function(t){W.times=t;W.timesAt=Date.now();W.radarErr=t.note||'';msg('')})
      .catch(function(e){W.radarErr=errText(e);if(!W.frames.length)msg('雨雲レーダーを取得できません')})
      .then(function(){timesBusy=false;note();rebuild()});
  }

  /* ---------- 更新の予定（§7） ---------- */
  var tW=null,tR=null,tA=null;
  function schedule(){
    clearInterval(tW);clearInterval(tR);clearInterval(tA);
    if(SET||C.stopped())return;
    var mult=S.power==='battery'?C.opt().mult:1;
    tW=setInterval(function(){fetchWeather();fetchWarnings();renderWeather()},intervalMin()*60000);
    if(S.radar){
      tR=setInterval(function(){fetchRadar(false)},5*mult*60000);
      tA=setInterval(step,S.power==='battery'?1400:700);
    }
  }
  var lastAll=0;
  function refreshAll(force){
    if(SET)return;
    /* ゲーム終了・スリープ復帰・起動時。同じ操作で続けて呼ばれたときだけ、10秒以内の重複を省く */
    if(!force&&Date.now()-lastAll<10000){renderWeather();renderWarns();return}
    lastAll=Date.now();
    fetchWeather();fetchWarnings();
    if(S.radar)fetchRadar(!!force);
  }

  /* ---------- 設定の変更 ---------- */
  $('#tempUnit').addEventListener('change',renderWeather);
  $('#wxMin').addEventListener('change',function(){schedule();renderWeather()});
  $('#radarZoom').addEventListener('change',function(){resize()});
  $('#warnOn').addEventListener('change',function(){if(S.warnOn&&!W.warn)fetchWarnings()});

  window.GadgetWx={
    refreshAll:function(){refreshAll(false)},
    renderWarns:renderWarns,
    schedule:schedule,
    resize:function(){if(S.radar)resize()},
    draw:draw,
    radarToggled:function(){
      schedule();
      if(S.radar){requestAnimationFrame(function(){resize();fetchRadar(false)})}
    },
    mapChanged:function(){legend();if(S.radar)resize()},
    /* 設定画面で変わった設定を取り込んだあと。場所が変わっていたら、取り直す */
    settingsReloaded:function(){
      var k=locKey();
      renderLoc();
      if(SET||k===lastLocKey){renderWeather();renderWarns();return}
      lastLocKey=k;
      W.wx=null;W.wxAt=0;W.warn=null;W.frames=[];W.geoKey='';W.base=null;
      var code=$('#areaSel').value;
      if(T&&code)inv('wx_area_info',{code:code}).then(fillAreas).catch(function(){});
      renderWeather();renderWarns();
      refreshAll(true);
      if(S.radar)resize();
    },
    /* 試験用 */
    _state:W
  };

  function locKey(){var p=loc();return p?p.lat+','+p.lon+'|'+$('#areaSel').value:''}
  var lastLocKey=locKey();

  /* ---------- 起動 ---------- */
  S.warnOn=$('#warnOn').checked;S.warnSet=$('#warnSel').value;S.radar=$('#radarOn').checked;S.map=$('#mapSel').value;
  $('#radarBox').style.display=S.radar?'':'none';
  legend();renderLoc();renderWeather();renderWarns();
  if(T){
    inv('wx_sources').then(function(s){$('#wxSrc').textContent='取得先の設定ファイル: '+s.path+(s.note?'\n'+s.note:'');$('#wxSrc').style.whiteSpace='pre-line'}).catch(function(){});
    var code=$('#areaSel').value;
    if(code)inv('wx_area_info',{code:code}).then(fillAreas).catch(function(e){W.warnErr=errText(e);note()});
  }
  if(SET){
    /* 設定画面では、警報の地域の名前と一覧だけ用意する */
    return;
  }
  schedule();
  refreshAll(true);
  if(S.radar)requestAnimationFrame(resize);
})();
