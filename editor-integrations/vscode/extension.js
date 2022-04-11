const { Buffer } = require("buffer");
const cp = require("child_process");
const fs = require("fs");
const net = require("net");
const path = require("path");

const ELM_PAIR_NIX_PATH = "nix-build-put-path-to-elm-pair-here";

const MSG_NEW_FILE = 0;
const MSG_FILE_CHANGED = 1;

const CMD_REFACTOR = 0;
const CMD_OPEN_FILES = 1;

const EDIT_METADATA = {
  label: "Change by Elm-pair",
  needsConfirmation: false,
};

let deactivate_;
module.exports = {
  activate: async function activate(context) {
    const vscode = require("vscode");
    try {
      const socketPath = await getElmPairSocket(context);
      const socket = await connectToElmPair(socketPath);
      deactivate_ = listenOnSocket(vscode, socket);
    } catch (err) {
      reportError(vscode, err);
      throw err;
    }
  },
  deactivate: function deactivate() {
    deactivate_();
  },

  // Exported for testing.
  listenOnSocket,
};

function listenOnSocket(vscode, socket) {
  // Elm-pair expects a 4-byte editor-id. For Visual Studio Code it's 0.
  writeInt32(socket, 0);
  const elmFileIdsByPath = {};

  let refactorUnderway = false;
  const setRefactorUnderway = (val) => {
    refactorUnderway = val;
  };

  const processData = listenForCommands(vscode, setRefactorUnderway);
  processData.next(); // Run to first `yield` (moment we need data).
  socket.on("data", (data) => {
    processData.next(data);
  });

  let deactivating = false;
  socket.on("end", () => {
    if (!deactivating) {
      const err = new Error("Connection to elm-pair daemon closed.");
      reportError(vscode, err);
    }
  });

  vscode.workspace.onDidOpenTextDocument((doc) => {
    if (doc.languageId === "elm") {
      onNewElmFile(socket, doc, elmFileIdsByPath);
    }
  });

  vscode.workspace.onDidChangeTextDocument((changeEvent) => {
    const doc = changeEvent.document;
    if (doc.languageId !== "elm") {
      return;
    }
    const fileName = doc.fileName;
    let fileId = elmFileIdsByPath[fileName];
    if (typeof fileId === "undefined") {
      onNewElmFile(socket, doc, elmFileIdsByPath);
    } else {
      // reason 1 and 2 correspond to UNDO and REDO modifications respectively.
      // We don't want Elm-pair to respond to undo or redo changes, as it might
      // result in programmers getting stuck in a loop.
      const doNotRefactor =
        refactorUnderway ||
        changeEvent.reason === 1 ||
        changeEvent.reason === 2;
      for (const change of changeEvent.contentChanges) {
        const range = change.range;
        writeInt32(socket, fileId);
        writeInt8(socket, MSG_FILE_CHANGED);
        writeInt8(socket, doNotRefactor ? 0 : 1);
        writeInt32(socket, range.start.line);
        writeInt32(socket, range.start.character);
        writeInt32(socket, range.end.line);
        writeInt32(socket, range.end.character);
        writeString(socket, change.text);
      }
    }
  });

  // Tell Elm-pair about files that were open before this activation code ran.
  for (const doc of vscode.workspace.textDocuments) {
    if (doc.languageId === "elm") {
      onNewElmFile(socket, doc, elmFileIdsByPath);
    }
  }

  return function deactivate() {
    deactivating = true;
    socket.end();
  };
}

function onNewElmFile(socket, doc, elmFileIdsByPath) {
  const fileId = (elmFileIdsByPath[doc.fileName] =
    Object.keys(elmFileIdsByPath).length);
  writeInt32(socket, fileId);
  writeInt8(socket, MSG_NEW_FILE);
  writeString(socket, doc.fileName);
  writeString(socket, doc.getText());
}

async function reportError(vscode, err) {
  let message = err.message || err;
  await vscode.window.showErrorMessage(
    "Elm-pair crashed. A bug report will be much appreciated! You can submit this bug at https://github.com/jwoudenberg/elm-pair/issues. Error reads: " +
      message
  );
}

