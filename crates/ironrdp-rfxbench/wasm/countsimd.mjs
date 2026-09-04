import { readFile } from 'node:fs/promises';
import { basename } from 'node:path';
import wabtInit from 'wabt';
const wabt = await wabtInit();
const re = /\b(v128\.\w+|i(?:8x16|16x8|32x4|64x2)\.\w+|f(?:32x4|64x2)\.\w+)/g;
for (const p of process.argv.slice(2)) {
  const bytes = await readFile(p);
  const mod = wabt.readWasm(new Uint8Array(bytes), { simd: true, readDebugNames: false });
  const wat = mod.toText({ foldExprs: false, inlineExport: false });
  mod.destroy();
  const counts = new Map();
  let total = 0;
  for (const m of wat.matchAll(re)) { total++; counts.set(m[1], (counts.get(m[1]) ?? 0) + 1); }
  const top = [...counts.entries()].sort((a,b)=>b[1]-a[1]).slice(0,5).map(([k,v])=>`${k}x${v}`).join(' ');
  console.log(`${basename(p).padEnd(28)} SIMD instrs: ${String(total).padEnd(7)} ${top}`);
}
