"use strict";

const fs = require("node:fs");
const path = require("node:path");
const { TextDecoder } = require("node:util");

const { ActionError, fail } = require("./errors");
const {
  MAX_LINE_BYTES, MAX_LIST_ENTRIES, MAX_PATH_BYTES, MAX_QUERY_BYTES, MAX_READ_LINES,
  MAX_RECURSION_DEPTH, MAX_SEARCH_FILES, MAX_SEARCH_RESULTS, MAX_SOURCE_FILE_BYTES,
  MAX_TOOL_RESULT_BYTES,
} = require("./limits");

const decoder = new TextDecoder("utf-8", { fatal: true });

function isInside(parent, child) {
  const relative = path.relative(parent, child);
  return relative === "" || (!relative.startsWith(`..${path.sep}`) && relative !== ".." &&
    !path.isAbsolute(relative));
}

function normalizeRepositoryPath(value) {
  if (typeof value !== "string" || value.length === 0 ||
      Buffer.byteLength(value, "utf8") > MAX_PATH_BYTES ||
      value.includes("\\") || /[\u0000-\u001F\u007F]/.test(value) ||
      value.startsWith("/") || /^[A-Za-z]:/.test(value)) {
    fail("invalid path");
  }
  const segments = value.split("/");
  if (segments.some((segment) => segment === "" || segment === "." || segment === ".." ||
      segment.toLowerCase() === ".git")) {
    fail("invalid path");
  }
  return segments.join("/");
}

function decodeText(buffer) {
  if (buffer.includes(0)) fail("binary file is not readable");
  try {
    return decoder.decode(buffer);
  } catch {
    fail("file is not valid UTF-8 text");
  }
}

function boundJson(value) {
  const encoded = JSON.stringify(value);
  if (Buffer.byteLength(encoded, "utf8") > MAX_TOOL_RESULT_BYTES) {
    fail("tool result exceeds byte limit");
  }
  return encoded;
}

class WorkspaceSandbox {
  constructor(workspace, capabilities) {
    if (typeof workspace !== "string" || workspace.length === 0) fail("workspace is unavailable");
    let workspaceReal;
    try {
      workspaceReal = fs.realpathSync(workspace);
      if (!fs.statSync(workspaceReal).isDirectory()) fail("workspace is not a directory");
    } catch (error) {
      if (error instanceof ActionError) throw error;
      fail("workspace is unavailable");
    }
    this.workspace = workspaceReal;
    this.allowedRoots = [];
    this.allowedFiles = [];
    if (capabilities) {
      for (const entry of capabilities.allowedRoots) {
        const resolved = this.resolve(entry, "directory", false);
        this.allowedRoots.push(resolved);
      }
      for (const entry of capabilities.allowedFiles) {
        const resolved = this.resolve(entry, "file", false);
        this.allowedFiles.push(resolved);
      }
    }
  }

  resolve(repositoryPath, expectedType, requireCapability = true) {
    const relative = normalizeRepositoryPath(repositoryPath);
    const lexical = path.resolve(this.workspace, ...relative.split("/"));
    if (!isInside(this.workspace, lexical)) fail("path escapes workspace");

    let cursor = this.workspace;
    try {
      for (const segment of relative.split("/")) {
        cursor = path.join(cursor, segment);
        const metadata = fs.lstatSync(cursor);
        if (metadata.isSymbolicLink()) fail("symbolic links and junctions are not allowed");
      }
      const metadata = fs.lstatSync(lexical);
      if (expectedType === "file" && !metadata.isFile()) fail("path is not a regular file");
      if (expectedType === "directory" && !metadata.isDirectory()) fail("path is not a directory");
      if (!metadata.isFile() && !metadata.isDirectory()) fail("unsupported filesystem object");
      const real = fs.realpathSync(lexical);
      if (!isInside(this.workspace, real)) fail("path escapes workspace");
      const resolved = { lexical, real, relative };
      if (requireCapability && !this.isAllowed(resolved, expectedType)) {
        fail("path is outside allowed capabilities");
      }
      return resolved;
    } catch (error) {
      if (error instanceof ActionError) throw error;
      fail("path is unavailable");
    }
  }

  isAllowed(target, expectedType) {
    if (expectedType === "file" && this.allowedFiles.some((file) =>
      file.relative === target.relative && file.real === target.real)) {
      return true;
    }
    return this.allowedRoots.some((root) =>
      (target.relative === root.relative || target.relative.startsWith(`${root.relative}/`)) &&
      isInside(root.real, target.real));
  }

  readTrustedFile(repositoryPath, maxBytes) {
    const target = this.resolve(repositoryPath, "file", false);
    return this.readText(target, maxBytes);
  }

  readText(target, maxBytes = MAX_SOURCE_FILE_BYTES) {
    let descriptor;
    try {
      const currentReal = fs.realpathSync(target.lexical);
      if (currentReal !== target.real || !isInside(this.workspace, currentReal)) {
        fail("path changed during access");
      }
      const noFollow = fs.constants.O_NOFOLLOW || 0;
      descriptor = fs.openSync(target.real, fs.constants.O_RDONLY | noFollow);
      const metadata = fs.fstatSync(descriptor);
      if (!metadata.isFile()) fail("path is not a regular file");
      if (metadata.size > maxBytes) fail("file exceeds byte limit");
      return decodeText(fs.readFileSync(descriptor));
    } catch (error) {
      if (error instanceof ActionError) throw error;
      fail("file is unavailable");
    } finally {
      if (descriptor !== undefined) fs.closeSync(descriptor);
    }
  }

