---
title: Lancer ou compiler
description: Ce que font kova run et kova build, comment une fonction peut différer entre les deux, et le header d'un programme.
sidebar:
  order: 14
---

## Deux façons, un résultat

Chaque programme peut démarrer de deux façons :

- `kova run app.kova` vérifie le programme et le lance tout de suite, dans l'interpréteur de Kova.
  Rien n'est écrit sur le disque ; il démarre en quelques millisecondes. C'est le mode pour écrire
  et essayer.
- `kova build app.kova` vérifie le programme et le compile en code natif, un exécutable qui tourne
  sans Kova. C'est le mode pour la vitesse et pour donner le programme à d'autres.

Les deux affichent la même chose, chiffre pour chiffre, sur chaque système. Les tests de Kova
lancent chaque programme d'exemple des deux façons et comparent les sorties.

Les programmes qui appellent des [bibliothèques C](/fr/guide/calling-c/) ou ouvrent des
[fenêtres](/fr/guide/windows/) tournent en code natif même sous `kova run` : Kova les compile
d'abord en mémoire, sans qu'on le demande.

## Une fonction qui diffère : `mode`

Parfois, une fonction ne devrait faire son travail que dans un mode : une bannière que seul le
programme compilé montre, une vérification lente qui ne vaut la peine qu'en écrivant, un appel à
quelque chose que l'interpréteur ne sait pas faire. `mode build` (ou `mode run`) sous la signature
le dit, et dit ce que fait un appel **dans l'autre mode** :

```kova
fn banner
    mode build else skip
{
    print("=== built ===")
}

fn workers_count -> Int
    mode build else return 1
{
    return 8
}

fn main {
    banner
    print(workers_count)
}
```

```text
1
```

Compilé, le même programme affiche `=== built ===` et `8`. Les choix après `else` :

| Écrit | Dans l'autre mode, un appel |
| --- | --- |
| `else skip` | ne fait rien (seulement pour une fonction qui ne rend rien) |
| `else return valeur` | donne cette valeur (`else return none` pour un `T?`) |
| `else "message"` | arrête le programme avec ce message, comme un `require` non respecté |
| pas de `else` | exécute aussi la fonction, avec les mêmes résultats |

Le dernier est pour la vitesse : sous `kova run`, une fonction marquée `mode build` sans `else`
fait compiler tout le programme en code natif par Kova avant de le lancer, ce qui prend quelques
millisecondes.

Dans l'autre mode, les contrats de la fonction sont aussi laissés de côté, tandis que ses
arguments sont quand même calculés (un argument `move` est donné quand même). `kova test` compte
comme `kova run`.

Pourquoi au niveau de la fonction et pas un `if` dans le code ? Parce que c'est alors visible à un
seul endroit, en haut de la fonction, où chaque lecteur d'un appel peut le trouver ; et le
compilateur peut vérifier que les deux modes donnent une valeur du bon type.

## Le header

Un programme peut commencer par un `header`, ses réglages :

```kova
header {
    mode := build
    lock := true
    lang := fr
}

fn main {
    print("Bonjour")
}
```

| Réglage | Valeurs | Dit |
| --- | --- | --- |
| `mode` | `run` (par défaut), `build` | ce que fait `kova app.kova`, sans commande |
| `lock` | `false` (par défaut), `true` | `true` : l'autre commande est refusée, pour un programme fait pour être seulement compilé (ou seulement lancé) |
| `lang` | `auto` (par défaut), `en`, `fr` | la langue des messages de Kova pour ce programme |

Avec `mode := build`, `kova app.kova` compile le programme, et avec `lock := true`,
`kova run app.kova` répond :

```console
erreur: ce programme est réservé à `kova build` (header ligne 3)
```

## L'exécutable

`kova build` écrit l'exécutable à côté du programme : `app` sous Linux et macOS, `app.exe` sous
Windows (`-o nom` en choisit un autre). Il n'a besoin ni de Kova, ni d'environnement d'exécution,
ni de bibliothèque pour tourner, sauf GTK 4 sous Linux pour les fenêtres et OpenSSL sous Linux pour
`https://`, tous deux cherchés au lancement du programme.

- `--target windows`, `linux`, `mac` (et leurs variantes `-arm`) compilent pour un autre système,
  depuis n'importe quel système ([la commande](/fr/tools/command/#les-cibles)).
- `--boost` retire les [contrats](/fr/guide/contracts-and-tests/).
- Un programme compilé choisit la langue de ses messages à son lancement, comme Kova lui-même.