function getElmPairSocket(context) {
  return new Promise((resolve, reject) => {
    const elmPairBin = fs.existsSync(ELM_PAIR_NIX_PATH)
      ? ELM_PAIR_NIX_PATH
      : path.join(context.extensionPath, "elm-pair");
    cp.exec(elmPairBin, (err, stdout, stderr) => {
      if (stderr) {
        console.log(stderr);
      }
      if (err) {
        reject(err);
      } else {
        resolve(stdout);
      }
    });
  });
}

// Parse refactors streamed from Elm-pair and apply them to vscode files.
// This is a generator function so it can 'yield's when it needs more bytes.
async function* listenForCommands(vscode, setRefactorUnderway) {
  let buffer = yield;
  while (true) {
    [commandId, buffer] = yield* readInt8(buffer);
    switch (commandId) {
      case CMD_REFACTOR:
        buffer = yield* processRefactor(vscode, buffer, setRefactorUnderway);
        break;
      case CMD_OPEN_FILES:
        buffer = yield* processOpenFiles(vscode, buffer);
        break;
      default:
        await reportError(vscode, "Unknown command id: " + commandId);
        return;
    }
  }
}

async function* processRefactor(vscode, buffer, setRefactorUnderway) {
  const edit = new vscode.WorkspaceEdit();
  let editsInRefactor;

  [editsInRefactor, buffer] = yield* readInt32(buffer);

  for (let i = 0; i < editsInRefactor; i++) {
    let path, startLine, startCol, oldEndLine, oldEndCol, newText;
    [path, buffer] = yield* readString(buffer);
    [startLine, buffer] = yield* readInt32(buffer);
    [startCol, buffer] = yield* readInt32(buffer);
    [oldEndLine, buffer] = yield* readInt32(buffer);
    [oldEndCol, buffer] = yield* readInt32(buffer);
    [newText, buffer] = yield* readString(buffer);
    const uri = vscode.Uri.file(path);
    const startPosition = new vscode.Position(startLine, startCol);
    const endPosition = new vscode.Position(oldEndLine, oldEndCol);
    const range = new vscode.Range(startPosition, endPosition);
    edit.replace(uri, range, newText, EDIT_METADATA);
  }

  setRefactorUnderway(true);
  await vscode.workspace.applyEdit(edit);
  setRefactorUnderway(false);

  return buffer;
}

async function* processOpenFiles(vscode, buffer) {
  let amountOfFiles, path;

  [amountOfFiles, buffer] = yield* readInt32(buffer);

  for (let i = 0; i < amountOfFiles; i++) {
    [path, buffer] = yield* readString(buffer);
    await vscode.workspace.openTextDocument(path);
  }

  return buffer;
}

function* readInt8(buffer) {
  const [sample, newBuffer] = yield* takeFromBuffer(buffer, 1);
  const num = sample.readInt8();
  return [num, newBuffer];
}

function* readInt32(buffer) {
  const [sample, newBuffer] = yield* takeFromBuffer(buffer, 4);
  const num = sample.readInt32BE();
  return [num, newBuffer];
}

function* readString(buffer) {
  const [stringLength, buffer2] = yield* readInt32(buffer);
  const [sample, buffer3] = yield* takeFromBuffer(buffer2, stringLength);
  const string = sample.toString("utf8");
  return [string, buffer3];
}

// Read a specific number of bytes from a buffer, waiting for additional data if
// necessary.
function* takeFromBuffer(buffer, bytes) {
  while (buffer.length < bytes) {
    buffer = Buffer.concat([buffer, yield]);
  }
  const sample = buffer.subarray(0, bytes);
  const rest = buffer.subarray(bytes);
  return [sample, rest];
}

function connectToElmPair(socketPath) {
  return new Promise((resolve, reject) => {
    const socket = net.createConnection(socketPath);
    socket.on("connect", () => {
      resolve(socket);
    });
    socket.on("error", (err) => {
      reject(err);
    });
    return socket;
  });
}

function writeInt8(socket, int) {
  const buffer = Buffer.allocUnsafe(1);
  buffer.writeInt8(int, 0);
  socket.write(buffer);
}

function writeInt32(socket, int) {
  const buffer = Buffer.allocUnsafe(4);
  buffer.writeInt32BE(int, 0);
  socket.write(buffer);
}

function writeString(socket, str) {
  const len = Buffer.byteLength(str, "utf8");
  writeInt32(socket, len);
  socket.write(str, "utf8");
}
