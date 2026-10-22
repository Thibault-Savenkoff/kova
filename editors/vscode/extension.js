// @ts-check
// Kova extension:
// - E2: ▶ Run / 🔨 Build / 🧪 Test buttons. Each button saves the file,
//   then runs `kova` in the "Kova" terminal.
// - E3 to E5: starts `kova lsp` (errors while typing, completion, hover, renaming…).
// - E5: a ▶ in the margin before each `test` (and in the Testing panel), which becomes ✅/❌.

const fs = require('fs');
const path = require('path');
const { execFile } = require('child_process');
const vscode = require('vscode');
const { LanguageClient } = require('vscode-languageclient/node');
const { command, results } = require('./commands');

// VSCodium as a flatpak: what it starts runs in a sandbox that sees neither `kova`
// nor your PATH. So flatpak-spawn is used to run on the host system.
const inFlatpak = fs.existsSync('/.flatpak-info');

// Kova's messages follow the editor's language (`fr`, `en-us`…), see docs/PLAN.md 2.13.
const lang = `KOVA_LANG=${vscode.env.language}`;

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
    ? { command: '/usr/bin/flatpak-spawn', args: ['--host', '--watch-bus', `--env=${lang}`, kova, 'lsp'] }
    : { command: kova, args: ['lsp'], options: { env: { ...process.env, KOVA_LANG: vscode.env.language } } };
  client = new LanguageClient('kova', 'Kova', server, {
    documentSelector: [{ language: 'kova' }],
  });
  client.start().then(() => tests(context));
}

/**
 * The tests of the open `.kova` files, as `kova lsp` reads them, in VSCode's Testing panel:
 * a ▶ in the margin before each `test`, which runs `kova test file "name"`.
 * @param {vscode.ExtensionContext} context
 */
function tests(context) {
  const controller = vscode.tests.createTestController('kova', 'Kova');
  context.subscriptions.push(controller);

  /** @type {Map<string, NodeJS.Timeout>} */
  const waiting = new Map();
  /** @param {vscode.TextDocument} doc */
  const refresh = async (doc) => {
    if (doc.languageId !== 'kova' || doc.uri.scheme !== 'file' || !client) return;
    const id = doc.uri.toString();
    /** @type {{ name: string, range: any }[] | null} */
    const found = await client.sendRequest('kova/tests', { textDocument: { uri: id } });
    if (!found?.length) {
      controller.items.delete(id);
      return;
    }
    const file = controller.items.get(id) ?? controller.createTestItem(id, path.basename(doc.uri.fsPath), doc.uri);
    file.children.replace(
      found.map((t) => {
        const item = controller.createTestItem(`${id}\n${t.name}`, t.name, doc.uri);
        item.range = client?.protocol2CodeConverter.asRange(t.range);
        return item;
      })
    );
    controller.items.add(file);
  };
  /** While typing: once it stops for half a second. @param {vscode.TextDocument} doc */
  const later = (doc) => {
    clearTimeout(waiting.get(doc.uri.toString()));
    waiting.set(doc.uri.toString(), setTimeout(() => refresh(doc), 500));
  };
  vscode.workspace.textDocuments.forEach(refresh);
  context.subscriptions.push(
    vscode.workspace.onDidOpenTextDocument(refresh),
    vscode.workspace.onDidChangeTextDocument((e) => later(e.document)),
    vscode.workspace.onDidCloseTextDocument((doc) => controller.items.delete(doc.uri.toString()))
  );

  controller.createRunProfile(
    'Run',
    vscode.TestRunProfileKind.Run,
    async (request) => {
      const run = controller.createTestRun(request);
      /** @type {vscode.TestItem[]} */
      const asked = [];
      if (request.include) asked.push(...request.include);
      else controller.items.forEach((file) => asked.push(file));
      for (const item of asked) {
        // A file: all its tests at once; a test: only that one.
        const file = item.parent ?? item;
        /** @type {vscode.TestItem[]} */
        const chosen = [];
        if (item.parent) chosen.push(item);
        else item.children.forEach((t) => chosen.push(t));
        if (!file.uri) continue;
        const doc = vscode.workspace.textDocuments.find((d) => d.uri.toString() === file.uri?.toString());
        if (doc?.isDirty) await doc.save(); // `kova` reads the file from disk
        chosen.forEach((t) => run.started(t));
        const out = await kovaTest(file.uri.fsPath, item.parent ? item.label : undefined);
        run.appendOutput(out.text.replace(/\r?\n/g, '\r\n'));
        const found = results(out.text);
        for (const t of chosen) {
          const result = found.get(t.label);
          if (result?.ok) {
            run.passed(t);
          } else if (result) {
            const message = new vscode.TestMessage(result.message);
            if (t.uri && t.range) message.location = new vscode.Location(t.uri, t.range);
            run.failed(t, message);
          } else {
            // `exit` in a test, or the file doesn't compile: the whole output says why.
            run.errored(t, new vscode.TestMessage(out.text));
          }
        }
      }
      run.end();
    },
    true
  );
}

/**
 * Runs `kova test` on `file` (only the test `name` if given) and gives what it printed.
 * @param {string} file
 * @param {string=} name
 * @returns {Promise<{ text: string }>}
 */
function kovaTest(file, name) {
  const args = ['test', path.basename(file), ...(name === undefined ? [] : [name])];
  const dir = path.dirname(file);
  const [cmd, all, options] = inFlatpak
    ? ['/usr/bin/flatpak-spawn', ['--host', `--directory=${dir}`, `--env=${lang}`, kovaPath(), ...args], {}]
    : [kovaPath(), args, { cwd: dir, env: { ...process.env, KOVA_LANG: vscode.env.language } }];
  return new Promise((resolve) => {
    execFile(cmd, all, options, (_error, stdout, stderr) => resolve({ text: `${stdout}${stderr}` }));
  });
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
      shellArgs: ['--host', '--watch-bus', '--env=TERM=xterm-256color', `--env=${lang}`, 'sh', '-c', 'exec "${SHELL:-sh}" -l'],
    });
  }
  return vscode.window.createTerminal({ name: 'Kova', env: { KOVA_LANG: vscode.env.language } });
}

module.exports = { activate, deactivate };
