---
title: Dates and time
description: Date, DateTime, Weekday, the clock and pauses.
sidebar:
  order: 6
---

Kova has two types for the calendar, without any import:

- `Date`: a day, `2026-10-23`;
- `DateTime`: a day and a time to the second, `2026-10-23 14:05:09`, in local wall-clock time.

Both are small values, copied like numbers, compared with `=` and `<`, shown as above, and usable
as `Map` keys. They go from year 1 to year 9999, in the Gregorian calendar.

## Making a date

| Written | Gives |
| --- | --- |
| `Date.year(2026) month(10) day(23)` | that day |
| `DateTime.year(2026) month(10) day(23) hour(14) minute(5)` | that minute (`second(…)` may follow) |
| `Date.today` | today, on this computer's clock and time zone |
| `DateTime.now` | now, to the second |
| `"2026-10-23".to_date` | a `Date?`: `none` if the text is not a date that exists |
| `"2026-10-23 14:05:09".to_datetime` | a `DateTime?` |

A date that does not exist (February 30, month 13) is refused by the compiler when it is written
with numbers, and stops the program when it comes from variables. A text that may be wrong goes
through `to_date`, which gives `none` instead.

## Reading a date

| Method | Gives |
| --- | --- |
| `d.year`, `d.month`, `d.day -> Int` | its parts (`month` from 1 to 12). |
| `t.hour`, `t.minute`, `t.second -> Int` | on a `DateTime`. |
| `t.date -> Date` | the day of a `DateTime`. |
| `d.weekday -> Weekday` | `Monday`, `Tuesday`, … `Sunday`. |
| `d.to_string -> String` | `2026-10-23`, or `2026-10-23 14:05:09`. |

`Weekday` is an [enum](/guide/enums-and-match/), so a `match` on it must handle every day:

```kova
fn main {
    let d := Date.year(2026) month(10) day(23)
    print(d.weekday)
    match d.weekday {
        Saturday, Sunday => print("weekend")
        _ => print("work")
    }
}
```

```text
Friday
work
```

## Computing with dates

| Method | Gives |
| --- | --- |
| `d.plus_days(n: Int)` | `n` days later (earlier if `n` is negative). |
| `d.plus_months(n: Int)` | `n` months later; a day past the end of the month becomes its last day (January 31 + 1 month is February 28 or 29). |
| `t.plus_hours(n)`, `t.plus_minutes(n)`, `t.plus_seconds(n)` | on a `DateTime`. |
| `a.days_until(b: Date) -> Int` | the number of days from `a` to `b` (negative if `b` is before). |
| `a.seconds_until(b: DateTime) -> Int` | the number of seconds from `a` to `b`. |

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

A `DateTime` is a wall-clock time without a time zone: adding hours never jumps over a change to
or from summer time. `Date.today` and `DateTime.now` read the computer's zone (`TZ`, or the
system's setting).

## The clock and pauses

| Function | Gives |
| --- | --- |
| `now -> Float` | the seconds since 1970, with a fraction: for measuring how long something takes. |
| `sleep(seconds: Float)` | waits that long. |

```kova
fn main {
    let start: Float := now
    sleep(0.1)
    print(now - start >= 0.1)
}
```

```text
true
```
