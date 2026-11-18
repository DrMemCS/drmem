# Logic Node Expression Reference

This chapter is the reference for the expression language used by logic nodes. For the configuration file layout (`inputs`, `outputs`, `defs`, `exprs`) see the Logic Nodes chapter; this chapter describes what you can write inside those strings.

- [Overview](#overview)
- [Lexical rules](#lexical-rules)
- [Types and literals](#types-and-literals)
- [Variables](#variables)
- [Built-in variables](#built-in-variables)
- [Operators and precedence](#operators-and-precedence)
- [Functions and conditionals](#functions-and-conditionals)
- [Evaluation semantics](#evaluation-semantics)
- [Optimizer](#optimizer)
- [Formal grammar](#formal-grammar)
- [Examples](#examples)
- [Common errors](#common-errors)

## Overview

Every entry in `exprs` has the form

```
EXPRESSION -> {OUTPUT}
```

- `EXPRESSION` computes a value.
- `{OUTPUT}` is a name (in braces) that is a key of the node's `outputs` table. The value of the expression is sent to the device associated with that key. Exactly one output is allowed per expression.

Entries in `defs` have no `->`; the node adds the target for you (`name = "EXPRESSION"` behaves like `EXPRESSION -> {name}`, where `name` becomes a new variable). A `defs` expression may only refer to `inputs`, not to other `defs` entries or itself.

An expression is re-evaluated whenever any of its inputs changes. When it produces a value that differs from the last one sent to the output device, the new value is sent. If it produces no value (see [None](#none-no-value)), nothing is sent.

```toml
[[logic]]
name = "porch light"
inputs  = { dark = "porch:dark:state", motion = "porch:motion:state" }
outputs = { light = "porch:light:enable" }
exprs   = ["{dark} and {motion} -> {light}"]
```

## Lexical rules

- Whitespace (spaces, tabs, newlines) between tokens is ignored. It is **not** allowed inside `{...}`.
- Keywords (`if`, `then`, `else`, `end`, `not`, `and`, `or`, `true`, `false`, `coalesce`, `with_alpha`, `brightness` and `blend`) are case-insensitive (`NOT`, `Not` and `not` are equivalent).
- Names inside `{...}`, and the built-in category and field names, are case-sensitive.
- There are no comments.
- Any other character is a lexical error.

Tokens:

| Token | Form |
|-------|------|
| arrow | `->` |
| punctuation | `(` `)` `,` `{` `}` `:` |
| comparison | `=` `<>` `<` `<=` `>` `>=` |
| arithmetic | `+` `-` `*` `/` `%` |
| boolean operators | `not` `and` `or` |
| boolean literals | `true` `false` |
| integer | `[0-9]+` |
| float | `[0-9]+\.[0-9]*([eE][-+]?[0-9]+)?`, `\.[0-9]+([eE][-+]?[0-9]+)?` or `[0-9]+[eE][-+]?[0-9]+` |
| string | `"[^"]*"` |
| color | `#` followed by hex digits, a color name or a temperature (see [Colors](#colors)) |
| identifier | `[a-zA-Z][0-9a-zA-Z_]*` (only inside `{...}`) |

> **Note:** a leading `-` is the unary minus operator, not part of the number. `-7` is the negation of `7`; the compiler folds it into a literal.

## Types and literals

Values have one of the following types:

| Type | Literal examples | Notes |
|------|------------------|-------|
| Boolean | `true`, `false` | |
| Integer | `0`, `42`, `-7` | 32-bit signed (`-2147483648 .. 2147483647`). Out-of-range literals are a compile error. |
| Float | `1.0`, `1.`, `.5`, `1e3`, `2.75e-10`, `1.0E+3` | 64-bit. Either a `.` or an exponent is required (otherwise it's an integer). No infinity or NaN. |
| String | `"hello"` | No escape sequences; a string cannot contain `"`. |
| Color | `#ff8000`, `#4000K@50%` | Either an RGBA color or a white color temperature. |

Devices can also supply these types (or others) at run time; the variables you read from `inputs` have whatever type the device reports.

### Colors

Color literals start with `#`, with **no space** after it.

**RGB / RGBA** — hex digits, case-insensitive. Valid lengths are 3, 4, 6 or 8 digits:

| Form | Meaning |
|------|---------|
| `#rgb` | each digit is doubled: `#f80` = `#ff8800`; alpha is 255 |
| `#rgba` | each digit is doubled |
| `#rrggbb` | alpha is 255 |
| `#rrggbbaa` | explicit alpha |

Any other digit count (e.g. `#12345`) is a compile error.

**Named colors** — `#` followed by a color name, case-insensitive: `#red`, `#AliceBlue`. The names are the SVG/CSS3 color keywords. A name that is also valid hex (such as `#bad` or `#fade`) is read as hex. Named colors have alpha 255. Unknown names are a compile error (`invalid color 'name'`).

**Color temperature** — `#<kelvin>K[@<alpha>]`:

| Form | Meaning |
|------|---------|
| `#2700K` | 2700 K at full alpha (255) |
| `#4000K@128` | alpha as an integer 0–255 |
| `#4500K@50%` | alpha as a percentage 0–100 (rounded down: `50%` = 127) |

- Kelvin must be in the range **2000–6500**.
- The `K` must be upper case.
- `alpha` acts as the brightness of the white light.

## Variables

`{name}` reads the most recent value of the input named `name`. `name` must be a key from the node's `inputs` table or a key from its `defs` table (in `exprs` only). Referring to an undefined name is a compile error: `variable 'name' is not defined`.

If a device hasn't reported a value yet, its variable has no value (see [None](#none-no-value)).

The target of `->` must be a key of `outputs`; outputs cannot be read back in an expression (use an `inputs` entry for the same device if you need its state).

## Built-in variables

Built-ins use `{category:field}`. Unknown categories or fields are compile errors.

### Time: `{utc:...}` and `{local:...}`

`utc` is Coordinated Universal Time; `local` is the system's local time zone. Each field is evaluated against the current time.

| Field | Type | Range / meaning |
|-------|------|-----------------|
| `second` | int | 0–59 |
| `minute` | int | 0–59 |
| `hour` | int | 0–23 |
| `day` | int | day of month, 1–31 |
| `month` | int | 1–12 |
| `year` | int | calendar year |
| `DOW` | int | day of week, 0–6 (Monday = 0, Sunday = 6) |
| `DOY` | int | day of year, 0–364. Leap years are adjusted so that Feb 29 has the same value as Feb 28 and all later days match a non-leap year |
| `SOM` | int | "Nth weekday from the start of the month": 1 for days 1–7, 2 for days 8–14, … up to 5 |
| `EOM` | int | "Nth weekday from the end of the month": 1 for the last 7 days of the month, 2 for the 7 before that, … |
| `LY` | bool | `true` in a leap year |

`SOM` and `EOM` make rules such as "the second Tuesday of the month" easy: `{local:DOW} = 1 and {local:SOM} = 2`.

A node only wakes up for time changes at the granularity it needs. If the finest time field used is `hour`, it is re-evaluated once per hour; if `second` is used, once per second. Using time fields therefore costs nothing unless they are referenced.

### Solar: `{solar:...}`

The sun's position, computed from the latitude and longitude in the DrMem configuration. All fields are floats.

| Field | Meaning |
|-------|---------|
| `alt` | elevation above the horizon; negative means the sun is below the horizon |
| `az` | azimuth |
| `ra` | right ascension |
| `dec` | declination |

Solar values are refreshed every 15 seconds. Until the first update arrives (or if the information isn't available), solar variables have no value.

## Operators and precedence

From lowest to highest binding:

| Level | Operators | Operand types | Result |
|-------|-----------|---------------|--------|
| 0 | `if … then … [else …] end` | condition: bool | value of chosen branch |
| 1 | `or` | bool, bool | bool |
| 2 | `and` | bool, bool | bool |
| 3 | `=` `<>` `<` `<=` `>` `>=` | see below | bool |
| 4 | `+` `-` | numeric | numeric |
| 5 | `*` `/` `%` | numeric | numeric |
| 6 | unary `-`, `not` | numeric, bool | numeric, bool |
| 7 | `( … )`, functions, literals, variables | | |

Things to know:

- Parentheses group sub-expressions and may contain any expression, including `if`, `and` and `or`.
- Comparison operators are **non-associative**: `1 < 2 < 3` is a syntax error. Combine them with `and`: `1 < 2 and 2 < 3`.
- `not` binds tighter than comparison, so `not a = b` means `(not a) = b`. Write `not (a = b)` or `a <> b` if that's what you want.
- `+ - * / %`, `and` and `or` all group **to the left**: `10 - 3 - 2` is `(10 - 3) - 2` = 5, `100 / 10 / 5` is `(100 / 10) / 5` = 2 and `a or b or c` is `(a or b) or c`.
- Unary minus negates a numeric operand: `-{a}`, `-(1 + 2)`, `3 - -2`. It binds tighter than `*`, so `-a * b` is `(-a) * b`. `-{a}` is evaluated as `0 - {a}`; literals such as `-7` and `-1.5` are folded into negative literals.
- `>`, `>=` and `<>` are rewritten internally to `<` (swapped operands), `<=` (swapped operands) and `not (… = …)`.

### Comparison

| Operator | Meaning |
|----------|---------|
| `a = b` | equal |
| `a <> b` | not equal |
| `a < b`, `a <= b` | less than, less than or equal |
| `a > b`, `a >= b` | greater than, greater than or equal |

| Operand types | `=` `<>` | `<` `<=` `>` `>=` |
|---------------|:--------:|:-----------------:|
| bool, bool | yes | **error** |
| int, int | yes | yes |
| float, float | yes | yes |
| int, float (either order) | yes (int converted to float) | yes |
| string, string | yes | yes (lexicographic) |
| anything else (colors, mixed types) | error | error |

### Arithmetic

| Operator | Meaning | Notes |
|----------|---------|-------|
| `a + b` | add | |
| `a - b` | subtract | |
| `a * b` | multiply | |
| `a / b` | divide | integer ÷ integer truncates toward zero (`7 / 2` = 3). Division by zero is an error. |
| `a % b` | remainder | `b` must be **greater than zero**; zero and negative divisors are errors. |

Operand types: int and float in any combination. For `+ - *` a bool (`true` = 1, `false` = 0) may be combined with an int or float, but two bools can't be combined. If either operand is a float the result is a float; otherwise it's an integer. Strings and colors can't be used in arithmetic. Integer arithmetic is 32-bit.

`/` and `%` do not accept bool operands.

### Boolean

| Expression | Meaning |
|------------|---------|
| `not a` | complement |
| `a and b` | both true; short-circuits (`b` isn't evaluated when `a` is `false`) |
| `a or b` | either true; short-circuits (`b` isn't evaluated when `a` is `true`) |

Operands **must** be boolean; there is no implicit conversion from numbers. See [None](#none-no-value) for how missing values interact with `and`/`or`.

## Functions and conditionals

### `if … then … [else …] end`

```
if CONDITION then EXPR [else EXPR] end
```

- `CONDITION` must evaluate to a boolean; anything else is an error.
- If true, the result is the `then` expression; if false, the `else` expression.
- Without `else`, a false condition produces **no value** (nothing is sent), which makes it handy for "only update the output when …" rules.
- Only the selected branch is evaluated, and the branches may have different types.
- `if` is an ordinary operand, so it can be nested and used inside larger expressions: `1 + if {a} then 1 else 2 end`.

### `coalesce(e1, e2, …)`

Returns the value of the first argument that has a value. Returns no value if none do. Arguments are evaluated left to right and evaluation stops at the first value. At least one argument is required.

```
coalesce({override}, {schedule}, #2700K) -> {bulb}
```

### `blend(c1, c2, …)`

Composites colors "over" one another, like layers. The **first** argument is the top layer and the **last** is the bottom layer; the colors' alpha channels determine how much of each layer shows through. At least one argument is required.

- Arguments with no value are skipped (treated as fully transparent).
- Arguments that have a value but are not colors are an error (no value is produced).
- If all arguments are temperatures, the result is a temperature (kelvin values are weighted by alpha).
- If any RGB color is involved, temperatures are converted to RGB and the result is RGBA.
- If no argument has a value, the result is fully transparent black (`#00000000`).

```
blend({alert_color}, with_alpha(#ff8000, 0.5), {base_color}) -> {bulb}
```

### `with_alpha(color, alpha)`

Returns `color` with its alpha replaced. `alpha` must be a number between 0 and 1, either a float or an integer (so `1` is opaque and `0` is transparent); values outside the range are clamped. Works with both RGB colors and temperatures. Both arguments are full expressions.

```
with_alpha({color}, {level} / 100.0) -> {bulb}
```

### `brightness(color)`

Returns a float describing the perceived brightness of a color:

- RGB: `(0.2126 R + 0.7152 G + 0.0722 B) × alpha / 255`, a value in 0.0–255.0.
- Temperature: the alpha value (0–255) as a float.

The argument is a full expression that must evaluate to a color, e.g. `brightness({color})` or `brightness(coalesce({a}, #fff))`.

## Evaluation semantics

### None (no value)

Many things can have "no value": a device that hasn't reported yet, solar data that isn't available, an `if` without `else` whose condition is false, an expression that failed. The rules are:

- An expression with no value sends nothing to its output.
- Arithmetic, comparison, `not` and function arguments (other than `coalesce`/`blend`) propagate no value: if an operand has none, so does the result.
- `or` treats a missing operand as "unknown": `true or <none>` is `true`, `false or <none>` is none. Similarly `false and <none>` is `false` and `true and <none>` is none. Order doesn't matter (`<none> or true` is `true`).
- `coalesce` and `blend` are the exceptions that handle missing values (see above); use `coalesce` to supply defaults.

### Errors at run time

Type errors (e.g. `"a" + 1`, `1 < true`, a non-boolean `if` condition, division by zero) are not detected at compile time. They are logged when the expression is evaluated, and the expression yields no value for that evaluation. The node keeps running and re-evaluates when inputs next change.

### Evaluation order

Each time an input, the time (at the granularity used) or the solar information changes:

1. All `defs` are evaluated and their results stored in their variables.
2. Then each `exprs` entry is evaluated, and the results sent to their outputs.

A value is only sent to a device if it differs from the last value sent to that device. If the driver adjusts the value (e.g. clamps it), a warning is logged.

## Optimizer

After parsing, each expression is simplified; this doesn't change results, but it's useful to know when reading debug logs.

- Constant sub-expressions are computed at startup (`60 * 60` → `3600`).
- Identities are removed: `x + 0`, `x - 0`, `x * 1`, `x / 1`, `true and x`, `false or x`, `not not x`.
- `x * 0`, `0 * x` and `0 / x` fold to `0`; `false and x` folds to `false`; `true or x` folds to `true`.
- Division or remainder by a literal zero becomes "never has a value" and the node logs a warning at startup: `expression … never generates a value`.
- `if true then A else B end` becomes `A`; `if false …` becomes `B` (or "no value" with no `else`).
- `coalesce` drops everything after the first literal; `blend` drops arguments that can never have a value.

## Formal grammar

Lowest-precedence rule first. `[ ]` means optional, `{ }` means zero or more, and quoted text is a token. Keywords are case-insensitive.

```
Statement   = Expr "->" "{" IDENT "}" ;

Expr        = OrExpr ;
OrExpr      = OrExpr "or" AndExpr
            | AndExpr ;
AndExpr     = AndExpr "and" CmpExpr
            | CmpExpr ;
CmpExpr     = Sum ( "=" | "<>" | "<" | "<=" | ">" | ">=" ) Sum
            | Sum ;
Sum         = Sum ( "+" | "-" ) Product
            | Product ;
Product     = Product ( "*" | "/" | "%" ) Factor
            | Factor ;

Factor      = "not" Factor
            | "-" Factor
            | "(" Expr ")"
            | "coalesce" "(" Expr { "," Expr } ")"
            | "blend" "(" Expr { "," Expr } ")"
            | "with_alpha" "(" Expr "," Expr ")"
            | "brightness" "(" Expr ")"
            | "if" Expr "then" Expr [ "else" Expr ] "end"
            | "true" | "false"
            | INT | FLOAT | STRING | COLOR | TEMPERATURE
            | "{" IDENT "}"
            | "{" IDENT ":" IDENT "}" ;

INT         = DIGIT { DIGIT } ;
FLOAT       = DIGIT { DIGIT } "." { DIGIT } [ EXPONENT ]
            | "." DIGIT { DIGIT } [ EXPONENT ]
            | DIGIT { DIGIT } EXPONENT ;
EXPONENT    = ( "e" | "E" ) [ "-" | "+" ] DIGIT { DIGIT } ;
STRING      = '"' { any character except '"' } '"' ;
COLOR       = "#" ( HEX{3|4|6|8} | NAME ) ;
TEMPERATURE = "#" DIGIT { DIGIT } "K" [ "@" DIGIT { DIGIT } [ "%" ] ] ;
IDENT       = LETTER { LETTER | DIGIT | "_" } ;
```

Note that comparison operands are `Sum`s, which is why comparisons can't be chained, and that an `if` expression is a `Factor`, so `if … end` can appear wherever a value can.

## Examples

Simple pass-through:

```
{switch} -> {light}
```

Turn a light on when it's dark and there's motion:

```
{motion} and {solar:alt} < 0.0 -> {light}
```

Only between 10 PM and 6 AM local time:

```
{motion} and ({local:hour} >= 22 or {local:hour} < 6) -> {light}
```

Different colors by time of day:

```
if {solar:alt} > 10.0 then #4000K
else if {solar:alt} > -6.0 then #2700K@50%
else #ff4000 end end -> {bulb}
```

Use a default when a sensor hasn't reported:

```
coalesce({temperature}, 70.0) > 75.0 -> {fan}
```

Convert a 0–100 dimmer into a color alpha:

```
with_alpha(#2700K, {dimmer} / 100.0) -> {bulb}
```

Scale an integer percentage (mixing int and float produces a float):

```
{level} * 2.55 -> {pwm}
```

A shared `defs` value (in the configuration):

```toml
defs  = { night = "{solar:alt} < -6.0" }
exprs = ["{motion} and {night} -> {porch}",
         "not {night} -> {sign}"]
```

Second Tuesday of the month, in the evening:

```
{local:DOW} = 1 and {local:SOM} = 2 and {local:hour} >= 18 -> {reminder}
```

Leave the output alone when the condition is false (`if` without `else` produces no value):

```
if {button} then #ffffff end -> {bulb}
```

## Common errors

| Symptom | Cause / fix |
|---------|-------------|
| `1 < 2 < 3` fails to parse | Comparisons don't chain. Use `and`. |
| `variable 'x' is not defined` | `{x}` isn't a key in `inputs` (or `defs`, in `exprs`), or the `->` target isn't a key in `outputs`. |
| `invalid color '12345'` | Hex colors have 3, 4, 6 or 8 digits; names must be SVG/CSS3 color keywords. |
| `invalid temperature '1999K'` | Temperature must be 2000–6500, upper-case `K`, alpha ≤ 255 or ≤ 100%. |
| `unknown built-in: utc:foo` | Check the field names in [Built-in variables](#built-in-variables); they're case-sensitive (`DOW`, not `dow`). |
| `… cannot be represented as an i32` | Integer literal outside the 32-bit range; write it as a float (`3000000000.0`). |
| `not {a} = {b}` is always an error or surprising | It's `(not {a}) = {b}`. Use `not ({a} = {b})`. |
| Expression never sends anything | One of its operands is always missing, or a run-time type error (see the log). Check for `never generates a value` warnings in the log at startup. |
