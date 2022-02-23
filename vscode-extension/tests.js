#!/usr/bin/env node

const assert = require("assert");
const extension = require("./extension.js");

test("example", () => {
  assert.equal(1, 1);
  assert.equal(2, 2);
});

function test(name, body) {
  try {
    body();
  } catch (err) {
    const message = `${name}:\n${err.message || err}`;
    throw new Error(message);
  }
}
