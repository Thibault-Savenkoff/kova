// Vérifie les lignes de commande des boutons : `node tests/commandes.test.js`.
const assert = require('assert');
const { citer, commande } = require('../commandes');

assert.strictEqual(citer("l'été"), `'l'\\''été'`);
assert.strictEqual(
  commande('run', '/home/moi/mes projets/fib.kova', 'kova'),
  `cd '/home/moi/mes projets' && 'kova' run 'fib.kova'`
);
assert.strictEqual(
  commande('build', '/p/fib.kova', '/opt/kova'),
  `cd '/p' && '/opt/kova' build 'fib.kova' && './fib'`
);
assert.strictEqual(commande('test', '/p/a.kova', 'kova'), `cd '/p' && 'kova' test 'a.kova'`);
console.log('commandes : ok');
