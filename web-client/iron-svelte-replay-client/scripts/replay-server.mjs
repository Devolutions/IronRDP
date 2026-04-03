#!/usr/bin/env node

// replay-server.mjs — Dev-only HTTP server for replay recording files.
//
// Serves a single binary file with HTTP Range request support and CORS headers,
// so the SvelteKit dev server (port 5173) can fetch it cross-origin.
//
// Usage:
//   node scripts/replay-server.mjs [--port 8000] [--file samples/sample.bin]
//
// Options:
//   --port <number>  Port to listen on (default: 8000)
//   --file <path>    Recording file to serve (default: samples/sample.bin)

import { createServer } from "node:http";
import { createReadStream, statSync } from "node:fs";
import { basename, resolve } from "node:path";
import { parseArgs } from "node:util";

const { values } = parseArgs({
  options: {
    port: { type: "string", default: "8000" },
    file: { type: "string", default: "samples/sample.bin" },
  },
});

const port = Number(values.port);
if (!Number.isFinite(port) || port < 1 || port > 65535) {
  process.stderr.write(`Error: invalid port: ${values.port}\n`);
  process.exit(1);
}

const filePath = resolve(values.file);

// Validate the file exists and is readable at startup.
let startupSize;
try {
  const stat = statSync(filePath);
  if (!stat.isFile()) {
    process.stderr.write(`Error: not a regular file: ${filePath}\n`);
    process.exit(1);
  }
  startupSize = stat.size;
} catch (err) {
  process.stderr.write(`Error: cannot access file: ${filePath}\n`);
  process.stderr.write(`  ${err.message}\n`);
  process.exit(1);
}

const urlPath = "/" + basename(filePath);

/** @param {string} header @param {number} size */
function parseRangeHeader(header, size) {
  if (!header.startsWith('bytes=')) return null;

  const spec = header.slice(6);

  // Reject multi-part ranges.
  if (spec.indexOf(",") !== -1) return null;

  // Find the first dash to split into left and right parts.
  const dashIndex = spec.indexOf("-");
  if (dashIndex === -1) return null;

  const left = spec.slice(0, dashIndex);
  const right = spec.slice(dashIndex + 1);

  let start;
  let end;

  if (left.length > 0 && right.length > 0) {
    // Both present: N-M (explicit start and end).
    start = Number(left);
    end = Number(right);
  } else if (left.length > 0 && right.length === 0) {
    // Right is empty: N- (open-ended).
    start = Number(left);
    end = size - 1;
  } else if (left.length === 0 && right.length > 0) {
    // Left is empty: -N (suffix range).
    const suffix = Number(right);
    if (!Number.isFinite(suffix) || suffix <= 0) return null;
    start = Math.max(0, size - suffix);
    end = size - 1;
  } else {
    // Both empty: just "-".
    return null;
  }

  // Validate all numbers.
  if (!Number.isFinite(start) || !Number.isFinite(end)) return null;
  if (start < 0 || end < 0) return null;

  return { start, end };
}

const CORS_HEADERS = {
  "Access-Control-Allow-Origin": "*",
  "Access-Control-Allow-Headers": "Range",
  "Access-Control-Expose-Headers": "Content-Range, Content-Length, Accept-Ranges",
};

const server = createServer((req, res) => {
  // CORS preflight.
  if (req.method === "OPTIONS") {
    res.writeHead(204, CORS_HEADERS);
    res.end();
    return;
  }

  // Only serve the derived URL path.
  if (req.url !== urlPath) {
    res.writeHead(404, CORS_HEADERS);
    res.end("Not Found\n");
    return;
  }

  // Re-stat the file on each request so size stays correct if the file is replaced.
  let fileSize;
  try {
    fileSize = statSync(filePath).size;
  } catch {
    res.writeHead(500, CORS_HEADERS);
    res.end("Internal Server Error: cannot stat file\n");
    return;
  }

  const commonHeaders = {
    ...CORS_HEADERS,
    "Accept-Ranges": "bytes",
    "Content-Type": "application/octet-stream",
  };

  const rangeHeader = req.headers["range"];

  if (!rangeHeader) {
    // Full file response.
    res.writeHead(200, {
      ...commonHeaders,
      "Content-Length": fileSize,
    });
    createReadStream(filePath).pipe(res);
    return;
  }

  // Range request.
  const range = parseRangeHeader(rangeHeader, fileSize);

  if (range === null || range.start > range.end || range.end >= fileSize) {
    res.writeHead(416, {
      ...commonHeaders,
      "Content-Range": `bytes */${fileSize}`,
    });
    res.end();
    return;
  }

  const contentLength = range.end - range.start + 1;

  res.writeHead(206, {
    ...commonHeaders,
    "Content-Range": `bytes ${range.start}-${range.end}/${fileSize}`,
    "Content-Length": contentLength,
  });
  createReadStream(filePath, { start: range.start, end: range.end }).pipe(res);
});

server.listen(port, () => {
  const sizeMB = (startupSize / (1024 * 1024)).toFixed(1);
  console.log(`Serving ${filePath} (${sizeMB} MB)`);
  console.log(`  http://localhost:${port}${urlPath}`);
});
