"use strict";

const fs = require("node:fs");
const path = require("node:path");

const scratchRoot = path.join(__dirname, ".scratch");

function scratchWorkspace() {
  fs.mkdirSync(scratchRoot, { recursive: true });
  const directory = fs.mkdtempSync(path.join(scratchRoot, "workspace-"));
  return {
    directory,
    cleanup() {
      fs.rmSync(directory, { recursive: true, force: true });
      try {
        fs.rmdirSync(scratchRoot);
      } catch {
        // Another test may still be using the shared scratch parent.
      }
    },
  };
}

function write(root, relative, content) {
  const target = path.join(root, ...relative.split("/"));
  fs.mkdirSync(path.dirname(target), { recursive: true });
  fs.writeFileSync(target, content);
  return target;
}

module.exports = { scratchWorkspace, write };
