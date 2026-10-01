#!/usr/bin/env python3
"""Local, auditable public macro corpus; parsing is not execution certification.

Only anonymous GET routes observed in the public JX3BOX macro frontend are
supported. Raw source blocks and native evidence stay in the ignored target
directory. No online template lookup is added to the production synthesizer.
"""
from __future__ import annotations

import argparse
from collections import Counter
import copy
from datetime import datetime, timezone
from decimal import Decimal, InvalidOperation
import hashlib
from html.parser import HTMLParser
import json
from pathlib import Path
import re
import subprocess
import sys
import time
import tomllib
from urllib.error import HTTPError, URLError
from urllib.parse import urlencode, urlparse
from urllib.request import Request, urlopen

ROOT = Path(__file__).resolve().parents[1]
STORE = ROOT / "backend/target/exact-macro-corpus"
SCHEMA = "exact-macro-corpus-v1"
AST_SCHEMA = "native-compatible-conservative-subset-v1"
API = "https://cms.jx3box.com"
OBSERVED_ROUTES = {
    "entry": "https://www.jx3box.com/macro/",
    "service_script": "https://cdn.jx3box.com/static/pve/js/1802.1361821e.js",
    "list_script": "https://cdn.jx3box.com/static/pve/js/2388.7c731806.js",
    "detail_script": "https://cdn.jx3box.com/static/pve/js/1436.5e89f93a.js",
    "list": "/api/cms/posts",
    "detail": "/api/cms/post/{id}",
    "observed_at": "2026-10-02",
}
SOURCE_FIELDS = (
    "ID", "post_title", "post_subtype", "post_author", "author", "post_date",
    "post_modified", "client", "zlp", "is_wujie", "lang", "original",
    "visible", "post_status", "post_content", "post_mode", "post_meta", "tags",
)
MANUAL_WORDS = ("手动", "切页", "辅助宏", "提前", "预释放", "手打", "按键", "切换宏")


def digest(value):
    data = value if isinstance(value, bytes) else value.encode("utf-8")
    return hashlib.sha256(data).hexdigest()


