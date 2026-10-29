// Measure the same RemoteFX workloads as the native Criterion benchmark.
//
//   node run.mjs <module.wasm>
//
// Build and run with SIMD using `cargo xtask bench rfx-decode-wasm --simd`.
// The report states whether the module has SIMD enabled.

import { readFile } from 'node:fs/promises';
import { basename } from 'node:path';
import wabtInit from 'wabt';

const PATTERNS = ['desktop', 'text', 'gradient', 'photo', 'noise'];
const SIZES = [
  [1280, 720],
  [1920, 1080],
];

// Per-stage breakdown: times are per 4096-coefficient component (one third of
// a tile), except ycbcr which is per whole tile.
const STAGES = ['rlgr', 'subband', 'quantization', 'dwt', 'ycbcr'];
const STAGE_PATTERNS = ['desktop', 'text', 'noise'];

// Enough repetitions that timer granularity and JIT noise wash out, but short
// enough to keep a full run to a couple of minutes.
const WARMUP_MS = 300;
const BATCH_MS = 150;
const BATCHES = 9;

const path = process.argv[2];
if (process.argv.length !== 3) {
  console.error('usage: node run.mjs <module.wasm>');
  process.exit(2);
}

const bytes = await readFile(path);
const { instance } = await WebAssembly.instantiate(bytes, {});
const exports = instance.exports;

// Disassembles the module and counts actual SIMD instructions.
// This checks that vector instructions really made it into the binary.
async function countSimd() {
  const wabt = await wabtInit();
  const mod = wabt.readWasm(new Uint8Array(bytes), { simd: true, readDebugNames: false });
  const wat = mod.toText({ foldExprs: false, inlineExport: false });
  mod.destroy();
  // SIMD mnemonics are exactly those containing a lane-type token or `v128`.
  const re = /\b(v128\.\w+|i(?:8x16|16x8|32x4|64x2)\.\w+|f(?:32x4|64x2)\.\w+)/g;
  const counts = new Map();
  let total = 0;
  for (const m of wat.matchAll(re)) {
    total++;
    counts.set(m[1], (counts.get(m[1]) ?? 0) + 1);
  }
  const top = [...counts.entries()].sort((a, b) => b[1] - a[1]).slice(0, 6);
  return { total, top };
}

let checksum = 0;

function timeBatch(run, iters, prepare) {
  let elapsed = 0;
  let sink = 0;
  if (prepare) {
    // Each call processes a batch of independent inputs, prepared before timing.
    for (let i = 0; i < iters; i++) {
      prepare();
      const start = performance.now();
      sink = (sink + run()) >>> 0;
      elapsed += performance.now() - start;
    }
  } else {
    const start = performance.now();
    for (let i = 0; i < iters; i++) sink = (sink + run()) >>> 0;
    elapsed = performance.now() - start;
  }
  checksum = (checksum + sink) >>> 0;
  return elapsed;
}

// Returns the median and best time of `run`, in milliseconds per call.
function measure(run, prepare) {
  // Warm up.
  let iters = 1;
  const warmStart = performance.now();
  while (performance.now() - warmStart < WARMUP_MS) {
    timeBatch(run, iters, prepare);
    iters *= 2;
  }

  // Size a batch to run for about BATCH_MS.
  const perIter = timeBatch(run, 8, prepare) / 8;
  const batchIters = Math.max(1, Math.round(BATCH_MS / Math.max(perIter, 1e-4)));

  const samples = [];
  for (let b = 0; b < BATCHES; b++) {
    samples.push(timeBatch(run, batchIters, prepare) / batchIters);
  }
  samples.sort((a, b) => a - b);
  return { median: samples[Math.floor(samples.length / 2)], best: samples[0] };
}

const simd = await countSimd();
console.log(`module: ${basename(path)}`);
console.log(`simd128: ${exports.bench_has_simd128() ? 'yes' : 'no'}`);
console.log(`SIMD instructions: ${simd.total} ${simd.top.map(([k, v]) => `${k}x${v}`).join(' ')}`);

console.log('\nframe decode');
console.log(
  'resolution'.padEnd(12) +
  'pattern'.padEnd(10) +
  'median'.padStart(12) +
  'best'.padStart(12) +
  'fps'.padStart(8) +
  'Mpx/s'.padStart(8) +
  'wire KiB'.padStart(10)
);
for (const [w, h] of SIZES) {
  for (const pattern of PATTERNS) {
    const wire = exports.bench_setup(PATTERNS.indexOf(pattern), w, h, 1);
    if (wire === 0) throw new Error(`setup failed for ${pattern} ${w}x${h}`);
    const { median, best } = measure(exports.bench_decode_frame);
    console.log(
      `${w}x${h}`.padEnd(12) +
      pattern.padEnd(10) +
      `${median.toFixed(2)}ms`.padStart(12) +
      `${best.toFixed(2)}ms`.padStart(12) +
      (1000 / median).toFixed(0).padStart(8) +
      ((w * h) / median / 1000).toFixed(0).padStart(8) +
      (wire / 1024).toFixed(0).padStart(10)
    );
  }
}

console.log('\nper-stage, us per 4096-coefficient component (ycbcr: per 64x64 tile)');
console.log('pattern'.padEnd(10) + 'stage'.padEnd(14) + 'median'.padStart(12) + 'best'.padStart(12));
for (const pattern of STAGE_PATTERNS) {
  for (const stage of STAGES) {
    const batchSize = exports.bench_stage_setup(PATTERNS.indexOf(pattern), STAGES.indexOf(stage), 1);
    if (batchSize === 0) {
      throw new Error(`stage setup failed for ${pattern}/${stage}`);
    }
    const { median, best } = measure(exports.bench_stage_run, exports.bench_stage_prepare);
    console.log(
      pattern.padEnd(10) +
      stage.padEnd(14) +
      `${(median * 1000 / batchSize).toFixed(3)}us`.padStart(12) +
      `${(best * 1000 / batchSize).toFixed(3)}us`.padStart(12)
    );
  }
}

console.log(`\nchecksum: ${checksum}`);