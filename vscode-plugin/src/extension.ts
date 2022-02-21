import * as cp from 'child_process';
import * as net from 'net';
import * as path from 'path';
import * as vscode from 'vscode';

export async function activate(context: vscode.ExtensionContext) {
  const socketPath = await getElmPairSocket(context);
  await connectToElmPair(socketPath);

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

function connectToElmPair(socketPath: string): Promise<void> {
  return new Promise((resolve, reject) => {
    const client = net.createConnection(socketPath);

    client.on('connect', () => { resolve(); });

    client.on('error', (err) => { reject(err); });

    client.on('data', () => {
      // TODO: apply refactors received from elm-pair.
      return;
    });

    client.on('end', () => {
      console.log("Elm-pair socket connection closed.");
      // TODO: handle this in some way.
    });
  });
}
