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
 * @param {string=} test the one test to run (the ▶ above a `test`)
 */
function command(action, file, kova, test) {
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
      return `cd ${dir} && ${k} test ${quote(name)}` + (test === undefined ? '' : ` ${quote(test)}`);
  }
}

module.exports = { quote, command };
