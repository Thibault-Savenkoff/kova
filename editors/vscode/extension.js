// @ts-check
// Extension Kova :
// - E2 : boutons ▶ Run / 🔨 Build / 🧪 Test. Chaque bouton enregistre le fichier,
//   puis lance `kova` dans le terminal « Kova ».
// - E3 : démarre `kova lsp` (erreurs pendant la frappe, complétion, aide sur les paramètres).

const fs = require('fs');
const vscode = require('vscode');
const { LanguageClient } = require('vscode-languageclient/node');
const { commande } = require('./commandes');

// VSCodium en flatpak : ce qu'il lance tourne dans un bac à sable qui ne voit ni `kova`
// ni ton PATH. On passe donc par flatpak-spawn pour lancer sur le système.
const enFlatpak = fs.existsSync('/.flatpak-info');

/** @type {LanguageClient | undefined} */
let client;

/** @param {vscode.ExtensionContext} context */
function activate(context) {
  for (const action of /** @type {const} */ (['run', 'build', 'test'])) {
    context.subscriptions.push(
      vscode.commands.registerCommand(`kova.${action}`, (/** @type {vscode.Uri=} */ uri) => lancer(action, uri))
    );
  }

  const kova = cheminKova();
  const serveur = enFlatpak
    ? { command: '/usr/bin/flatpak-spawn', args: ['--host', '--watch-bus', kova, 'lsp'] }
    : { command: kova, args: ['lsp'] };
  client = new LanguageClient('kova', 'Kova', serveur, {
    documentSelector: [{ language: 'kova' }],
  });
  client.start();
}

function deactivate() {
  return client?.stop();
}

/** @returns {string} */
function cheminKova() {
  return vscode.workspace.getConfiguration('kova').get('chemin', 'kova');
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

  const t = terminal();
  t.show(true);
  t.sendText(commande(action, fichier, cheminKova()));
}

/** Le terminal « Kova », réutilisé d'un clic à l'autre. */
function terminal() {
  const existant = vscode.window.terminals.find((t) => t.name === 'Kova' && t.exitStatus === undefined);
  if (existant) return existant;
  // En flatpak : ton shell habituel, sur le système.
  if (enFlatpak) {
    return vscode.window.createTerminal({
      name: 'Kova',
      shellPath: '/usr/bin/flatpak-spawn',
      shellArgs: ['--host', '--watch-bus', '--env=TERM=xterm-256color', 'sh', '-c', 'exec "${SHELL:-sh}" -l'],
    });
  }
  return vscode.window.createTerminal({ name: 'Kova' });
}

module.exports = { activate, deactivate };
