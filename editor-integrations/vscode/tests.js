#!/usr/bin/env node

const assert = require("assert");
const stream = require("stream");
const { listenOnSocket } = require("./extension.js");

const suite = () => {
  const fakeSocket = makeFakeSocket();
  const fakeVscode = makeFakeVscode();
  fakeVscode.vscode.workspace.textDocuments.push({
    languageId: "elm",
    fileName: "Existing.elm",
    getText: () => "elm!",
  });
  const deactivate = listenOnSocket(fakeVscode.vscode, fakeSocket.socket);

  test("upon activation send editor-id of 0 and initial open documents", () => {
    const chunk = fakeSocket.read();
    assert.equal(int32FromChunk(chunk), 0);

    assert.equal(int8FromChunk(fakeSocket.read()), 0);
    assert.equal(int32FromChunk(fakeSocket.read()), 0);
    assert.equal(int32FromChunk(fakeSocket.read()), "Existing.elm".length);
    assert.equal(stringFromChunk(fakeSocket.read()), "Existing.elm");
    assert.equal(int32FromChunk(fakeSocket.read()), "elm!".length);
    assert.equal(stringFromChunk(fakeSocket.read()), "elm!");
    assert.equal(fakeSocket.read(), undefined);
  });

  test("opening a non-elm file is ignored", () => {
    const document = { languageId: "md" };
    fakeVscode.simulateOpen(document);
    const chunk = fakeSocket.read();
    assert.equal(chunk, undefined);
  });

  test("opening Elm puts whole file source on socket", () => {
    const document = {
      languageId: "elm",
      fileName: "New.elm",
      getText: () => "abcd",
    };
    fakeVscode.simulateOpen(document);

    assert.equal(int8FromChunk(fakeSocket.read()), 0);
    assert.equal(int32FromChunk(fakeSocket.read()), 1);
    assert.equal(int32FromChunk(fakeSocket.read()), "New.elm".length);
    assert.equal(stringFromChunk(fakeSocket.read()), "New.elm");
    assert.equal(int32FromChunk(fakeSocket.read()), "abcd".length);
    assert.equal(stringFromChunk(fakeSocket.read()), "abcd");
    assert.equal(fakeSocket.read(), undefined);
  });

  test("change to non-elm file is ignored", () => {
    const change = { document: { languageId: "md" } };
    fakeVscode.simulateChange(change);
    const chunk = fakeSocket.read();
    assert.equal(chunk, undefined);
  });

  test("first change to elm file puts whole file source on socket", () => {
    const change = {
      document: {
        languageId: "elm",
        fileName: "Test.elm",
        getText: () => "abcd",
      },
    };
    fakeVscode.simulateChange(change);

    assert.equal(int8FromChunk(fakeSocket.read()), 0);
    assert.equal(int32FromChunk(fakeSocket.read()), 2);
    assert.equal(int32FromChunk(fakeSocket.read()), "Test.elm".length);
    assert.equal(stringFromChunk(fakeSocket.read()), "Test.elm");
    assert.equal(int32FromChunk(fakeSocket.read()), "abcd".length);
    assert.equal(stringFromChunk(fakeSocket.read()), "abcd");
    assert.equal(fakeSocket.read(), undefined);
  });

  test("second change to elm file reports diff on socket", () => {
    const change = {
      document: { languageId: "elm", fileName: "Test.elm" },
      contentChanges: [
        {
          range: {
            start: { line: 1, character: 2 },
            end: { line: 3, character: 4 },
          },
          text: "pqr",
        },
        {
          range: {
            start: { line: 5, character: 6 },
            end: { line: 7, character: 8 },
          },
          text: "xyz",
        },
      ],
    };
    fakeVscode.simulateChange(change);

    assert.equal(int8FromChunk(fakeSocket.read()), 1);
    assert.equal(int32FromChunk(fakeSocket.read()), 2);
    assert.equal(int8FromChunk(fakeSocket.read()), 1);
    assert.equal(int32FromChunk(fakeSocket.read()), 1);
    assert.equal(int32FromChunk(fakeSocket.read()), 2);
    assert.equal(int32FromChunk(fakeSocket.read()), 3);
    assert.equal(int32FromChunk(fakeSocket.read()), 4);
    assert.equal(int32FromChunk(fakeSocket.read()), "pqr".length);
    assert.equal(stringFromChunk(fakeSocket.read()), "pqr");

    assert.equal(int8FromChunk(fakeSocket.read()), 1);
    assert.equal(int32FromChunk(fakeSocket.read()), 2);
    assert.equal(int8FromChunk(fakeSocket.read()), 1);
    assert.equal(int32FromChunk(fakeSocket.read()), 5);
    assert.equal(int32FromChunk(fakeSocket.read()), 6);
    assert.equal(int32FromChunk(fakeSocket.read()), 7);
    assert.equal(int32FromChunk(fakeSocket.read()), 8);
    assert.equal(int32FromChunk(fakeSocket.read()), "xyz".length);
    assert.equal(stringFromChunk(fakeSocket.read()), "xyz");

    assert.equal(fakeSocket.read(), undefined);
  });

  test("refactor sent by elm-pair gets applied to editor", async () => {
    const refactorBuffer = Buffer.concat([
      int8ToChunk(0), // command id, indicating a refactor.
      int32ToChunk(2), // Number of included changes.

      // Change 1
      int32ToChunk("Test.elm".length),
      stringToChunk("Test.elm"),
      int32ToChunk(1),
      int32ToChunk(2),
      int32ToChunk(3),
      int32ToChunk(4),
      int32ToChunk("newCode".length),
      stringToChunk("newCode"),

      // Change 2
      int32ToChunk("Test2.elm".length),
      stringToChunk("Test2.elm"),
      int32ToChunk(5),
      int32ToChunk(6),
      int32ToChunk(7),
      int32ToChunk(8),
      int32ToChunk("moreCode".length),
      stringToChunk("moreCode"),
    ]);

    // Feed the data to the extension as individual bytes to stress-test logic
    // in extension responsible for blocking on limited data.
    for (const byte of refactorBuffer) {
      fakeSocket.push(Buffer.from([byte]));
    }

    const edit = await fakeVscode.recordedEdits.next();
    assert.deepEqual(edit.value.replacements, [
      {
        metadata: { label: "Change by Elm-pair", needsConfirmation: false },
        newText: "newCode",
        range: { end: { column: 4, line: 3 }, start: { column: 2, line: 1 } },
        uri: "uri:Test.elm",
      },
      {
        metadata: { label: "Change by Elm-pair", needsConfirmation: false },
        newText: "moreCode",
        range: { end: { column: 8, line: 7 }, start: { column: 6, line: 5 } },
        uri: "uri:Test2.elm",
      },
    ]);
  });

  test("command to open files sent by elm-pair is executed", async () => {
    const openFilesBuffer = Buffer.concat([
      int8ToChunk(1), // command id, indicating a an open files command.
      int32ToChunk(2), // Number of files to open.

      // File 1
      int32ToChunk("/My/Module.elm".length),
      stringToChunk("/My/Module.elm"),

      // File 2
      int32ToChunk("/My/SecondModule.elm".length),
      stringToChunk("/My/SecondModule.elm"),
    ]);

    // Feed the data to the extension as individual bytes to stress-test logic
    // in extension responsible for blocking on limited data.
    for (const byte of openFilesBuffer) {
      fakeSocket.push(Buffer.from([byte]));
    }

    const path1 = await fakeVscode.recordedOpenFiles.next();
    assert.deepEqual(path1.value, "/My/Module.elm");
    const path2 = await fakeVscode.recordedOpenFiles.next();
    assert.deepEqual(path2.value, "/My/SecondModule.elm");
  });

  test("command to show file sent by elm-pair is executed", async () => {
    const showFileBuffer = Buffer.concat([
      int8ToChunk(2), // command id, indicating a an open files command.
      int32ToChunk("/my/file.txt".length),
      stringToChunk("/my/file.txt"),
    ]);

    // Feed the data to the extension as individual bytes to stress-test logic
    // in extension responsible for blocking on limited data.
    for (const byte of showFileBuffer) {
      fakeSocket.push(Buffer.from([byte]));
    }

    const path1 = await fakeVscode.recordedShowFile.next();
    assert.deepEqual(path1.value, "uri:/my/file.txt");
  });

  test("deactivating plugin calls finishes the socket", async () => {
    deactivate();
    await new Promise((resolve, reject) => {
      fakeSocket.socket.on("finish", () => resolve());
    });
  });

  test("plugin thows an error if socket is closed unexpectedly", async () => {
    const { socket } = makeFakeSocket();
    listenOnSocket(fakeVscode.vscode, socket);
    socket.push(null);
    const { value: err } = await fakeVscode.recordedErrors.next();
    assert.equal(
      err,
      "Elm-pair crashed. A bug report will be much appreciated! You can submit this bug at https://github.com/jwoudenberg/elm-pair/issues. Error reads: Connection to elm-pair daemon closed."
    );
  });
};

