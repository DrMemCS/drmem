# Configuration.md

## Templates

A `[[driver]]` or `[[logic]]` entry that contains only a `template` field is expanded into several entries when the configuration is loaded. `vars` defines the substitutions and `body` is TOML text in which `${var}` is replaced.

```toml
[[driver]]
template = { vars = { p = ["a", "b"] }, body = '''
name = "timer"
prefix = ${p}
''' }
```

- A `vars` table maps each variable to an array; all arrays must have the same length and are consumed in parallel (one expansion per index).
- A `vars` array of such tables yields the cross product of the tables.
- Substitution inserts a TOML value, so strings arrive quoted (use `${p}`, not `"${p}"`).
- Each expansion must be a valid driver (or logic block) definition. Duplicate names are reported when the entries are started.
