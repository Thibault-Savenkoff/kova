// Checks the command lines of the buttons: `node tests/commands.test.js`.
const assert = require('assert');
const { quote, command, results } = require('../commands');

assert.strictEqual(quote("l'été"), `'l'\\''été'`);
assert.strictEqual(
  command('run', '/home/me/my projects/fib.kova', 'kova'),
  `cd '/home/me/my projects' && 'kova' run 'fib.kova'`
);
assert.strictEqual(
  command('build', '/p/fib.kova', '/opt/kova'),
  `cd '/p' && '/opt/kova' build 'fib.kova' && './fib'`
);
assert.strictEqual(command('test', '/p/a.kova', 'kova'), `cd '/p' && 'kova' test 'a.kova'`);
const found = results('✅ un\n❌ deux\n   error: non\n     check x = 1\n\n1 passed, 1 failed out of 2\n');
assert.deepStrictEqual([...found], [
  ['un', { ok: true, message: '' }],
  ['deux', { ok: false, message: 'error: non\n  check x = 1' }],
]);
console.log('commands: ok');

// VSCodium reads a `wordPattern` string without the `u` flag, where `\p{L}` is no letter:
// no word under the cursor, so the suggestion list never opened while typing.
const words = require('../language-configuration.json').wordPattern;
const word = new RegExp(words.pattern, words.flags);
assert.deepStrictEqual('    sc'.match(new RegExp(word, 'gu')), ['sc']);
assert.deepStrictEqual('été_2 := -1.5e3'.match(new RegExp(word, 'gu')), ['été_2', '-1.5e3']);
