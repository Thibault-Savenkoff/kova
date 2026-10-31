---
title: Dates et heures
description: Date, DateTime, Weekday, l'horloge et les pauses.
sidebar:
  order: 6
---

Kova a deux types pour le calendrier, sans aucun import :

- `Date` : un jour, `2026-10-23` ;
- `DateTime` : un jour et une heure à la seconde près, `2026-10-23 14:05:09`, en heure locale.

Les deux sont de petites valeurs, copiées comme des nombres, comparées avec `=` et `<`, affichées
comme ci-dessus, et utilisables comme clés de `Map`. Elles vont de l'an 1 à l'an 9999, dans le
calendrier grégorien.

## Faire une date

| Écrit | Donne |
| --- | --- |
| `Date.year(2026) month(10) day(23)` | ce jour-là |
| `DateTime.year(2026) month(10) day(23) hour(14) minute(5)` | cette minute-là (`second(…)` peut suivre) |
| `Date.today` | aujourd'hui, selon l'horloge et le fuseau horaire de cet ordinateur |
| `DateTime.now` | maintenant, à la seconde près |
| `"2026-10-23".to_date` | un `Date?` : `none` si le texte n'est pas une date qui existe |
| `"2026-10-23 14:05:09".to_datetime` | un `DateTime?` |

Une date qui n'existe pas (le 30 février, le mois 13) est refusée par le compilateur quand elle
est écrite avec des nombres, et arrête le programme quand elle vient de variables. Un texte qui
peut être faux passe par `to_date`, qui donne `none` à la place.

## Lire une date

| Méthode | Donne |
| --- | --- |
| `d.year`, `d.month`, `d.day -> Int` | ses parties (`month` de 1 à 12). |
| `t.hour`, `t.minute`, `t.second -> Int` | sur un `DateTime`. |
| `t.date -> Date` | le jour d'un `DateTime`. |
| `d.weekday -> Weekday` | `Monday`, `Tuesday`, … `Sunday`. |
| `d.to_string -> String` | `2026-10-23`, ou `2026-10-23 14:05:09`. |

`Weekday` est une [enum](/fr/guide/enums-and-match/), donc un `match` dessus doit traiter chaque
jour :

```kova
fn main {
    let d := Date.year(2026) month(10) day(23)
    print(d.weekday)
    match d.weekday {
        .Saturday, .Sunday => print("weekend")
        _ => print("work")
    }
}
```

```text
Friday
work
```

## Calculer avec des dates

| Méthode | Donne |
| --- | --- |
| `d.plus_days(n: Int)` | `n` jours plus tard (plus tôt si `n` est négatif). |
| `d.plus_months(n: Int)` | `n` mois plus tard ; un jour après la fin du mois devient son dernier jour (31 janvier + 1 mois donne le 28 ou le 29 février). |
| `t.plus_hours(n)`, `t.plus_minutes(n)`, `t.plus_seconds(n)` | sur un `DateTime`. |
| `a.days_until(b: Date) -> Int` | le nombre de jours de `a` à `b` (négatif si `b` est avant). |
| `a.seconds_until(b: DateTime) -> Int` | le nombre de secondes de `a` à `b`. |

```kova
fn main {
    let start := Date.year(2024) month(1) day(31)
    print(start.plus_months(1))
    print(start.plus_days(30))
    let christmas := Date.year(2026) month(12) day(25)
    print(Date.year(2026) month(10) day(23).days_until(christmas))
    let t := DateTime.year(2026) month(10) day(23) hour(23) minute(30)
    print(t.plus_minutes(90))
}
```

```text
2024-02-29
2024-03-01
63
2026-10-24 01:00:00
```

Un `DateTime` est une heure locale sans fuseau horaire : ajouter des heures ne saute jamais un
passage à l'heure d'été ou d'hiver. `Date.today` et `DateTime.now` lisent le fuseau de
l'ordinateur (`TZ`, ou le réglage du système).

## L'horloge et les pauses

| Fonction | Donne |
| --- | --- |
| `now -> Float` | les secondes depuis 1970, avec une fraction : pour mesurer combien de temps prend quelque chose. |
| `sleep(seconds: Float)` | attend ce temps-là. |

```kova
fn main {
    let start: Float := now
    sleep(0.1)
    print(now - start >= 0.05)
}
```

```text
true
```
