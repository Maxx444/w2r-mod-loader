"""Reference validator for plugin.json (docs/manifest.md). Returns "ok" or a reason code."""
import re

from . import jsonutil

MAX_BYTES = 64 * 1024
FIELDS = {
    "schema", "id", "name", "version", "author", "description", "entry", "abi",
    "game_builds", "capabilities", "affects_simulation", "dependencies",
}
OPTIONAL_FIELDS = {"replaces"}
CAPABILITIES = {"game.read", "chat.read", "chat.write", "net.send", "hooks.raw"}
RESERVED_DEVICES = {"CON", "PRN", "AUX", "NUL"} | {"COM%d" % i for i in range(1, 10)} | {
    "LPT%d" % i for i in range(1, 10)
}

ID_RE = re.compile(r"^[a-z0-9][a-z0-9-]{1,62}[a-z0-9]$")
_PRE = r"(0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)"
SEMVER_RE = re.compile(
    r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-" + _PRE + r"(\." + _PRE + r")*)?$"
)
_BUILD = r"[0-9]{1,5}(?:\.[0-9]{1,5}){3}"
BUILD_RE = re.compile(r"^(" + _BUILD + r")(?:-(" + _BUILD + r"))?$")
ENTRY_RE = re.compile(r"^[A-Za-z0-9_][A-Za-z0-9_.-]{0,59}\.dll$")
_NUM = r"(?:0|[1-9][0-9]*)"
_CMP = r"(?:>=|<=|>|<|=)" + _NUM + r"\." + _NUM + r"\." + _NUM
REQ_RE = re.compile(r"^" + _CMP + r"(?:," + _CMP + r"){0,3}$")

U32_MAX = 2**32 - 1


def valid_id(s):
    return isinstance(s, str) and ID_RE.fullmatch(s) is not None and s.upper() not in RESERVED_DEVICES


def valid_semver(s):
    """SemVer without build metadata, at most 64 bytes, numeric parts at most 2**32 - 1."""
    if not isinstance(s, str) or len(s) > 64:
        return False
    m = SEMVER_RE.fullmatch(s)
    return m is not None and all(int(m.group(i)) <= U32_MAX for i in (1, 2, 3))


def _req_ok(req):
    return (isinstance(req, str) and REQ_RE.fullmatch(req) is not None
            and all(int(n) <= U32_MAX for n in re.findall(r"[0-9]+", req)))


REASONS = {
    "too_large", "not_json", "duplicate_key", "not_object", "unknown_field", "missing_field",
    "bad_schema", "bad_id", "bad_type", "bad_string", "too_long", "bad_version", "bad_entry",
    "bad_abi", "bad_build", "bad_capability", "bad_dependency", "bad_replaces",
}


def _is_int(v):
    return isinstance(v, int) and not isinstance(v, bool)


def _text(v, max_bytes):
    if not isinstance(v, str):
        return "bad_type"
    if not v or any(ord(c) < 0x20 or ord(c) == 0x7F for c in v):
        return "bad_string"
    if len(v.encode("utf-8")) > max_bytes:
        return "too_long"
    return None


def _build_tuple(s):
    return tuple(int(p) for p in s.split("."))


def _builds_ok(builds):
    if not isinstance(builds, list):
        return "bad_type"
    if not 1 <= len(builds) <= 16:
        return "bad_build"
    for b in builds:
        m = BUILD_RE.fullmatch(b) if isinstance(b, str) else None
        if not m:
            return "bad_build"
        if m.group(2) and _build_tuple(m.group(1)) > _build_tuple(m.group(2)):
            return "bad_build"
    return None


def _entry_ok(e):
    if not isinstance(e, str) or not ENTRY_RE.fullmatch(e) or ".." in e:
        return False
    return e.split(".")[0].upper() not in RESERVED_DEVICES


def _deps_ok(deps, own_id):
    if not isinstance(deps, list):
        return "bad_type"
    if len(deps) > 16:
        return "bad_dependency"
    seen = set()
    for d in deps:
        if not isinstance(d, dict) or set(d) != {"id", "version_req"}:
            return "bad_dependency"
        did, req = d["id"], d["version_req"]
        if not valid_id(did) or did == own_id or did in seen or not _req_ok(req):
            return "bad_dependency"
        seen.add(did)
    return None


def _replaces_ok(replaces, own_id, deps):
    """`replaces` has the shape of `dependencies`; no id may be both replaced and depended on."""
    err = _deps_ok(replaces, own_id)
    if err == "bad_type":
        return err
    if err or {d["id"] for d in replaces} & {d["id"] for d in deps}:
        return "bad_replaces"
    return None


def _caps_ok(caps):
    if not isinstance(caps, list):
        return "bad_type"
    if not all(isinstance(c, str) and c in CAPABILITIES for c in caps):
        return "bad_capability"
    if len(set(caps)) != len(caps):
        return "bad_capability"
    return None


def validate_manifest(raw):
    if len(raw) > MAX_BYTES:
        return "too_large"
    try:
        doc = jsonutil.parse(raw)
    except jsonutil.Reject as r:
        return r.code
    if not isinstance(doc, dict):
        return "not_object"
    if set(doc) - FIELDS - OPTIONAL_FIELDS:
        return "unknown_field"
    if FIELDS - set(doc):
        return "missing_field"
    if not _is_int(doc["schema"]) or doc["schema"] != 1:
        return "bad_schema"
    if not valid_id(doc["id"]):
        return "bad_id"
    for field, limit in (("name", 64), ("author", 64), ("description", 256)):
        err = _text(doc[field], limit)
        if err:
            return err
    if not valid_semver(doc["version"]):
        return "bad_version"
    if not _entry_ok(doc["entry"]):
        return "bad_entry"
    if not _is_int(doc["abi"]) or not 1 <= doc["abi"] <= U32_MAX:
        return "bad_abi"
    err = _builds_ok(doc["game_builds"]) or _caps_ok(doc["capabilities"])
    if err:
        return err
    if not isinstance(doc["affects_simulation"], bool):
        return "bad_type"
    err = _deps_ok(doc["dependencies"], doc["id"])
    if err:
        return err
    if "replaces" in doc:
        return _replaces_ok(doc["replaces"], doc["id"], doc["dependencies"]) or "ok"
    return "ok"
