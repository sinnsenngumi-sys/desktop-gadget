/* ネットワークのカード（追加カード）
 * 受信・送信の速さを数値と小さなグラフ（直近60回分）で出す。
 * 値はRust側の get_usage に含まれる net（物理的なアダプターの合計、バイト/秒）。
 */
(function(){
  'use strict';
  var $=function(s){return document.querySelector(s)};
  var N=60,H={rx:[],tx:[]},last=null;

  function unit(){var u=$('#netUnit');return u?u.value:'bit'}
  /* 表示の形: ビットなら kbps / Mbps / Gbps、バイトなら KB/s / MB/s */
  function fmt(bps){
    if(bps==null||!isFinite(bps))return '—';
    if(unit()==='byte'){
      var k=bps/1024;
      if(k<1000)return (k<10?k.toFixed(1):Math.round(k))+' KB/s';
      var m=k/1024;return (m<10?m.toFixed(1):Math.round(m))+' MB/s';
    }
    var b=bps*8/1000;
    if(b<1000)return Math.round(b)+' kbps';
    var mb=b/1000;
    if(mb<1000)return (mb<10?mb.toFixed(1):Math.round(mb))+' Mbps';
    return (mb/1000).toFixed(1)+' Gbps';
  }
  function color(name){return getComputedStyle(document.documentElement).getPropertyValue(name).trim()||'#fff'}

  function spark(cv,data,col){
    var w=cv.clientWidth,h=cv.clientHeight;
    if(!w||!h)return;
    var dpr=window.devicePixelRatio||1,bw=Math.round(w*dpr),bh=Math.round(h*dpr);
    if(cv.width!==bw||cv.height!==bh){cv.width=bw;cv.height=bh}
    var x=cv.getContext('2d');
    x.setTransform(bw/w,0,0,bh/h,0,0);
    x.clearRect(0,0,w,h);
    /* 下地（使用量の棒と同じ薄い白） */
    x.fillStyle='rgba(255,255,255,.14)';
    x.beginPath();if(x.roundRect)x.roundRect(0,0,w,h,8);else x.rect(0,0,w,h);x.fill();
    if(data.length<2)return;
    /* 目盛りは自動。最低でも 1Mbps（125KB/s）を上端にして、小さな通信が大げさに見えないようにする */
    var max=Math.max(125000,Math.max.apply(null,data))*1.1;
    var step=w/(N-1),off=(N-data.length)*step;
    x.save();
    x.beginPath();if(x.roundRect)x.roundRect(0,0,w,h,8);else x.rect(0,0,w,h);x.clip();
    x.beginPath();
    data.forEach(function(v,i){var px=off+i*step,py=h-2-(v/max)*(h-4);i?x.lineTo(px,py):x.moveTo(px,py)});
    x.lineTo(off+(data.length-1)*step,h);x.lineTo(off,h);x.closePath();
    x.globalAlpha=.35;x.fillStyle=col;x.fill();
    x.globalAlpha=1;x.beginPath();
    data.forEach(function(v,i){var px=off+i*step,py=h-2-(v/max)*(h-4);i?x.lineTo(px,py):x.moveTo(px,py)});
    x.strokeStyle=col;x.lineWidth=1.8;x.lineJoin='round';x.stroke();
    x.restore();
  }
  function draw(){
    var n=last;
    $('#vrx').textContent=fmt(n?n.rx_bps:null);
    $('#vtx').textContent=fmt(n?n.tx_bps:null);
    var name='';
    if(n){
      if(!n.up)name='つながっていません';
      else name=(n.name||'')+(n.up>1?'（ほか'+(n.up-1)+'つと合計）':'');
    }
    $('#netName').textContent=name;
    spark($('#srx'),H.rx,color('--rx'));
    spark($('#stx'),H.tx,color('--tx'));
  }
  function push(n){
    last=n||null;
    if(n&&n.rx_bps!=null&&n.tx_bps!=null){
      H.rx.push(n.rx_bps);H.tx.push(n.tx_bps);
      if(H.rx.length>N){H.rx.shift();H.tx.shift()}
    }
    draw();
  }
  var u=$('#netUnit');if(u)u.addEventListener('change',draw);
  window.addEventListener('resize',draw);
  window.GadgetNet={push:push,draw:draw,_h:H};
  draw();
})();
