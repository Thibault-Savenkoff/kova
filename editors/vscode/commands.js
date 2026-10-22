// @ts-check
// The command line run by each button. No dependency on VSCode: testable with node.

const path = require('path');

/** Quotes a text for the shell: `l'été` → `'l'\''été'`. */
function quote(text) {
  return `'${text.replace(/'/g, `'\\''`)}'`;
}

/**
 * @param {'run' | 'build' | 'test'} action
 * @param {string} file absolute path of the `.kova`
 * @param {string} kova path of the `kova` executable
 */
function command(action, file, kova) {
  const dir = quote(path.dirname(file));
  const name = path.basename(file);
  const k = quote(kova);
  switch (action) {
    case 'run':
      return `cd ${dir} && ${k} run ${quote(name)}`;
    case 'build': {
      // Compiles, then runs the produced executable (same name, without `.kova`).
      const exe = name.replace(/\.kova$/, '');
      return `cd ${dir} && ${k} build ${quote(name)} && ${quote('./' + exe)}`;
    }
    case 'test':
      return `cd ${dir} && ${k} test ${quote(name)}`;
  }
}

/**
 * What `kova test` printed → each test's result: `✅ name`, or `❌ name` then its message,
 * indented by 3 spaces.
 * @param {string} out
 * @returns {Map<string, { ok: boolean, message: string }>}
 */
function results(out) {
  const found = new Map();
  /** @type {{ ok: boolean, message: string } | undefined} */
  let failed;
  for (const line of out.split('\n')) {
    if (line.startsWith('✅ ') || line.startsWith('❌ ')) {
      const result = { ok: line.startsWith('✅'), message: '' };
      found.set(line.slice(2), result);
      failed = result.ok ? undefined : result;
    } else if (failed && line.startsWith('   ')) {
      failed.message += (failed.message ? '\n' : '') + line.slice(3);
    } else {
      failed = undefined;
    }
  }
  return found;
}

module.exports = { quote, command, results };
