// 版をまとめて変える: npm run set-version -- 1.2.0
//
// 版は3か所（package.json、src-tauri/Cargo.toml、src-tauri/tauri.conf.json）と、
// Cargo.lock の自分の項目にある。手で直すと食い違いやすいので、このスクリプトでそろえる。
// リリースの GitHub Actions は、タグ（v1.2.0）とこれらの版が一致しないと止まる。
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const ver = process.argv[2];

if (!ver || !/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(ver)) {
  console.error('使い方: npm run set-version -- <版>（例: 1.2.0）');
  process.exit(1);
}

function edit(rel, fn) {
  const p = join(root, rel);
  const before = readFileSync(p, 'utf8');
  const after = fn(before);
  // 変わらなかったとき: すでにその版なら何もしない。版の書き場所が見つからないなら止める
  if (after === before && !before.includes(ver)) throw new Error(`${rel} の版を見つけられませんでした`);
  writeFileSync(p, after);
  console.log(`  ${rel}`);
}

const json = (rel) =>
  edit(rel, (s) => s.replace(/("version"\s*:\s*")[^"]+(")/, `$1${ver}$2`));

console.log(`版を ${ver} にします:`);
json('package.json');
json('src-tauri/tauri.conf.json');
// Cargo.toml は [package] の version だけ（依存の version は変えない）
edit('src-tauri/Cargo.toml', (s) => s.replace(/(\[package\][^[]*?\nversion\s*=\s*")[^"]+(")/, `$1${ver}$2`));
edit('src-tauri/Cargo.lock', (s) => s.replace(/(\[\[package\]\]\nname = "desktop-gadget"\nversion = ")[^"]+(")/, `$1${ver}$2`));
// package-lock.json は、自分の項目（先頭と packages[""]）だけ
edit('package-lock.json', (s) => {
  const o = JSON.parse(s);
  o.version = ver;
  if (o.packages?.['']) o.packages[''].version = ver;
  return JSON.stringify(o, null, 2) + '\n';
});

console.log(`
次にすること:
  1. CHANGELOG.md に ${ver} の変更点を書く
  2. git commit -am "v${ver}"
  3. git tag v${ver}
  4. git push origin main --tags   （GitHub Actions がインストーラーを作り、Releases に下書きを置く）`);
