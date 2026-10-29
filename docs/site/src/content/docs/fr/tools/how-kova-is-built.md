---
title: Comment Kova est construit
description: Le compilateur, le runtime et les tests, pour qui veut les lire ou les changer.
sidebar:
  order: 4
---

Pour qui veut lire ou changer le compilateur. Tout est dans les dossiers `src/` et `runtime/` du
dépôt, écrit en Rust.

## La chaîne

```text
texte ─ lexer.rs ─ parser.rs ─ types.rs ─ ownership.rs ─ kir.rs ─┬─ interp.rs   (kova run, kova test)
                                                                 └─ codegen.rs  (kova build) ─ link.rs
```

1. **`lexer.rs`** découpe le texte en jetons, chacun avec sa position.
2. **`parser.rs`** construit l'arbre (`ast.rs`). Il continue après une erreur, pour que l'éditeur
   puisse en montrer plusieurs à la fois.
3. **`types.rs`** est le vérificateur : il donne un type à chaque expression et note ce dont les
   passes suivantes ont besoin (`Info`). Les types sont surtout écrits par le programmeur ; presque
   rien n'est deviné.
4. **`ownership.rs`** est le seul juge des déplacements et des emprunts
   ([La possession](/fr/guide/ownership/) : pas de durées de vie, des marques `var` / `move` à
   chaque appel).
5. **`kir.rs`** transforme l'arbre vérifié en KIR, un petit jeu d'instructions typé partagé par les
   deux backends. Il fait aussi les copies des fonctions génériques et construit les fonctions
   d'aide (libérer, copier, afficher, comparer) de chaque type.
6. **`interp.rs`** exécute le KIR directement. **`codegen.rs`** le transforme en code machine avec
   Cranelift pour x86-64 et arm64.
7. **`link.rs`** est l'éditeur de liens de Kova : il écrit des exécutables ELF (Linux), PE
   (Windows) et Mach-O (macOS) à partir de l'objet du programme et du runtime. Aucun compilateur C
   ni éditeur de liens du système n'est nécessaire.

`kova run` et `kova build` doivent afficher exactement la même chose : les tests lancent chaque
programme des deux façons.

## Le runtime

`runtime/kova.rs` est une bibliothèque Rust `no_std` (textes, listes, fichiers, web, expressions
régulières, JSON, cryptographie…), avec un fichier par système pour ce qu'il lui demande :
`linux.rs` (appels système directs, son propre allocateur de mémoire), `windows.rs`
(`kernel32.dll` et quelques autres), `macos.rs` (`libSystem`). Le code dont l'interpréteur et le
runtime ont tous deux besoin (mise en forme des nombres, dates, HTTP, `src/shared.rs`,
`src/json.rs`, `src/regex.rs`, `src/crypto.rs`…) est écrit une fois et inclus dans les deux, pour
qu'ils ne puissent pas différer.

## Le côté éditeur

`lsp.rs` (serveur de langage) et `debug.rs` (adaptateur de débogage) appellent la même fonction
`analyze` que la ligne de commande, sur du texte pas encore enregistré.

## Messages et langues

Chaque texte que montre Kova est une clé dans `locales/en.kova` et `locales/fr.kova` ; le test
unitaire `complete_catalogs` vérifie que les deux ont les mêmes clés et les mêmes trous. Les textes
du runtime sont écrits dans chaque exécutable, qui choisit sa langue au démarrage.

## Les tests

- `cargo test` lance les tests unitaires et `tests/kova.rs` : chaque `tests/ok/*.kova` (avec des
  lignes `// expect:`, lancé **et** compilé), chaque `tests/error/*.kova` (avec des lignes
  `// error:`), chaque exemple de ce site, en anglais et en français, le serveur de langage et le
  débogueur.
- Un programme de plusieurs fichiers est un dossier `tests/ok/<nom>/main.kova`. `// stdin:` et
  `// args:` nourrissent le programme.
- Le code derrière `cfg(target_os = …)` se vérifie depuis n'importe quelle machine avec
  `cargo check --tests --target aarch64-apple-darwin` et `--target x86_64-pc-windows-msvc`.
- `.github/workflows/tests.yml` lance tout sous Linux, Windows et macOS (x86-64 et arm64) quand on
  le démarre à la main.

## Ajouter quelque chose

- Un message : une clé dans `locales/en.kova` et dans `locales/fr.kova`.
- Une méthode intégrée : sa signature dans `types.rs`, sa traduction dans `kir.rs`, un test dans
  `tests/ok`.
- Une fonction du runtime : l'écrire une fois dans `src/shared.rs` quand les deux backends en ont
  besoin.