  readFile(args) {
    assertObject(args, ["path", "start_line", "end_line"]);
    if (typeof args.path !== "string") fail("read_file path must be a string");
    const start = args.start_line === undefined ? 1 : positiveInteger(args.start_line, "invalid start line");
    const requestedEnd = args.end_line === undefined ? start + MAX_READ_LINES - 1 :
      positiveInteger(args.end_line, "invalid end line");
    if (requestedEnd < start || requestedEnd - start + 1 > MAX_READ_LINES) fail("invalid line range");

    const target = this.resolve(args.path, "file");
    const lines = this.readText(target).split(/\r?\n/);
    if (start > lines.length) fail("start line exceeds file length");
    const end = Math.min(requestedEnd, lines.length);
    const selected = [];
    for (let index = start; index <= end; index++) {
      const line = lines[index - 1];
      if (Buffer.byteLength(line, "utf8") > MAX_LINE_BYTES) fail("source line exceeds byte limit");
      selected.push(`${index}: ${line}`);
    }
    return boundJson({
      ok: true,
      path: target.relative,
      start_line: start,
      end_line: end,
      truncated: end < lines.length,
      content: selected.join("\n"),
    });
  }

  listFiles(args) {
    assertObject(args, ["path", "recursive"]);
    if (typeof args.path !== "string" ||
        (args.recursive !== undefined && typeof args.recursive !== "boolean")) {
      fail("invalid list_files arguments");
    }
    const target = this.resolve(args.path, "directory");
    const entries = [];
    this.walk(target, args.recursive === true, ({ relative, metadata }) => {
      if (entries.length >= MAX_LIST_ENTRIES) return false;
      entries.push({ path: relative, type: metadata.isDirectory() ? "directory" : "file" });
      return true;
    });
    return boundJson({
      ok: true,
      path: target.relative,
      recursive: args.recursive === true,
      truncated: entries.length >= MAX_LIST_ENTRIES,
      entries,
    });
  }

  searchText(args) {
    assertObject(args, ["path", "query"]);
    if (typeof args.path !== "string" || typeof args.query !== "string" ||
        args.query.length === 0 || Buffer.byteLength(args.query, "utf8") > MAX_QUERY_BYTES ||
        /[\u0000-\u001F\u007F]/.test(args.query)) {
      fail("invalid search_text arguments");
    }

    let target;
    try {
      target = this.resolve(args.path, "file");
    } catch (fileError) {
      if (!(fileError instanceof ActionError) ||
          !["path is not a regular file", "unsupported filesystem object"].includes(fileError.code)) {
        throw fileError;
      }
      target = this.resolve(args.path, "directory");
    }

    const results = [];
    let filesSearched = 0;
    const searchFile = (file) => {
      if (filesSearched >= MAX_SEARCH_FILES || results.length >= MAX_SEARCH_RESULTS) return false;
      filesSearched++;
      let text;
      try {
        text = this.readText(file);
      } catch (error) {
        if (error instanceof ActionError &&
            ["binary file is not readable", "file is not valid UTF-8 text", "file exceeds byte limit"]
              .includes(error.code)) {
          return true;
        }
        throw error;
      }
      for (const [index, line] of text.split(/\r?\n/).entries()) {
        if (line.includes(args.query)) {
          if (Buffer.byteLength(line, "utf8") > MAX_LINE_BYTES) continue;
          results.push({ path: file.relative, line: index + 1, text: line });
          if (results.length >= MAX_SEARCH_RESULTS) return false;
        }
      }
      return true;
    };

    if (fs.lstatSync(target.real).isFile()) {
      searchFile(target);
    } else {
      this.walk(target, true, ({ lexical, real, relative, metadata }) => {
        if (!metadata.isFile()) return true;
        return searchFile({ lexical, real, relative });
      });
    }
    return boundJson({
      ok: true,
      path: target.relative,
      files_searched: filesSearched,
      truncated: filesSearched >= MAX_SEARCH_FILES || results.length >= MAX_SEARCH_RESULTS,
      matches: results,
    });
  }

  walk(root, recursive, visitor) {
    const visit = (directory, depth) => {
      if (depth > MAX_RECURSION_DEPTH) return true;
      let names;
      try {
        names = fs.readdirSync(directory.real).sort((left, right) => left.localeCompare(right, "en"));
      } catch {
        fail("directory is unavailable");
      }
      for (const name of names) {
        if (name.toLowerCase() === ".git") continue;
        let relative;
        try {
          relative = normalizeRepositoryPath(`${directory.relative}/${name}`);
        } catch (error) {
          if (error instanceof ActionError) continue;
          throw error;
        }
        const lexical = path.join(directory.lexical, name);
        let metadata;
        try {
          metadata = fs.lstatSync(lexical);
        } catch {
          continue;
        }
        if (metadata.isSymbolicLink() || (!metadata.isFile() && !metadata.isDirectory())) continue;
        const real = fs.realpathSync(lexical);
        if (!isInside(root.real, real) || !isInside(this.workspace, real)) continue;
        const entry = { lexical, real, relative, metadata };
        if (visitor(entry) === false) return false;
        if (recursive && metadata.isDirectory() &&
            visit({ lexical, real, relative }, depth + 1) === false) return false;
      }
      return true;
    };
    visit(root, 1);
  }
}

function assertObject(value, allowedKeys) {
  if (value === null || typeof value !== "object" || Array.isArray(value) ||
      Object.getPrototypeOf(value) !== Object.prototype ||
      Object.keys(value).some((key) => !allowedKeys.includes(key))) {
    fail("malformed tool arguments");
  }
}

function positiveInteger(value, code) {
  if (!Number.isSafeInteger(value) || value < 1) fail(code);
  return value;
}

module.exports = { WorkspaceSandbox, boundJson, normalizeRepositoryPath };
