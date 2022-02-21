import {Buffer} from 'buffer';
import * as cp from 'child_process';
import * as net from 'net';
import * as path from 'path';
import * as vscode from 'vscode';

export async function activate(context: vscode.ExtensionContext) {
  const socketPath = await getElmPairSocket(context);
  const socket = await connectToElmPair(socketPath);
  writeEditorIdentifier(socket);

  socket.on('data', () => {
    // TODO: apply refactors received from elm-pair.
    return;
  });

  socket.on('end', () => {
    console.log("Elm-pair socket connection closed.");
    // TODO: handle this in some way.
  });

  vscode.workspace.onDidChangeTextDocument(changeEvent => {
    if (changeEvent.document.languageId === "elm") {
      debugger;
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

function writeEditorIdentifier(socket: net.Socket): void {
  const buffer = Buffer.allocUnsafe(4); // Elm-pair expects a 4-byte editor id.
  buffer.writeInt32BE(0, 0); // The vscode editor id to elm-pair is 0.
  socket.write(buffer);
}