def canonical(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def output_path(path):
    resolved = Path(path).resolve()
    if not resolved.is_relative_to(STORE.resolve()):
        raise ValueError("raw corpus output must stay under backend/target/exact-macro-corpus")
    return resolved


class TextContext(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.parts, self.hidden = [], 0

    def handle_starttag(self, tag, attrs):
        if tag in ("script", "style"):
            self.hidden += 1
        if tag in ("p", "div", "li", "br", "h1", "h2", "h3", "pre"):
            self.parts.append("\n")

    def handle_endtag(self, tag):
        if tag in ("script", "style"):
            self.hidden = max(0, self.hidden - 1)
        if tag in ("p", "div", "li", "h1", "h2", "h3", "pre"):
            self.parts.append("\n")

    def handle_data(self, data):
        if not self.hidden:
            self.parts.append(data)


def context_text(text, mode="tinymce"):
    if mode == "markdown":
        return text
    parser = TextContext()
    parser.feed(text)
    return "\n".join(line.strip() for line in "".join(parser.parts).splitlines() if line.strip())


def decimal_value(raw):
    try:
        value = Decimal(raw)
    except InvalidOperation as error:
        raise ValueError("invalid numeric threshold") from error
    if not value.is_finite():
        raise ValueError("non-finite numeric threshold")
    return format(value.normalize(), "f") if value else "0"


def parse_atom(text):
    """Conservative structure reader, never a replacement for Rust parsing."""
    text = text.strip()
    if any(char in text for char in "(),[]"):
        raise ValueError("unsupported punctuation in condition")
    match = re.fullmatch(r"(rage|energy|sun|berserk|baonu|life|nearby_enemy)(>=|<=|~=|>|<|=)(.+)", text)
    if match:
        field, op, raw = match.groups()
        value = decimal_value(raw.strip())
        if field != "life" and (Decimal(value) != Decimal(value).to_integral() or not -(2**31) <= Decimal(value) < 2**31):
            raise ValueError("integer condition outside native i32 domain")
        return {"kind": {"sun": "berserk", "baonu": "berserk"}.get(field, field), "op": op, "value": value}
    match = re.fullmatch(r"(bufftime|tbufftime|skill_energy|buff):(.+?)(>=|<=|~=|>|<|=)(.+)", text)
    if match:
        field, name, op, raw = match.groups()
        value = decimal_value(raw.strip())
        if field in ("buff", "skill_energy") and (Decimal(value) < 0 or Decimal(value) != Decimal(value).to_integral() or Decimal(value) >= 2**32):
            raise ValueError("fractional or negative stack/charge threshold requires native inspection")
        return {"kind": field + ("_stack" if field == "buff" else ""), "name": name.strip(), "op": op, "value": value}
    match = re.fullmatch(r"(buff|nobuff|tbuff|tnobuff|skill_notin_cd):(.+)", text)
    if match:
        return {"kind": match[1], "name": match[2].strip()}
    match = re.fullmatch(r"(skill|noskill):(\d+)", text)
    if match and int(match[2]) < 2**32:
        return {"kind": match[1], "id": int(match[2])}
    match = re.fullmatch(r"last_skill(~=|=)(.+)", text)
    if match:
        return {"kind": "last_skill", "op": match[1], "name": match[2].strip()}
    raise ValueError("unknown condition atom: " + text)


def parse_condition(text):
    parts = re.split(r"([&|])", text)
    if not parts or any(not item.strip() for item in parts):
        raise ValueError("empty condition leaf")
    leaves = [parse_atom(item) for item in parts[::2]]
    if len(leaves) > 512:
        raise ValueError("condition exceeds conservative local parser depth; not a language limit")
    tree = leaves[-1]
    for leaf, op in reversed(list(zip(leaves[:-1], parts[1::2]))):
        tree = {"op": op, "left": leaf, "right": tree}
    return tree


def parse_macro(text):
    pages, lines, stance, errors, normalized = [], [], None, [], []
    for number, raw in enumerate(text.splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("//"):
            continue
        try:
            if line.startswith("#page"):
                name = line[5:].strip()
                choices = {"": None, "shield": "shield", "擎盾": "shield", "blade": "blade", "擎刀": "blade", "wall": "wall", "盾墙": "wall"}
                if name not in choices:
                    raise ValueError("unsupported page selector")
                if lines:
                    pages.append({"stance_filter": stance, "lines": lines})
                    lines = []
                stance = choices[name]
                normalized.append(line)
                continue
            match = re.fullmatch(r"/(fcast|cast)\s+(.+)", line)
            if not match:
                raise ValueError("unsupported command or non-macro line")
            command, rest = match.groups()
            condition, skill, guard = None, rest, None
            if rest.startswith("["):
                close = rest.find("]")
                if close < 0:
                    raise ValueError("missing closing bracket")
                guard, skill = rest[1:close], rest[close + 1:].strip()
            elif re.match(r"(?:rage|energy|sun|berserk|baonu|life|nearby_enemy|buff|nobuff|tbuff|tnobuff|skill|noskill|last_skill)", rest):
                if " " not in rest:
                    raise ValueError("unbracketed condition needs skill separator")
                guard, skill = rest.rsplit(" ", 1)
            if not skill or any(char in skill for char in "[]"):
                raise ValueError("missing or malformed skill name")
            if guard is not None:
                condition = parse_condition(guard)
            lines.append({"command": command, "skill": skill, "condition": condition})
            normalized.append(f"/{command} " + (f"[{guard.strip()}] " if guard is not None else "") + skill)
        except ValueError as error:
            errors.append({"line": number, "reason": str(error), "raw_line": raw})
            normalized.append(raw)  # Unsupported input is retained, never silently discarded.
    if lines:
        pages.append({"stance_filter": stance, "lines": lines})
    if not pages:
        errors.append({"line": 0, "reason": "empty macro", "raw_line": ""})
    ast = {"schema": AST_SCHEMA, "pages": pages}
    return {"parse_status": "unsupported" if errors else "parsed_subset", "ast": ast,
            "parsed_ast_hash": None if errors else digest(canonical(ast)),
            "unsupported_features": errors, "normalized_text": "\n".join(normalized),
            "native_parse_status": "not_checked"}


def skeleton(value):
    if isinstance(value, dict):
        return {key: ("<number>" if key == "value" else skeleton(item)) for key, item in value.items()}
    if isinstance(value, list):
        return [skeleton(item) for item in value]
    return value


def safe_post(post):
    # Avoid persisting IPs, profiles, contact fields, comment feeds or counters.
    return {key: copy.deepcopy(post[key]) for key in SOURCE_FIELDS if key in post}


def safe_listing(post):
    # Lists may include a restricted item's metadata; do not retain its body.
    return {key: value for key, value in safe_post(post).items() if key not in ("post_meta", "post_content")}


class PublicClient:
    def __init__(self, out, delay=0.5, retries=2, refresh=False):
        self.out, self.delay, self.retries, self.last_call = out, max(0.5, delay), retries, 0.0
        self.refresh = refresh
        self.requests = []

    def get(self, url, kind):
        parsed = urlparse(url)
        if parsed.scheme != "https" or parsed.netloc != "cms.jx3box.com" or not (
            parsed.path == "/api/cms/posts" or re.fullmatch(r"/api/cms/post/\d+", parsed.path)
        ):
            raise ValueError("only observed public JX3BOX GET routes are permitted")
        path = self.out / "cache" / (digest(url) + ".json")
        if path.exists() and not self.refresh:
            saved = json.loads(path.read_text(encoding="utf-8"))
            if kind == "list":
                saved["data"]["list"] = [safe_listing(post) for post in saved["data"]["list"]]
                saved["request"]["stored_projection"] = "list_metadata_only"
                write_json(path, saved)
            self.requests.append(dict(saved["request"], from_cache=True))
            if kind == "detail":
                source_path = self.out / "sources" / str(saved["data"]["ID"]) / (saved["request"]["response_hash"] + ".json")
                if not source_path.exists():
                    write_json(source_path, saved)
            return saved["data"]
        for attempt in range(self.retries + 1):
            time.sleep(max(0.0, self.delay - (time.monotonic() - self.last_call)))
            self.last_call = time.monotonic()
            try:
                request = Request(url, headers={"User-Agent": "exact-macro-corpus/1.0 (local research)", "Accept": "application/json"})
                with urlopen(request, timeout=20) as response:
                    payload = response.read(5 * 1024 * 1024 + 1)
                    if len(payload) > 5 * 1024 * 1024:
                        raise ValueError("public response exceeds local collection limit")
                    result = json.loads(payload)
                    if result.get("code") != 0:
                        raise ValueError("public API did not return success")
                data = result["data"]
                if kind == "list":
                    data = {key: data.get(key) for key in ("page", "per", "pages", "total")}
                    data["list"] = [safe_listing(post) for post in result["data"]["list"]]
                else:
                    data = safe_post(data)
                    identity = str(data.get("ID"))
                    if not re.fullmatch(r"\d+", identity) or identity != parsed.path.rsplit("/", 1)[1]:
                        raise ValueError("public detail identity does not match its observed URL")
                    if str(data.get("visible")) != "0" or data.get("post_status") not in ("publish", "published"):
                        # Availability through an API alone does not authorize restricted content.
                        self.requests.append({"url": url, "method": "GET", "http_status": 200,
                            "status": "not_explicitly_public_published", "authentication": "none"})
                        raise ValueError("source is not explicitly public")
                metadata = {"url": url, "method": "GET", "http_status": 200,
                            "response_hash": digest(payload), "collected_at": datetime.now(timezone.utc).isoformat(),
                            "stored_projection": "list_metadata_only" if kind == "list" else "macro_and_provenance_fields_only", "authentication": "none"}
                write_json(path, {"request": metadata, "data": data})
                if kind == "detail":
                    source_path = self.out / "sources" / str(data["ID"]) / (metadata["response_hash"] + ".json")
                    if not source_path.exists():
                        write_json(source_path, {"request": metadata, "data": data})
                self.requests.append(dict(metadata, from_cache=False))
                return data
            except HTTPError as error:
                if error.code in (401, 403):
                    self.requests.append({"url": url, "http_status": error.code, "status": "access_restricted"})
                    raise ValueError("login or access restriction; collection stopped") from error
                if error.code not in (429, 500, 502, 503, 504) or attempt == self.retries:
                    raise
                retry_after = error.headers.get("Retry-After", "")
                wait = float(retry_after) if retry_after.isdigit() else 2**attempt
                time.sleep(min(5.0, max(self.delay, wait)))
            except URLError:
                if attempt == self.retries:
                    raise
                time.sleep(2**attempt)
        raise RuntimeError("public collection retry exhausted")


def records_for_post(post, request, list_context):
    blocks = (post.get("post_meta") or {}).get("data") or []
    article = context_text(post.get("post_content") or "", post.get("post_mode") or "tinymce")
    records = []
    for index, block in enumerate(blocks):
        raw = block.get("macro")
        if not isinstance(raw, str) or not raw.strip():
            continue
        parsed = parse_macro(raw)
        desc = context_text(str(block.get("desc") or ""))
        notes = [line for line in (article + "\n" + desc).splitlines() if any(word in line for word in MANUAL_WORDS)]
        derivation_notes = [line for line in (article + "\n" + desc).splitlines() if any(word in line for word in ("转载", "抄", "修改", "改编", "来自"))]
        ancestor_ids = sorted(set(re.findall(r"(?:www\.|origin\.)?jx3box\.com/macro/(\d+)", post.get("post_content") or "")) - {str(post["ID"])})
        talent = block.get("talent") or None
        try:
            talent_structure = json.loads(talent) if isinstance(talent, str) and talent else None
        except json.JSONDecodeError:
            talent_structure = None
        missing = ["complete_equipment", "attribute_panel", "target", "team_buffs", "formation", "recipes", "network_delay", "initial_state", "source_to_project_talent_id_mapping"]
        if not post.get("zlp"):
            missing.append("season")
        if len(blocks) > 1:
            missing.append("relation_between_source_blocks_and_page_selection")
        if not notes:
            missing.append("manual_steps")
        record = {
            "schema_version": SCHEMA, "record_id": f"jx3box:{post['ID']}:block:{index}",
            "source_url": f"https://www.jx3box.com/macro/{post['ID']}", "source_item_id": str(post["ID"]),
            "public_author_id": str(post["post_author"]) if post.get("post_author") not in (None, "") else None,
            "public_author_name": post.get("author"), "source_title": post.get("post_title"),
            "published_at": post.get("post_date"), "updated_at": post.get("post_modified"),
            "collected_at": request.get("collected_at"), "collection_request": request, "list_context": list_context,
            "client": post.get("client"), "season": post.get("zlp") or None,
            "mount": post.get("post_subtype") or None,
            "test_server": None,  # Posting dates/title/version names do not prove server type.
            "is_wujie": post.get("is_wujie"), "raw_text_hash": digest(raw),
            "source_original_flag": post.get("original"), "derived_from_source_ids": ancestor_ids,
            "source_derivation_notes": derivation_notes,
            "source_context_hash": digest(post.get("post_content") or ""),
            "reference_b_marker_in_source": "武学助手" in canonical(post),
            "macro_blocks": [{"source_block_index": index, "name": block.get("name"), "raw_text": raw}],
            "source_block_count": len(blocks), "page_selection": {"status": "native_explicit" if any(line.strip().startswith("#page") for line in raw.splitlines()) else "unknown" if len(blocks) > 1 else "single_source_block", "source_blocks_flattened": False},
            "manual_steps": notes, "manual_steps_complete": False,
            "talents": {"raw": talent, "source_structure": talent_structure, "project_ids": None},
            "recipes": None, "scenario_metadata": {"origin": "source_metadata_only", "author_scenario_reconstructed": False,
                "speed_raw": block.get("speed"), "equipment_note_raw": block.get("equip"), "block_description": desc},
            "missing_fields": missing, "replay_status": "not_run", "certification": None,
            "source_usage_notes": "Public user content, local research only. Frontend code license does not license user macro republication.",
            "reference_b_status": "not_detected_not_proof", "split": "review", **parsed,
        }
        record["near_duplicate_structure_hash"] = None if parsed["parse_status"] != "parsed_subset" else digest(canonical(skeleton(parsed["ast"])))
        records.append(record)
    return records


def assign_families(records, exclusions=None, reviewed_sources=(), test_sources=()):
    """Group before augmentation; aliases/constant variants never cross splits."""
    exclusions, reviewed_sources = exclusions or {}, set(map(str, reviewed_sources))
    test_sources = set(map(str, test_sources))
    parent = list(range(len(records)))
    def find(index):
        while parent[index] != index:
            parent[index] = parent[parent[index]]
            index = parent[index]
        return index
    def join(left, right):
        parent[find(right)] = find(left)
    seen = {}
    for index, record in enumerate(records):
        for kind in ("source_item_id", "public_author_id", "raw_text_hash", "parsed_ast_hash", "near_duplicate_structure_hash"):
            value = record.get(kind)
            if value is None:
                continue
            key = (kind, value)
            if key in seen:
                join(seen[key], index)
            else:
                seen[key] = index
        for identity in record.get("derived_from_source_ids", []):
            key = ("source_item_id", identity)
            if key in seen:
                join(seen[key], index)
            else:
                seen[key] = index
    groups = {}
    for index in range(len(records)):
        groups.setdefault(find(index), []).append(records[index])
    for members in groups.values():
        ids = sorted({record["source_item_id"] for record in members})
        group = "source-family-" + digest(canonical(ids))[:20]
        quarantined = any(
            "武学助手" in str(record.get("source_title") or "") + str(record["macro_blocks"][0].get("name") or "")
            or record.get("reference_b_marker_in_source", False)
            or any(record.get(key) is not None and record.get(key) in values for key, values in exclusions.items() if isinstance(values, list))
            or any(identity in exclusions.get("source_item_id", []) for identity in record.get("derived_from_source_ids", []))
            for record in members
        )
        reviewed = not quarantined and set(ids) <= reviewed_sources
        bucket = int(digest(group)[:8], 16) % 100
        proposal = "train" if bucket < 70 else "validation" if bucket < 85 else "test"
        explicit_test = bool(set(ids) & test_sources)
        for record in members:
            record.update(lineage_group_id=group, lineage_source_ids=ids,
                          lineage_policy="same_source_or_explicit_ancestor_or_author_or_exact_ast_or_numeric_skeleton",
                          reference_b_status="quarantined" if quarantined else "reviewed_non_b" if reviewed else "not_detected_not_proof",
                          proposed_split=proposal, split="quarantine" if quarantined else "test" if reviewed and explicit_test else proposal if reviewed else "review",
                          split_basis="reference_b_quarantine" if quarantined else "reviewed_whole_source_family_test" if reviewed and explicit_test else "reviewed_family_hash" if reviewed else "source_evidence_not_yet_reviewed")
    return records


def merge_records(previous, current, replaced_sources=()):
    """Append pagination results; only identical source versions retain a certificate."""
    replaced_sources = set(map(str, replaced_sources))
    current_ids = {record["record_id"] for record in current}
    merged = {record["record_id"]: copy.deepcopy(record) for record in previous
              if record["source_item_id"] not in replaced_sources or record["record_id"] in current_ids}
    for record in current:
        old = merged.get(record["record_id"])
        if old and all(old.get(key) == record.get(key) for key in ("raw_text_hash", "season", "mount", "client")):
            for key in ("certification", "replay_status", "native_parse_status", "project_compatibility"):
                if key in old:
                    record[key] = copy.deepcopy(old[key])
        merged[record["record_id"]] = record
    return sorted(merged.values(), key=lambda record: record["record_id"])


def record_schema():
    nullable_string = {"type": ["string", "null"]}
    return {"$schema": "https://json-schema.org/draft/2020-12/schema", "title": SCHEMA,
        "type": "object", "required": ["schema_version", "record_id", "source_url", "source_item_id", "public_author_id",
            "published_at", "updated_at", "collected_at", "client", "season", "mount", "test_server",
            "raw_text_hash", "parsed_ast_hash", "lineage_group_id", "macro_blocks", "page_selection", "manual_steps",
            "talents", "recipes", "scenario_metadata", "missing_fields", "parse_status", "unsupported_features",
            "native_parse_status", "replay_status", "source_usage_notes", "split"],
        "properties": {"schema_version": {"const": SCHEMA}, "record_id": {"type": "string"},
            "source_url": {"type": "string", "format": "uri"}, "source_item_id": {"type": "string"},
            **{key: nullable_string for key in ("public_author_id", "published_at", "updated_at", "collected_at", "client", "season", "mount", "parsed_ast_hash")},
            "raw_text_hash": {"type": "string", "pattern": "^[0-9a-f]{64}$"}, "test_server": {"type": ["boolean", "null"]},
            "lineage_group_id": {"type": "string"}, "macro_blocks": {"type": "array", "minItems": 1,
                "items": {"type": "object", "required": ["source_block_index", "name", "raw_text"], "properties": {
                    "source_block_index": {"type": "integer", "minimum": 0}, "name": nullable_string, "raw_text": {"type": "string"}}}},
            "page_selection": {"type": "object"}, "manual_steps": {"type": "array", "items": {"type": "string"}},
            "talents": {"type": "object"}, "recipes": {"type": ["array", "null"]}, "scenario_metadata": {"type": "object"},
            "missing_fields": {"type": "array", "items": {"type": "string"}}, "unsupported_features": {"type": "array"},
            "parse_status": {"enum": ["parsed_subset", "unsupported"]},
            "native_parse_status": {"enum": ["not_checked", "passed", "not_verified"]},
            "replay_status": {"enum": ["not_run", "trace_generated", "native_rejected_or_empty", "certified_project_constructed", "failed_project_constructed"]},
            "split": {"enum": ["review", "quarantine", "train", "validation", "test"]}, "source_usage_notes": {"type": "string"}}}


def split_config(records, previous, reviewed_sources=(), test_sources=()):
    versions = {}
    for record in records:
        versions.setdefault(record["source_item_id"], []).append({key: record.get(key) for key in (
            "record_id", "raw_text_hash", "source_context_hash", "season", "mount", "client", "derived_from_source_ids", "reference_b_marker_in_source")})
    versions = {identity: digest(canonical(sorted(items, key=lambda item: item["record_id"]))) for identity, items in versions.items()}
    reviewed = set(map(str, reviewed_sources)) | {identity for identity, version in previous.get("reviewed_source_versions", {}).items() if versions.get(identity) == version}
    tests = set(map(str, test_sources)) | set(previous.get("test_source_families", []))
    return reviewed, tests, {"reviewed_source_versions": {identity: versions[identity] for identity in sorted(reviewed) if identity in versions},
                             "test_source_families": sorted(tests), "review_scope": "Source provenance evidence under authorized corpus work; not a user permission gate"}


def save_manifest(out, records, requests, failures=(), exclusions=None, splits=None):
    previous = json.loads((out / "manifest.json").read_text(encoding="utf-8")) if (out / "manifest.json").exists() else {}
    if exclusions is None:
        exclusions = previous.get("quarantine_rules", {})
    if splits is None:
        splits = previous.get("source_review", {})
    write_json(out / "manifest.json", {"schema_version": SCHEMA, "route_evidence": OBSERVED_ROUTES,
        "records": records, "requests": requests, "failures": list(failures),
        "quarantine_rules": exclusions or {},
        "source_review": splits,
        "split_policy": "Source-family first; unreviewed B relations withheld. No snapshot random split.",
        "raw_scope": "local ignored output only", "dataset_hash": digest(canonical(records))})
    summary = {"source_count": len({record["source_item_id"] for record in records}), "macro_block_count": len(records),
               "family_count": len({record["lineage_group_id"] for record in records}),
               "parse_status": dict(Counter(record["parse_status"] for record in records)),
               "split": dict(Counter(record["split"] for record in records)), "failure_count": len(failures),
               "replay_status": dict(Counter(record["replay_status"] for record in records))}
    write_json(out / "summary.json", summary)
    return summary


def collect(args):
    out = output_path(args.out)
    previous = json.loads((out / "manifest.json").read_text(encoding="utf-8")) if (out / "manifest.json").exists() else {}
    client = PublicClient(out, args.delay, refresh=args.refresh)
    records, failures, seen, updated_sources = [], [], set(), set()
    for mount in args.mount or ["分山劲", "铁骨衣"]:
        for page in range(args.start_page, args.start_page + args.pages):
            params = {"type": "macro", "subtype": mount, "client": args.client, "page": page, "per": args.per, "order": "update", "sticky": 1}
            url = API + "/api/cms/posts?" + urlencode(params)
            try:
                listing = client.get(url, "list")
                context = {"url": url, "page": listing.get("page"), "per": listing.get("per"), "reported_total": listing.get("total"), "reported_pages": listing.get("pages")}
                for item in listing["list"]:
                    identity = item.get("ID")
                    if identity in seen:
                        continue
                    seen.add(identity)
                    try:
                        post = client.get(API + f"/api/cms/post/{int(identity)}", "detail")
                        updated_sources.add(str(post["ID"]))
                        records.extend(records_for_post(post, client.requests[-1], context))
                    except (ValueError, HTTPError, URLError) as error:
                        failures.append({"source_item_id": str(identity), "status": "uncollected", "reason": str(error)})
            except (ValueError, HTTPError, URLError) as error:
                failures.append({"mount": mount, "page": page, "status": "uncollected", "reason": str(error)})
                break
    for identity in args.source_id or []:
        if identity in seen:
            continue
        seen.add(identity)
        try:
            post = client.get(API + f"/api/cms/post/{int(identity)}", "detail")
            updated_sources.add(str(post["ID"]))
            records.extend(records_for_post(post, client.requests[-1], {"explicit_source_id": identity, "page": None, "per": None}))
        except (ValueError, HTTPError, URLError) as error:
            failures.append({"source_item_id": str(identity), "status": "uncollected", "reason": str(error)})
    records = merge_records(previous.get("records", []), records, updated_sources)
    exclusions = json.loads(Path(args.quarantine_manifest).read_text(encoding="utf-8")) if args.quarantine_manifest else previous.get("quarantine_rules", {})
    reviewed, tests, splits = split_config(records, previous.get("source_review", {}), args.reviewed_non_b_source or [], args.test_source_family or [])
    assign_families(records, exclusions, reviewed, tests)
    requests = {(item["url"], item.get("response_hash")): item for item in previous.get("requests", []) + client.requests}
    print(canonical(save_manifest(out, records, list(requests.values()), failures, exclusions, splits)))


def rebuild(args):
    out = output_path(args.out)
    manifest = json.loads((out / "manifest.json").read_text(encoding="utf-8"))
    records = manifest["records"]
    exclusions = json.loads(Path(args.quarantine_manifest).read_text(encoding="utf-8")) if args.quarantine_manifest else manifest.get("quarantine_rules", {})
    reviewed, tests, splits = split_config(records, manifest.get("source_review", {}), args.reviewed_non_b_source or [], args.test_source_family or [])
    assign_families(records, exclusions, reviewed, tests)
    print(canonical(save_manifest(out, records, manifest["requests"], manifest.get("failures", []), exclusions, splits)))


def schema_command(args):
    out = output_path(args.out)
    write_json(out / "record-schema.json", record_schema())
    print(canonical({"schema_version": SCHEMA, "schema_path": str(out / "record-schema.json")}))


def oracle_run(exe, request):
    # CLI mode does not start a worker/router, open HTTP or write userdata.
    process = subprocess.run([str(exe), "--exact-macro-oracle"], input=canonical(request) + "\n",
        cwd=ROOT / "backend", capture_output=True, text=True, encoding="utf-8", timeout=30,
        creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
    if process.returncode:
        raise RuntimeError("native oracle process failed")
    lines = [line[11:] for line in process.stdout.splitlines() if line.startswith("EXACT_JSON ")]
    if len(lines) != 1:
        raise RuntimeError("native oracle did not return exactly one result")
    return json.loads(lines[0])


def certified(result):
    return (result.get("status") == "ok" and result.get("comparison", {}).get("reproduced") is True
            and result.get("comparison", {}).get("completed_full_replay") is True and not result.get("truncated", True))


def inspect_compatibility(record, request):
    """Read versioned public project data; do not silently interpret unknown names."""
    versions = {"AnYingQianJi": ("2026_04_暗影千机", "v2026_04_AnYingQianJi", "暗影千机"),
                "ShanHaiYuanLiu": ("2025_10_山海源流", "v2025_10_ShanHaiYuanLiu", "山海源流")}
    if request["version"] not in versions:
        return {"status": "unsupported_version", "reasons": ["No automatic server inference or transfer to test-server data"]}
    data_version, script_version, season = versions[request["version"]]
    reasons = []
    if record["season"] != season or record["client"] != "std" or record.get("is_wujie") not in (None, 0, "0", False):
        reasons.append("source client/season does not explicitly match project scene")
    folder = ROOT / "backend/data" / data_version / record["mount"] / "skills"
    skills, skill_ids = set(), set()
    for path in folder.glob("*.toml"):
        if path.name.startswith("_"):
            continue
        data = tomllib.loads(path.read_text(encoding="utf-8-sig"))
        if isinstance(data.get("name"), str):
            skills.update((data["name"], data["name"].split("·")[0]))
        if isinstance(data.get("id"), int):
            skill_ids.add(data["id"])
    definitions = (ROOT / "backend/src/scripts" / script_version / "buffs/defs.rs").read_text(encoding="utf-8")
    registered = set(re.findall(r"buff_id:\s*([A-Z][A-Z_0-9]*)", definitions))
    registered.update(("BUFF_STANCE_SHIELD_GAME", "BUFF_STANCE_BLADE_GAME"))
    source = (ROOT / "backend/src/macro_eval.rs").read_text(encoding="utf-8")
    lookup = source.split("pub(crate) fn buff_name_to_id", 1)[1].split("// ──", 1)[0]
    buffs = set()
    for match in re.finditer(r'(?m)^\s*((?:"[^"\n]+"\s*(?:\|\s*)?)+)=>\s*Some\(([A-Z][A-Z_0-9]*)\)', lookup):
        if match[2] in registered:
            buffs.update(re.findall(r'"([^"\n]+)"', match[1]))
    def visit(node):
        if not node:
            return
        if "left" in node:
            visit(node["left"])
            visit(node["right"])
        elif node["kind"] in ("buff", "buff_stack", "nobuff", "tbuff", "tnobuff", "bufftime", "tbufftime"):
            if node["name"] not in buffs:
                reasons.append("unknown or unregistered versioned buff: " + node["name"])
        elif node["kind"] in ("skill_energy", "skill_notin_cd", "last_skill"):
            if node["name"] not in skills:
                reasons.append("unknown skill condition: " + node["name"])
        elif node["kind"] in ("skill", "noskill") and node["id"] not in skill_ids:
            reasons.append("unknown skill ID condition: " + str(node["id"]))
    for page in record["ast"]["pages"]:
        for line in page["lines"]:
            if line["skill"] not in skills:
                reasons.append("unknown command skill: " + line["skill"])
            visit(line["condition"])
    return {"status": "catalog_compatible" if not reasons else "incompatible_or_unknown", "reasons": sorted(set(reasons)),
            "scope": "Versioned skill data plus native buff-name mapping; release legality still belongs to the oracle"}


def certify(args):
    out = output_path(args.out)
    manifest = json.loads((out / "manifest.json").read_text(encoding="utf-8"))
    record = next(item for item in manifest["records"] if item["record_id"] == args.record)
    if record["parse_status"] != "parsed_subset" or record["reference_b_status"] == "quarantined":
        raise ValueError("unsupported or quarantined source cannot enter native corpus experiment")
    request = json.loads(Path(args.scene).read_text(encoding="utf-8-sig"))
    required = {"version", "mount", "simulation", "horizon", "time_tolerance_seconds", "acceptance"}
    if not required <= request.keys():
        raise ValueError("explicit project scene, horizon, tolerance and acceptance are required")
    if request["acceptance"] != "skills_and_time":
        raise ValueError("this corpus demonstration supports only skills_and_time")
    source_mount = {"分山劲": "FenShanJin", "铁骨衣": "TieGuYi"}.get(record["mount"])
    if source_mount != request["mount"]:
        raise ValueError("source/project mount mismatch")
    required_simulation = {"haste_level", "sequence", "talents", "recipes", "initial_rage", "network_delay",
        "attributes", "target", "equipment", "team_buffs", "formation", "pre_releases"}
    if not required_simulation <= request["simulation"].keys():
        raise ValueError("project scene is missing explicit simulation environment fields")
    compatibility = inspect_compatibility(record, request)
    record["project_compatibility"] = compatibility
    if compatibility["status"] != "catalog_compatible":
        write_json(out / "manifest.json", manifest)
        raise ValueError("source is incompatible or contains unknown names: " + "; ".join(compatibility["reasons"]))
    if any(page["stance_filter"] is not None for page in record["ast"]["pages"]) or len(record["ast"]["pages"]) > 1:
        raise ValueError("multi-page source requires its own explicit scenario; not this single-block demonstration")
    request.pop("archive_path", None)  # Native output may never escape the corpus store via scene fields.
    oracle_hash = digest(Path(args.exe).read_bytes())
    case_key = digest(canonical({"source": record["raw_text_hash"], "project_scene": request, "oracle": oracle_hash}))[:20]
    case = output_path(out / "cases" / record["record_id"].replace(":", "_") / case_key)
    case.mkdir(parents=True, exist_ok=True)
    text = record["macro_blocks"][0]["raw_text"]
    request.update(candidate=text, atoms=["rage<0"], compact_result=False, stop_on_divergence=False)
    frozen_contract = {key: copy.deepcopy(request[key]) for key in ("version", "mount", "horizon", "time_tolerance_seconds", "acceptance")}
    write_json(case / "project-scene-seed.json", request)
    generated = oracle_run(Path(args.exe).resolve(), request)
    write_json(case / "source-trace.json", generated)
    record["native_parse_status"] = "passed" if generated.get("status") in ("ok", "probe_budget", "semantic_mismatch") else "not_verified"
    actual = generated.get("actual") or []
    record["replay_status"] = "trace_generated" if actual else "native_rejected_or_empty"
    if not actual or generated.get("truncated") or not generated.get("comparison", {}).get("completed_full_replay"):
        reason = "source trace is empty, truncated or incomplete"
    elif any(not event.get("solidify") for event in actual) and any(event.get("channel_ticks") is not None for event in actual):
        reason = "channel trace without native frozen casts requires a separate construction path"
    else:
        target = copy.deepcopy(request)
        simulation = target["simulation"]
        simulation["sequence"] = [event["name"] if 90010 <= event["skill_id"] <= 90012 else event["name"].split("·")[0] for event in actual]
        for key in ("timing_offsets", "channel_ticks", "qijin_buffs"):
            simulation[key] = {}
        if all(event.get("solidify") for event in actual):
            simulation["solidified_casts"] = {str(index): event["solidify"] for index, event in enumerate(actual)}
            construction = "native concrete casts with relative event checkpoints"
        else:
            simulation.pop("solidified_casts", None)
            simulation["timing_offsets"] = {str(index): event["timing_offset"] for index, event in enumerate(actual) if event.get("timing_offset") is not None}
            construction = "native active sequence and exported relative offsets; hypothesis until independent full replay"
        teacher_target = {key: value for key, value in target.items() if key not in ("candidate", "atoms", "compact_result", "stop_on_divergence")}
        write_json(case / "target-contract.json", teacher_target)
        result = oracle_run(Path(args.exe).resolve(), target)
        write_json(case / "certification.json", result)
        if any(target[key] != frozen_contract[key] for key in frozen_contract):
            raise RuntimeError("constructed source contract changed during certification")
        record["replay_status"] = "certified_project_constructed" if certified(result) else "failed_project_constructed"
        record["certification"] = {"scenario_origin": "project_constructed_test_scene", "author_configuration_reconstructed": False,
            "contract_hash": digest(canonical({key: target[key] for key in required})),
            "source_record_id": record["record_id"], "raw_text_hash": record["raw_text_hash"], "oracle_executable_hash": oracle_hash,
            "contract": frozen_contract, "active_count": len(actual), "certified": certified(result),
            "comparison": result.get("comparison"), "evidence_directory": str(case.relative_to(out)),
            "target_construction": construction, "source_body_in_target": False}
        reason = None
    if reason:
        record["certification"] = {"certified": False, "scenario_origin": "project_constructed_test_scene", "limitation": reason}
    save_manifest(out, manifest["records"], manifest["requests"], manifest.get("failures", []))
    print(canonical({"record_id": record["record_id"], "replay_status": record["replay_status"], "certification": record["certification"]}))


def main(argv=None):
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8")
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    for command in ("collect", "rebuild", "certify", "schema"):
        item = sub.add_parser(command)
        item.add_argument("--out", type=Path, default=STORE)
        if command in ("collect", "rebuild"):
            item.add_argument("--quarantine-manifest", type=Path)
            item.add_argument("--reviewed-non-b-source", action="append", help="Source ID whose provenance evidence was reviewed under the corpus task; not a permission request")
            item.add_argument("--test-source-family", action="append", help="Reserve the reviewed whole family containing this source ID for final test")
        if command == "collect":
            item.add_argument("--mount", action="append", choices=["分山劲", "铁骨衣"])
            item.add_argument("--client", choices=["std", "origin"], default="std")
            item.add_argument("--pages", type=int, default=1)
            item.add_argument("--start-page", type=int, default=1)
            item.add_argument("--per", type=int, default=4)
            item.add_argument("--delay", type=float, default=0.5)
            item.add_argument("--source-id", type=int, action="append", help="Explicit publicly observed source ID in addition to list pages")
            item.add_argument("--refresh", action="store_true", help="Refresh public GET cache; append original source versions instead of replacing them")
        if command == "certify":
            item.add_argument("--record", required=True)
            item.add_argument("--scene", type=Path, required=True)
            item.add_argument("--exe", type=Path, default=ROOT / "backend/target/release/jx3-combat-sim.exe")
    args = parser.parse_args(argv)
    if args.command == "collect" and (args.pages < 1 or args.start_page < 1 or not 1 <= args.per <= 50):
        parser.error("pages/start-page must be positive; per must be 1..50")
    {"collect": collect, "rebuild": rebuild, "certify": certify, "schema": schema_command}[args.command](args)


if __name__ == "__main__":
    main()
