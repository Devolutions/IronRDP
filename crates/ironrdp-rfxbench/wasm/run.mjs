// Drives the ironrdp-rfxbench WASM module and reports RemoteFX decode
// throughput, so the browser target can be measured with the same workload as
// the native criterion bench.
//
//   node run.mjs <module.wasm> [more.wasm ...]
//
// Each module is labelled with whether it was built with wasm SIMD enabled, so
// a baseline and a +simd128 build can be compared side by side.

import { readFile } from 'node:fs/promises';
import { basename } from 'node:path';
import wabtInit from 'wabt';

const wabt = await wabtInit();

const PATTERNS = ['desktop', 'text', 'gradient', 'photo', 'noise'];
const SIZES = [
  [1280, 720],
  [1920, 1080],
];

// Enough repetitions that timer granularity and JIT noise wash out, but short
// enough to keep a full sweep to a couple of minutes.
const WARMUP_MS = 300;
const BATCH_MS = 150;
const BATCHES = 9;

async function load(path) {
  const bytes = await readFile(path);
  const { instance } = await WebAssembly.instantiate(bytes, {});
  return { path, exports: instance.exports, simd: countSimd(bytes) };
}

// Disassembles the module and counts actual SIMD instructions.
// This checks that vector instructions really made it into the binary.
function countSimd(bytes) {
  const mod = wabt.readWasm(new Uint8Array(bytes), { simd: true, readDebugNames: false });
  const wat = mod.toText({ foldExprs: false, inlineExport: false });
  mod.destroy();
  const counts = new Map();
  // SIMD mnemonics are exactly those containing a lane-type token or `v128`.
  const re = /\b(v128\.\w+|[iuf]?(?:8x16|16x8|32x4|64x2)\.\w+|i(?:8x16|16x8|32x4|64x2)\.\w+|f(?:32x4|64x2)\.\w+)/g;
  let total = 0;
  for (const m of wat.matchAll(re)) {
    total++;
    counts.set(m[1], (counts.get(m[1]) ?? 0) + 1);
  }
  const top = [...counts.entries()].sort((a, b) => b[1] - a[1]).slice(0, 6);
  return { total, top };
}

function timeBatch(decode, iters) {
  const start = performance.now();
  let sink = 0;
  for (let i = 0; i < iters; i++) sink += decode();
  const elapsed = performance.now() - start;
  return { elapsed, sink };
}

function measure(exports) {
  const decode = exports.bench_decode_frame;

  // Warm up and simultaneously calibrate the batch size.
  let iters = 1;
  const warmStart = performance.now();
  while (performance.now() - warmStart < WARMUP_MS) {
    timeBatch(decode, iters);
    iters *= 2;
  }

  // Size a batch to run for about BATCH_MS.
  const probe = timeBatch(decode, 8);
  const perIter = probe.elapsed / 8;
  const batchIters = Math.max(1, Math.round(BATCH_MS / Math.max(perIter, 1e-4)));

  const samples = [];
  for (let b = 0; b < BATCHES; b++) {
    const { elapsed } = timeBatch(decode, batchIters);
    samples.push(elapsed / batchIters);
  }
  samples.sort((a, b) => a - b);
  return {
    median: samples[Math.floor(samples.length / 2)],
    best: samples[0],
  };
}

const modules = [];
for (const path of process.argv.slice(2)) modules.push(await load(path));

if (modules.length === 0) {
  console.error('usage: node run.mjs <module.wasm> [more.wasm ...]');
  process.exit(2);
}

console.log('module                          simd128  SIMD instrs  most frequent');
for (const m of modules) {
  const flag = m.exports.bench_has_simd128() ? 'yes' : 'no ';
  const top = m.simd.top.map(([k, v]) => `${k}x${v}`).join(' ');
  console.log(`${basename(m.path).padEnd(31)} ${flag}      ${String(m.simd.total).padEnd(12)} ${top}`);
}
console.log();

