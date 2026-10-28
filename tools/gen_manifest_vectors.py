"""Regenerate docs/vectors/manifest.json. Run: python3 tools/gen_manifest_vectors.py"""
import json
from pathlib import Path

from w2r_spec.samples import base_manifest

OUT = Path(__file__).resolve().parents[1] / "docs" / "vectors" / "manifest.json"


def mut(**changes):
    m = base_manifest()
    m.update(changes)
    return m


def drop(key):
    m = base_manifest()
    del m[key]
    return m


DEP = {"id": "other-plugin", "version_req": ">=1.0.0,<2.0.0"}

OBJECT_CASES = [
    ("valid_example", "ok", mut()),
    ("valid_minimal", "ok", mut(capabilities=[], dependencies=[], version="0.0.1", id="ab0")),
    ("valid_full", "ok", mut(
        version="2.0.0-beta.1", game_builds=["1.0.2.2818", "1.0.2.2900-1.0.3.100"],
        capabilities=["game.read", "chat.read", "chat.write", "net.send", "hooks.raw"],
        affects_simulation=True, dependencies=[DEP])),
    ("schema_2", "bad_schema", mut(schema=2)),
    ("schema_string", "bad_schema", mut(schema="1")),
    ("schema_bool", "bad_schema", mut(schema=True)),
    ("unknown_field", "unknown_field", mut(extra="x")),
    ("missing_id", "missing_field", drop("id")),
    ("missing_capabilities", "missing_field", drop("capabilities")),
    ("id_uppercase", "bad_id", mut(id="Example")),
    ("id_too_short", "bad_id", mut(id="ab")),
    ("id_too_long", "bad_id", mut(id="a" * 65)),
    ("id_leading_hyphen", "bad_id", mut(id="-abc")),
    ("id_trailing_hyphen", "bad_id", mut(id="abc-")),
    ("id_dotdot", "bad_id", mut(id="../x")),
    ("id_not_string", "bad_id", mut(id=5)),
    ("version_leading_zero", "bad_version", mut(version="01.2.0")),
    ("version_build_metadata", "bad_version", mut(version="1.2.0+abc")),
    ("version_two_parts", "bad_version", mut(version="1.2")),
    ("version_prerelease_leading_zero", "bad_version", mut(version="1.2.0-01")),
    ("entry_slash", "bad_entry", mut(entry="dir/x.dll")),
    ("entry_backslash", "bad_entry", mut(entry="dir\\x.dll")),
    ("entry_dotdot", "bad_entry", mut(entry="a..dll")),
    ("entry_unc", "bad_entry", mut(entry="\\\\host\\share\\x.dll")),
    ("entry_drive", "bad_entry", mut(entry="C:x.dll")),
    ("entry_not_dll", "bad_entry", mut(entry="x.exe")),
    ("entry_reserved_device", "bad_entry", mut(entry="CON.dll")),
    ("entry_reserved_device_lower", "bad_entry", mut(entry="nul.dll")),
    ("entry_leading_dot", "bad_entry", mut(entry=".x.dll")),
    ("id_trailing_newline", "bad_id", mut(id="abc\n")),
    ("id_device_con", "bad_id", mut(id="con")),
    ("id_device_nul", "bad_id", mut(id="nul")),
    ("id_device_com1", "bad_id", mut(id="com1")),
    ("id_device_lpt9", "bad_id", mut(id="lpt9")),
    ("entry_trailing_newline", "bad_entry", mut(entry="foo.dll\n")),
    ("version_trailing_newline", "bad_version", mut(version="1.0.0\n")),
    ("version_part_too_large", "bad_version", mut(version="1.2.4294967296")),
    ("version_part_max", "ok", mut(version="4294967295.0.0")),
    ("version_too_long", "bad_version", mut(version="1.0.0-" + "a" * 59)),
    ("version_max_length", "ok", mut(version="1.0.0-" + "a" * 58)),
    ("build_trailing_newline", "bad_build", mut(game_builds=["1.0.2.2818\n"])),
    ("abi_too_large", "bad_abi", mut(abi=2**32)),
    ("abi_max", "ok", mut(abi=2**32 - 1)),
    ("dep_req_trailing_newline", "bad_dependency", mut(dependencies=[{"id": "other-plugin", "version_req": ">=1.0.0\n"}])),
    ("dep_req_leading_zero", "bad_dependency", mut(dependencies=[{"id": "other-plugin", "version_req": ">=01.0.0"}])),
    ("dep_req_too_large", "bad_dependency", mut(dependencies=[{"id": "other-plugin", "version_req": ">=1.0.4294967296"}])),
    ("dep_id_device", "bad_dependency", mut(dependencies=[{"id": "aux", "version_req": ">=1.0.0"}])),
    ("abi_zero", "bad_abi", mut(abi=0)),
    ("abi_bool", "bad_abi", mut(abi=True)),
    ("abi_string", "bad_abi", mut(abi="1")),
    ("build_three_parts", "bad_build", mut(game_builds=["1.0.2"])),
    ("build_range_reversed", "bad_build", mut(game_builds=["1.0.2.3000-1.0.2.2818"])),
    ("build_empty_list", "bad_build", mut(game_builds=[])),
    ("build_too_many", "bad_build", mut(game_builds=["1.0.0.%d" % i for i in range(17)])),
    ("builds_not_list", "bad_type", mut(game_builds="1.0.2.2818")),
    ("capability_unknown", "bad_capability", mut(capabilities=["net.recv"])),
    ("capability_implicit_refused", "bad_capability", mut(capabilities=["fs.data"])),
    ("capability_duplicate", "bad_capability", mut(capabilities=["game.read", "game.read"])),
    ("capabilities_not_list", "bad_type", mut(capabilities="game.read")),
    ("simulation_not_bool", "bad_type", mut(affects_simulation=1)),
    ("dep_self", "bad_dependency", mut(dependencies=[{"id": "example-plugin", "version_req": ">=1.0.0"}])),
    ("dep_duplicate", "bad_dependency", mut(dependencies=[DEP, DEP])),
    ("dep_bad_req", "bad_dependency", mut(dependencies=[{"id": "other-plugin", "version_req": "^1.0"}])),
    ("dep_extra_field", "bad_dependency", mut(dependencies=[dict(DEP, extra=1)])),
    ("dep_bad_id", "bad_dependency", mut(dependencies=[{"id": "Bad", "version_req": ">=1.0.0"}])),
    ("dep_too_many", "bad_dependency", mut(dependencies=[
        {"id": "dep-%02d" % i, "version_req": ">=1.0.0"} for i in range(17)])),
    ("replaces_ok", "ok", mut(replaces=[{"id": "old-plugin", "version_req": "<1.0.0"}])),
    ("replaces_empty_ok", "ok", mut(replaces=[])),
    ("replaces_with_dependency_ok", "ok", mut(dependencies=[DEP], replaces=[
        {"id": "old-plugin", "version_req": ">=0.1.0,<1.0.0"}])),
    ("replaces_not_list", "bad_type", mut(replaces="old-plugin")),
    ("replaces_self", "bad_replaces", mut(replaces=[{"id": "example-plugin", "version_req": "<1.0.0"}])),
    ("replaces_duplicate", "bad_replaces", mut(replaces=[
        {"id": "old-plugin", "version_req": "<1.0.0"}, {"id": "old-plugin", "version_req": ">=2.0.0"}])),
    ("replaces_bad_req", "bad_replaces", mut(replaces=[{"id": "old-plugin", "version_req": "^1.0"}])),
    ("replaces_extra_field", "bad_replaces", mut(replaces=[{"id": "old-plugin", "version_req": "<1.0.0", "x": 1}])),
    ("replaces_bad_id", "bad_replaces", mut(replaces=[{"id": "Bad", "version_req": "<1.0.0"}])),
    ("replaces_too_many", "bad_replaces", mut(replaces=[
        {"id": "old-%02d" % i, "version_req": "<1.0.0"} for i in range(17)])),
    ("replaces_also_dependency", "bad_replaces", mut(dependencies=[DEP], replaces=[
        {"id": "other-plugin", "version_req": "<1.0.0"}])),
    ("unknown_field_still_refused", "unknown_field", mut(replace=[])),
    ("name_control_char", "bad_string", mut(name="Bad\u0007Name")),
    ("name_empty", "bad_string", mut(name="")),
    ("name_too_long", "too_long", mut(name="n" * 65)),
    ("description_too_long", "too_long", mut(description="d" * 257)),
    ("author_not_string", "bad_type", mut(author=7)),
]

