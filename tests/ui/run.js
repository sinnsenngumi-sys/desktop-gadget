// 画面の自動テストを、すべて順に実行する（npm run test:ui）
const { spawnSync } = require('child_process');
const fs = require('fs'), path = require('path');
const files = fs.readdirSync(__dirname).filter(f => f.endsWith('.test.js')).sort();
let failed = [];
for (const f of files) {
  const t0 = Date.now();
  const r = spawnSync(process.execPath, [path.join(__dirname, f)], { encoding: 'utf8', timeout: 300000 });
  const ok = r.status === 0;
  console.log(`${ok ? 'OK ' : 'NG '} ${f}  (${((Date.now() - t0) / 1000).toFixed(1)}s)`);
  if (!ok) { failed.push(f); process.stdout.write((r.stdout || '') + (r.stderr || '')); }
}
console.log(`\n${files.length - failed.length}/${files.length} 件が通りました`);
if (failed.length) process.exit(1);
