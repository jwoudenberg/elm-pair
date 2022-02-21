import {Buffer} from 'buffer';
import * as cp from 'child_process';
import * as net from 'net';
import * as path from 'path';
import * as vscode from 'vscode';

const NEW_FILE_MSG = 0;
const FILE_CHANGED_MSG = 1;

type FileIdMap = {
  [key: string]: number;
};

export async function activate(context: vscode.ExtensionContext) {
  const socketPath = await getElmPairSocket(context);
  const socket = await connectToElmPair(socketPath);
  // Elm-pair expects a 4-byte editor-id. For Visual Studio Code it's 0.
  writeInt32(socket, 0);
  const elmFileIdsByPath: FileIdMap = {};

  socket.on('data', () => {
    // TODO: apply refactors received from elm-pair.
    return;
  });

  socket.on('end', () => {
    console.log("Elm-pair socket connection closed.");
    // TODO: handle this in some way.
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
      writeInt8(socket, NEW_FILE_MSG);
      writeInt32(socket, fileId);
      writeString(socket, fileName);
      writeString(socket, doc.getText());
    }
    for (const change of changeEvent.contentChanges) {
      const range = change.range;
      writeInt8(socket, FILE_CHANGED_MSG);
      writeInt32(socket, fileId);
      writeInt32(socket, range.start.line);
      writeInt32(socket, range.start.character);
      writeInt32(socket, range.end.line);
      writeInt32(socket, range.end.character);
      writeString(socket, change.text);
    }
  });
}

export function deactivate(): void {}

function getElmPairSocket(context: vscode.ExtensionContext): Promise<string> {
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

function connectToElmPair(socketPath: string): Promise<net.Socket> {
  return new Promise((resolve, reject) => {
    const socket = net.createConnection(socketPath);
    socket.on('connect', () => { resolve(socket); });
    socket.on('error', (err) => { reject(err); });
    return socket;
  });
}

function writeInt8(socket: net.Socket, int: number): void {
  const buffer = Buffer.allocUnsafe(1);
  buffer.writeInt8(int, 0);
  socket.write(buffer);
}

function writeInt32(socket: net.Socket, int: number): void {
  const buffer = Buffer.allocUnsafe(4);
  buffer.writeInt32BE(int, 0);
  socket.write(buffer);
}

function writeString(socket: net.Socket, str: string): void {
  const len = Buffer.byteLength(str, 'utf8');
  writeInt32(socket, len);
  socket.write(str, 'utf8');
}
