// @ts-check
// Extension Kova (E2) : boutons ▶ Run / 🔨 Build / 🧪 Test en haut de l'éditeur.
// Chaque bouton enregistre le fichier, puis lance `kova` dans le terminal « Kova ».

const fs = require('fs');
const vscode = require('vscode');
const { commande } = require('./commandes');

/** @param {vscode.ExtensionContext} context */
function activate(context) {
  for (const action of /** @type {const} */ (['run', 'build', 'test'])) {
    context.subscriptions.push(
      vscode.commands.registerCommand(`kova.${action}`, (/** @type {vscode.Uri=} */ uri) => lancer(action, uri))
    );
  }
}

/**
 * @param {'run' | 'build' | 'test'} action
 * @param {vscode.Uri=} uri fichier du bouton cliqué (sinon : l'éditeur actif)
 */
async function lancer(action, uri) {
  const doc = uri
    ? vscode.workspace.textDocuments.find((d) => d.uri.toString() === uri.toString())
    : vscode.window.activeTextEditor?.document;
  const fichier = uri?.fsPath ?? doc?.uri.fsPath;
  if (!fichier) return;
  if (doc?.isDirty) await doc.save(); // `kova` lit le fichier sur le disque

  const kova = vscode.workspace.getConfiguration('kova').get('chemin', 'kova');
  const t = terminal();
  t.show(true);
  t.sendText(commande(action, fichier, kova));
}

/** Le terminal « Kova », réutilisé d'un clic à l'autre. */
function terminal() {
  const existant = vscode.window.terminals.find((t) => t.name === 'Kova' && t.exitStatus === undefined);
  if (existant) return existant;
  // VSCodium en flatpak : le terminal normal est dans le bac à sable et ne voit ni `kova`
  // ni ton PATH. On ouvre donc ton shell habituel sur le système, via flatpak-spawn.
  if (fs.existsSync('/.flatpak-info')) {
    return vscode.window.createTerminal({
      name: 'Kova',
      shellPath: '/usr/bin/flatpak-spawn',
      shellArgs: ['--host', '--watch-bus', '--env=TERM=xterm-256color', 'sh', '-c', 'exec "${SHELL:-sh}" -l'],
    });
  }
  return vscode.window.createTerminal({ name: 'Kova' });
}

module.exports = { activate };
