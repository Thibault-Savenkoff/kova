// @ts-check
// Kova extension:
// - E2: ▶ Run / 🔨 Build / 🧪 Test buttons. Each button saves the file,
//   then runs `kova` in the "Kova" terminal.
// - E3: starts `kova lsp` (errors while typing, completion, parameter help).

const fs = require('fs');
const vscode = require('vscode');
const { LanguageClient } = require('vscode-languageclient/node');
const { command } = require('./commands');

// VSCodium as a flatpak: what it starts runs in a sandbox that sees neither `kova`
// nor your PATH. So flatpak-spawn is used to run on the host system.
const inFlatpak = fs.existsSync('/.flatpak-info');

/** @type {LanguageClient | undefined} */
let client;

/** @param {vscode.ExtensionContext} context */
function activate(context) {
  for (const action of /** @type {const} */ (['run', 'build', 'test'])) {
    context.subscriptions.push(
      vscode.commands.registerCommand(`kova.${action}`, (/** @type {vscode.Uri=} */ uri) => launch(action, uri))
    );
  }

  const kova = kovaPath();
  const server = inFlatpak
    ? { command: '/usr/bin/flatpak-spawn', args: ['--host', '--watch-bus', kova, 'lsp'] }
    : { command: kova, args: ['lsp'] };
  client = new LanguageClient('kova', 'Kova', server, {
    documentSelector: [{ language: 'kova' }],
  });
  client.start();
}

function deactivate() {
  return client?.stop();
}

/** @returns {string} */
function kovaPath() {
  return vscode.workspace.getConfiguration('kova').get('path', 'kova');
}

/**
 * @param {'run' | 'build' | 'test'} action
 * @param {vscode.Uri=} uri file of the clicked button (otherwise: the active editor)
 */
async function launch(action, uri) {
  const doc = uri
    ? vscode.workspace.textDocuments.find((d) => d.uri.toString() === uri.toString())
    : vscode.window.activeTextEditor?.document;
  const file = uri?.fsPath ?? doc?.uri.fsPath;
  if (!file) return;
  if (doc?.isDirty) await doc.save(); // `kova` reads the file from disk

  const t = terminal();
  t.show(true);
  t.sendText(command(action, file, kovaPath()));
}

/** The "Kova" terminal, reused from one click to the next. */
function terminal() {
  const existing = vscode.window.terminals.find((t) => t.name === 'Kova' && t.exitStatus === undefined);
  if (existing) return existing;
  // In flatpak: your usual shell, on the host system.
  if (inFlatpak) {
    return vscode.window.createTerminal({
      name: 'Kova',
      shellPath: '/usr/bin/flatpak-spawn',
      shellArgs: ['--host', '--watch-bus', '--env=TERM=xterm-256color', 'sh', '-c', 'exec "${SHELL:-sh}" -l'],
    });
  }
  return vscode.window.createTerminal({ name: 'Kova' });
}

module.exports = { activate, deactivate };
