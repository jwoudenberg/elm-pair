import * as vscode from 'vscode';

export function activate(context: vscode.ExtensionContext) {
  vscode.workspace.onDidChangeTextDocument(changeEvent => { 
    if (changeEvent.document.languageId === "elm") {
      debugger; 
    }
  });
}

export function deactivate(): void {}