RAW_CASES = [
    # (name, expect, raw text, trailing spaces to append)
    ("not_json", "not_json", "{", 0),
    ("not_object", "not_object", "[]", 0),
    ("nan", "not_json", '{"schema": NaN}', 0),
    ("bom", "not_json", "﻿" + json.dumps(base_manifest()), 0),
    ("duplicate_key", "duplicate_key",
     '{"schema": 1, "schema": 1}', 0),
    ("lone_surrogate_value", "not_json", '{"schema": 1, "name": "\\ud800"}', 0),
    ("lone_surrogate_key", "not_json", '{"\\udc00": 1}', 0),
    ("depth_32_reaches_field_checks", "unknown_field", '{"schema": 1, "x": ' + "[" * 31 + "]" * 31 + "}", 0),
    ("depth_33_refused", "not_json", '{"schema": 1, "x": ' + "[" * 32 + "]" * 32 + "}", 0),
    ("oversize", "too_large", json.dumps(base_manifest()), 70000),
]


def main():
    cases = []
    for name, expect, manifest in OBJECT_CASES:
        cases.append({"name": name, "expect": expect, "manifest": manifest})
    for name, expect, raw, pad in RAW_CASES:
        case = {"name": name, "expect": expect, "raw": raw}
        if pad:
            case["pad"] = pad
        cases.append(case)
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps(cases, indent=1, sort_keys=True) + "\n", encoding="utf-8")
    print("wrote", OUT, len(cases), "cases")


if __name__ == "__main__":
    main()