// Test helpers.

function int8ToChunk(int8) {
  return Buffer.from([int8]);
}

function int32ToChunk(int32) {
  const buffer = Buffer.alloc(4);
  buffer.writeInt32BE(int32);
  return buffer;
}

function stringToChunk(string) {
  return Buffer.from(string, "utf8");
}

function int8FromChunk(buffer) {
  assert.equal(buffer.length, 1);
  return buffer.readInt8();
}

function int32FromChunk(buffer) {
  assert.equal(buffer.length, 4);
  return buffer.readInt32BE();
}

function stringFromChunk(buffer) {
  return buffer.toString("utf8");
}

function makeFakeSocket() {
  const written = [];

  const socket = new stream.Duplex({
    read() {},
    write(data, encoding, callback) {
      written.push(data);
      callback();
    },
  });

  function push(chunk) {
    socket.push(chunk);
  }

  function read() {
    return written.shift();
  }

  return {
    socket,
    push,
    read,
  };
}

function makeFakeVscode() {
  const editsStream = new stream.PassThrough({ objectMode: true });
  const openFilesStream = new stream.PassThrough({ objectMode: true });
  const showFileStream = new stream.PassThrough({ objectMode: true });
  const errorStream = new stream.PassThrough({ objectMode: true });
  const ret = {
    recordedEdits: editsStream[Symbol.asyncIterator](),
    recordedOpenFiles: openFilesStream[Symbol.asyncIterator](),
    recordedShowFile: showFileStream[Symbol.asyncIterator](),
    recordedErrors: errorStream[Symbol.asyncIterator](),
  };
  ret.vscode = {
    workspace: {
      textDocuments: [],
      onDidChangeTextDocument(callback) {
        ret.simulateChange = callback;
      },
      onDidOpenTextDocument(callback) {
        ret.simulateOpen = callback;
      },
      applyEdit(edit) {
        editsStream.write(edit);
      },
      openTextDocument(path) {
        openFilesStream.write(path);
      },
    },
    window: {
      showErrorMessage(err) {
        errorStream.write(err);
      },
      showTextDocument(path) {
        showFileStream.write(path);
      },
    },
    WorkspaceEdit,
    Uri: { file: (path) => `uri:${path}` },
    Position,
    Range,
  };
  return ret;
}

class WorkspaceEdit {
  constructor() {
    this.replacements = [];
  }
  replace(uri, range, newText, metadata) {
    this.replacements.push({ uri, range, newText, metadata });
  }
}

class Position {
  constructor(line, column) {
    return { line, column };
  }
}

class Range {
  constructor(start, end) {
    return { start, end };
  }
}

let testPromise = Promise.resolve();
function test(name, body) {
  // The tests in this file assume they're running sequentially
  testPromise = testPromise.then(async () => {
    try {
      await body();
    } catch (err) {
      const message = `${name}:\n${err.message || err}`;
      throw new Error(message);
    }
  });
}
suite();
Promise.race([
  testPromise,
  new Promise((resolve, reject) =>
    setTimeout(() => reject("Tests took too long."), 10)
  ),
]).catch((err) => {
  console.log(err);
  process.exit(1);
});