const header = ['resolution', 'pattern'].concat(modules.map((m) => basename(m.path, '.wasm')));
const rows = [];

for (const [w, h] of SIZES) {
  for (const pattern of PATTERNS) {
    const cells = [];
    for (const m of modules) {
      const wire = m.exports.bench_setup(PATTERNS.indexOf(pattern), w, h, 1);
      if (wire === 0) throw new Error(`setup failed for ${pattern} ${w}x${h}`);
      const { median } = measure(m.exports);
      const fps = 1000 / median;
      const mpx = (w * h) / median / 1000;
      cells.push({ median, fps, mpx });
    }
    rows.push({ res: `${w}x${h}`, pattern, cells });
  }
}

// ms/frame, frames/s, and megapixels/s
const w0 = 12;
const w1 = 10;
console.log(
  'resolution'.padEnd(w0) +
  'pattern'.padEnd(w1) +
  modules.map((m) => basename(m.path, '.wasm').padStart(22)).join('')
);
for (const row of rows) {
  let line = row.res.padEnd(w0) + row.pattern.padEnd(w1);
  for (const c of row.cells) {
    line += `${c.median.toFixed(2)}ms ${c.fps.toFixed(0)}fps ${c.mpx.toFixed(0)}Mpx/s`.padStart(22);
  }
  if (row.cells.length === 2) {
    const speedup = row.cells[0].median / row.cells[1].median;
    line += `   ${speedup.toFixed(2)}x`;
  }
  console.log(line);
}

if (modules.length === 2) {
  const ratios = rows.map((r) => r.cells[0].median / r.cells[1].median);
  ratios.sort((a, b) => a - b);
  const median = ratios[Math.floor(ratios.length / 2)];
  console.log(`\nmedian speedup of ${basename(modules[1].path, '.wasm')} over ${basename(modules[0].path, '.wasm')}: ${median.toFixed(2)}x`);
}

// Per-stage breakdown: says where the frame time goes.
// Times are per 4096-coefficient component (one third of a tile), except ycbcr which is per whole tile.
const STAGES = ['rlgr', 'subband', 'quantization', 'dwt', 'ycbcr'];
// Stages that restore their input buffer each iteration; the copy is measured
// separately and subtracted.
const RESTORES = new Set(['subband', 'quantization', 'dwt']);

if (modules.some((m) => m.exports.bench_stage_setup)) {
  console.log('\nper-stage, us per 4096-coefficient component (ycbcr: per 64x64 tile)');
  console.log(
    'pattern'.padEnd(10) +
    'stage'.padEnd(14) +
    modules.map((m) => basename(m.path, '.wasm').padStart(14)).join('') +
    (modules.length === 2 ? '        speedup' : '')
  );

  for (const pattern of ['desktop', 'text', 'noise']) {
    for (const stage of STAGES) {
      const cells = [];
      for (const m of modules) {
        if (!m.exports.bench_stage_setup) {
          cells.push(null);
          continue;
        }
        if (m.exports.bench_stage_setup(PATTERNS.indexOf(pattern), STAGES.indexOf(stage), 1) !== 1) {
          throw new Error(`stage setup failed for ${pattern}/${stage}`);
        }
        let us = measure({ bench_decode_frame: m.exports.bench_stage_run }).median * 1000;
        if (RESTORES.has(stage)) {
          const overhead = measure({ bench_decode_frame: m.exports.bench_stage_restore_only }).median * 1000;
          us = Math.max(0, us - overhead);
        }
        cells.push(us);
      }
      let line = pattern.padEnd(10) + stage.padEnd(14);
      for (const c of cells) line += (c === null ? '-' : c.toFixed(3)).padStart(14);
      if (cells.length === 2 && cells[0] !== null && cells[1] !== null && cells[1] > 0) {
        line += `        ${(cells[0] / cells[1]).toFixed(2)}x`;
      }
      console.log(line);
    }
  }
}
