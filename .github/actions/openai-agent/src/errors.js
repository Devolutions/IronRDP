"use strict";

class ActionError extends Error {
  constructor(code) {
    super(code);
    this.name = "ActionError";
    this.code = code;
  }
}

function fail(code) {
  throw new ActionError(code);
}

module.exports = { ActionError, fail };
