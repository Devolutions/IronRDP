// Headless harness for the IronRDP web (WASM) replay benchmark.
//
// Serves the repo root over HTTP (ES modules + WASM need http, not file://), loads the bench page in
// headless Chromium, runs `run_web_bench` over a capture, and verifies the framebuffer CRC32 against
// the recorded ground truth. Usage:
//   node run.mjs [--capture /bench-corpus/smoke.irdprec] [--passes 3]
// Exits 0 on checksum MATCH, 1 on mismatch/error.

import http from 'node:http';
import { readFile, stat } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { chromium } from 'playwright';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(HERE, '..', '..', '..'); // crates/ironrdp-web/bench-harness -> repo root

function arg(name, fallback) {
  const i = process.argv.indexOf(name);
  return i !== -1 && i + 1 < process.argv.length ? process.argv[i + 1] : fallback;
}
const capture = arg('--capture', '/bench-corpus/smoke.irdprec');
const passes = arg('--passes', '3');

const MIME = {
  '.html': 'text/html',
  '.js': 'text/javascript',
  '.mjs': 'text/javascript',
  '.wasm': 'application/wasm',
  '.json': 'application/json',
  '.irdprec': 'application/octet-stream',
};

const server = http.createServer(async (req, res) => {
  try {
    const urlPath = decodeURIComponent(new URL(req.url, 'http://localhost').pathname);
    const filePath = path.join(REPO_ROOT, urlPath);
    if (!filePath.startsWith(REPO_ROOT)) {
      res.writeHead(403).end('forbidden');
      return;
    }
    await stat(filePath);
    const body = await readFile(filePath);
    res.writeHead(200, { 'content-type': MIME[path.extname(filePath)] ?? 'application/octet-stream' });
    res.end(body);
  } catch {
    res.writeHead(404).end('not found');
  }
});

async function main() {
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const port = server.address().port;
  const pageUrl = `http://127.0.0.1:${port}/crates/ironrdp-web/bench-harness/index.html?capture=${encodeURIComponent(capture)}&passes=${passes}`;

  const expected = JSON.parse(await readFile(path.join(REPO_ROOT, capture.replace(/\.irdprec$/, '.checksum.json'))));

  const browser = await chromium.launch({ headless: true });
  let exitCode = 1;
  try {
    const page = await browser.newPage();
    page.on('console', (msg) => console.log(`[page] ${msg.text()}`));
    await page.goto(pageUrl, { waitUntil: 'load' });
    await page.waitForFunction(() => window.__BENCH_RESULT__ || window.__BENCH_ERROR__, null, { timeout: 120000 });

    const err = await page.evaluate(() => window.__BENCH_ERROR__);
    if (err) throw new Error(`bench error in page: ${err}`);

    const result = JSON.parse(await page.evaluate(() => window.__BENCH_RESULT__));
    const matches = result.canonicalChecksum === expected.crc32;
    console.log('web bench result:', JSON.stringify(result));
    console.log(
      `checksum: replay=${result.canonicalChecksum} expected=${expected.crc32} -> ${matches ? 'MATCH' : 'MISMATCH'}`,
    );
    console.log(
      `frames=${result.counts.frames} rects=${result.counts.rects} totalMs=${result.totalMs.toFixed(2)} ` +
        `stages(read/decode/extract/draw)=${result.stageMs.readPdu.toFixed(2)}/${result.stageMs.decode.toFixed(2)}/` +
        `${result.stageMs.extract.toFixed(2)}/${result.stageMs.draw.toFixed(2)}`,
    );
    exitCode = matches ? 0 : 1;
  } finally {
    await browser.close();
    server.close();
  }
  process.exit(exitCode);
}

main().catch((e) => {
  console.error(e);
  server.close();
  process.exit(1);
});
