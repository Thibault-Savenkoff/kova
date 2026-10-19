// @ts-check
// La ligne de commande lancée par chaque bouton. Sans dépendance à VSCode : testable avec node.

const path = require('path');

/** Met un texte entre apostrophes pour le shell : `l'été` → `'l'\''été'`. */
function citer(texte) {
  return `'${texte.replace(/'/g, `'\\''`)}'`;
}

/**
 * @param {'run' | 'build' | 'test'} action
 * @param {string} fichier chemin absolu du `.kova`
 * @param {string} kova chemin de l'exécutable `kova`
 */
function commande(action, fichier, kova) {
  const dossier = citer(path.dirname(fichier));
  const nom = path.basename(fichier);
  const k = citer(kova);
  switch (action) {
    case 'run':
      return `cd ${dossier} && ${k} run ${citer(nom)}`;
    case 'build': {
      // Compile, puis lance l'exécutable produit (même nom, sans `.kova`).
      const exe = nom.replace(/\.kova$/, '');
      return `cd ${dossier} && ${k} build ${citer(nom)} && ${citer('./' + exe)}`;
    }
    case 'test':
      return `cd ${dossier} && ${k} test ${citer(nom)}`;
  }
}

module.exports = { citer, commande };
