const {Buffer} = require('buffer');
const cp = require('child_process');
const net = require('net');
const path = require('path');
const vscode = require('vscode');

const NEW_FILE_MSG = 0;
const FILE_CHANGED_MSG = 1;
const EDIT_METADATA = {
  label : "Change by Elm-pair",
  needsConfirmation : false,
};

let socket;
let deactivating = false;

async function activate(context) {
  const socketPath = await getElmPairSocket(context);
  socket = await connectToElmPair(socketPath);
  // Elm-pair expects a 4-byte editor-id. For Visual Studio Code it's 0.
  writeInt32(socket, 0);
  const elmFileIdsByPath = {};

  const processData = processRefactors();
  processData.next(); // Run to first `yield` (moment we need data).
  socket.on('data', (data) => { processData.next(data); });

  socket.on('end', () => {
    if (!deactivating) {
      throw new Error("Connection to elm-pair daemon closed unexpectedly.");
    }
  });

  vscode.workspace.onDidChangeTextDocument(changeEvent => {
    const doc = changeEvent.document;
    if (doc.languageId !== "elm") {
      return;
    }
    const fileName = doc.fileName;
    let fileId = elmFileIdsByPath[fileName];
    if (typeof fileId === "undefined") {
      fileId = elmFileIdsByPath[doc.fileName] =
          Object.keys(elmFileIdsByPath).length;
      writeInt32(socket, fileId);
      writeInt8(socket, NEW_FILE_MSG);
      writeString(socket, fileName);
      writeString(socket, doc.getText());
    } else {
      for (const change of changeEvent.contentChanges) {
        const range = change.range;
        writeInt32(socket, fileId);
        writeInt8(socket, FILE_CHANGED_MSG);
        writeInt32(socket, range.start.line);
        writeInt32(socket, range.start.character);
        writeInt32(socket, range.end.line);
        writeInt32(socket, range.end.character);
        writeString(socket, change.text);
      }
    }
  });
}

function deactivate() {
  deactivating = true;
  socket.end();
}

function getElmPairSocket(context) {
  return new Promise((resolve, reject) => {
    const elmPairBin = path.join(context.extensionPath, "elm-pair");
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
async function* processRefactors() {
  const edit = new vscode.WorkspaceEdit();
  let buffer = yield;
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

  await vscode.workspace.applyEdit(edit);
  yield* processRefactors();
}

function* readInt32(buffer) {
  const [sample, newBuffer] = yield* takeFromBuffer(buffer, 4);
  const num = sample.readInt32BE();
  return [ num, newBuffer ];
}

function* readString(buffer) {
  const [stringLength, buffer2] = yield* readInt32(buffer);
  const [sample, buffer3] = yield* takeFromBuffer(buffer2, stringLength);
  const string = sample.toString('utf8');
  return [ string, buffer3 ];
}

// Read a specific number of bytes from a buffer, waiting for additional data if
// necessary.
function* takeFromBuffer(buffer, bytes) {
  while (buffer.length < bytes) {
    buffer = Buffer.concat([ buffer, yield ]);
  }
  const sample = buffer.subarray(0, bytes);
  const rest = buffer.subarray(bytes);
  return [ sample, rest ];
}

function connectToElmPair(socketPath) {
  return new Promise((resolve, reject) => {
    const socket = net.createConnection(socketPath);
    socket.on('connect', () => { resolve(socket); });
    socket.on('error', (err) => { reject(err); });
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
  const len = Buffer.byteLength(str, 'utf8');
  writeInt32(socket, len);
  socket.write(str, 'utf8');
}

module.exports = {
  activate,
  deactivate
};
