// Checks the command lines of the buttons: `node tests/commands.test.js`.
const assert = require('assert');
const { quote, command } = require('../commands');

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
assert.strictEqual(command('test', '/p/a.kova', 'kova', "l'un"), `cd '/p' && 'kova' test 'a.kova' 'l'\\''un'`);
console.log('commands: ok');
