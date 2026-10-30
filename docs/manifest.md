# Plugin manifest (`plugin.json`)

Every plugin ships a `plugin.json` next to its DLL (see [layout.md](layout.md)). The manager
validates it at install and the host validates it again at load ([load-policy.md](load-policy.md)).
The rules below are normative; `tools/w2r_spec/manifest.py` is the executable reference and
`docs/vectors/manifest.json` holds the test vectors.

## Encoding and size

- UTF-8, no byte order mark, at most 65,536 bytes.
- Strict JSON: duplicate keys, `NaN`/`Infinity`, a BOM, invalid UTF-8, lone surrogates (`\ud800`
  without its pair, in a string or a key) and nesting deeper than **32** containers (the root object
  is depth 1) are all refused as `not_json` (`duplicate_key` for repeated keys). While parsing, the
  first problem met in document order wins; the surrogate and depth checks run after a successful
  parse. Every implementation must use these exact limits so results match.
- Every pattern below must match the **whole** string. A trailing newline is not accepted (use
  full-match semantics, not an end-of-line anchor).

## Example

```json
{
  "schema": 1,
  "id": "example-plugin",
  "name": "Example Plugin",
  "version": "1.2.0",
  "author": "someone",
  "description": "One sentence.",
  "entry": "example_plugin.dll",
  "abi": 1,
  "game_builds": ["1.0.2.2818"],
  "capabilities": ["game.read", "chat.read"],
  "affects_simulation": false,
  "dependencies": []
}
```

## Fields

Twelve fields are required and one is optional (`replaces`, below). No other fields are allowed.

| Field | Type | Rule | Reason code |
|---|---|---|---|
| `schema` | integer | exactly `1`; any other value, type or a boolean is refused | `bad_schema` |
| `id` | string | `[a-z0-9][a-z0-9-]{1,62}[a-z0-9]` (3 to 64 characters) and not a Windows device name (`con`, `prn`, `aux`, `nul`, `com1`-`com9`, `lpt1`-`lpt9`) | `bad_id` |
| `name` | string | 1 to 64 bytes UTF-8, no control characters (U+0000-U+001F, U+007F) | `bad_string`, `too_long`, `bad_type` |
| `author` | string | same as `name` | same |
| `description` | string | 1 to 256 bytes, same character rule | same |
| `version` | string | SemVer 2.0.0 without build metadata (`1.2.0`, `2.0.0-beta.1`); no leading zeros; at most 64 bytes; major, minor and patch at most 4,294,967,295 | `bad_version` |
| `entry` | string | `[A-Za-z0-9_][A-Za-z0-9_.-]{0,59}.dll`, no `..`, stem not a Windows device name | `bad_entry` |
| `abi` | integer | 1 to 4,294,967,295; the minimum ABI version the plugin needs | `bad_abi` |
| `game_builds` | array | 1 to 16 entries, each `a.b.c.d` or an inclusive range `a.b.c.d-w.x.y.z` | `bad_build`, `bad_type` |
| `capabilities` | array | duplicate-free subset of `game.read`, `chat.read`, `chat.write`, `net.send`, `hooks.raw` | `bad_capability`, `bad_type` |
| `affects_simulation` | boolean | must be a JSON boolean (`1` is refused) | `bad_type` |
| `dependencies` | array | at most 16 objects, see below | `bad_dependency`, `bad_type` |
| `replaces` | array | optional; same entries as `dependencies`, see below | `bad_replaces`, `bad_type` |

Notes:

- **`id`** is used to name folders and settings files (`plugins\<id>\`, `config\<id>.json`), so it
  is restricted to a path-safe set and may not be a Windows device name.
  It never reaches a path unchecked.
- **`entry`** is a plain file name inside the plugin's own folder. Separators, drive letters, UNC
  forms, a leading dot, `..` and Windows device names (`CON`, `PRN`, `AUX`, `NUL`, `COM1`-`COM9`,
  `LPT1`-`LPT9`, case-insensitive, with any extension) are refused.
- **`game_builds`**: each part is 1 to 5 digits. A range has min <= max, compared numerically
  component by component. Matching a running build against the list is defined in
  [load-policy.md](load-policy.md).
- **`capabilities`** lists only capabilities that need approval. The implicit capabilities
  (`log`, `settings`, `ui`, `fs.data`) are not listed; listing one is refused as unknown.
- **`dependencies`** entries are exactly `{"id": ..., "version_req": ...}`. `id` follows the id
  rule (device names included), is not the plugin's own id and does not repeat. `version_req` is 1
  to 4 comparators `(>=|<=|>|<|=)MAJOR.MINOR.PATCH` joined by `,` with no spaces, for example
  `>=1.0.0,<2.0.0`; numbers have no leading zeros and are at most 4,294,967,295.
- **`replaces`** (optional, absent means none) lists plugins this one supersedes, for a rename, a
  merge or a successor. Entries have the same shape and rules as `dependencies` (`id`,
  `version_req`, at most 16, no repeats, not the plugin's own id), and an id may not appear in both
  lists. `version_req` says which versions of the old plugin are replaced, so a cap such as
  `<1.0.0` is written here, on the new plugin: a published plugin version never changes, so the
  old one cannot carry it. Any problem is `bad_replaces` (`bad_type` if it is not an array). The
  host skips a replaced plugin once its replacer has loaded (`E_REPLACED`,
  [load-policy.md](load-policy.md)), and the manager disables it when the replacer is installed
  enabled. A loader older than this field refuses the manifest as `unknown_field`. The replacer may
  read the replaced plugin's stored whole-number settings with `replaced_setting_get_i64`
  ([abi.md](abi.md), "Settings of a replaced plugin").

## Reason codes

The first failing check wins, in this order: size, JSON syntax (`too_large`, `not_json`,
`duplicate_key`), top-level type (`not_object`), field set (`unknown_field`, `missing_field`),
then each field in the order of the table above.

| Code | Meaning |
|---|---|
| `too_large` | more than 65,536 bytes |
| `not_json` | invalid UTF-8 or JSON, BOM, `NaN`, runaway nesting |
| `duplicate_key` | an object repeats a key |
| `not_object` | top level is not an object |
| `unknown_field` / `missing_field` | a required field is missing or one outside the thirteen is present |
| `bad_schema`, `bad_id`, `bad_version`, `bad_entry`, `bad_abi`, `bad_build`, `bad_capability`, `bad_dependency`, `bad_replaces` | that field violates its rule |
| `bad_type` | wrong JSON type for a container or boolean field |
| `bad_string` | empty, or contains a control character |
| `too_long` | string over its byte limit |

## Not checked here

- Whether dependencies exist, are enabled or form a cycle: load policy (`E_DEP`).
- Whether the running game build matches `game_builds`: load policy (`E_BUILD`).
- DLL hashes. The manifest carries none, so it cannot vouch for itself. The pin lives in
  `state.json` and the package hash in the signed [index](index.md).

## Test vectors

`docs/vectors/manifest.json` is a list of cases `{name, expect, manifest | raw[, pad]}`. Regenerate
with `python3 tools/gen_manifest_vectors.py`; run everything with `bash tools/run_all.sh`. Every
non-ok reason code has at least one vector. A Rust parser in Phase 4 must produce the same result
for every case.
