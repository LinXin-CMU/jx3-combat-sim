"""版本化确定性、Lite/Full 与 Golden v2 回归验证。

Golden v2 绑定游戏版本、心法、场景哈希、数据/脚本哈希和生成提交，
避免把技改后的合理变化误判成代码回归。

用法（先启动 release 后端）：
    python tests/diff_baseline.py
    python tests/diff_baseline.py --version 2026_04
    python tests/diff_baseline.py --version all --update

更新模式默认拒绝脏工作区。确认行为正确并提交实现后再生成 Golden。
"""

import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import urllib.request


BACKEND = "http://127.0.0.1:3005"
TEST_DIR = Path(__file__).resolve().parent
BACKEND_ROOT = TEST_DIR.parent
REPO_ROOT = BACKEND_ROOT.parent
GOLDEN_ROOT = TEST_DIR / "golden"
SCHEMA_VERSION = 2

ENVIRONMENTS = {
    "2025_10": {
        "slug": "2025_10_shanhai",
        "game_version": "ShanHaiYuanLiu",
        "version_dir": "2025_10_山海源流",
        "script_dir": "v2025_10_ShanHaiYuanLiu",
        "mount": "FenShanJin",
        "label": "山海源流（2025.10）· 分山劲",
    },
    "2026_04": {
        "slug": "2026_04_anying",
        "game_version": "AnYingQianJi",
        "version_dir": "2026_04_暗影千机",
        "script_dir": "v2026_04_AnYingQianJi",
        "mount": "FenShanJin",
        "label": "暗影千机（2026.04）· 分山劲",
    },
}


def request_json(method, path, payload=None):
    data = None
    headers = {}
    if payload is not None:
        data = json.dumps(payload, ensure_ascii=False).encode("utf-8")
        headers["Content-Type"] = "application/json; charset=utf-8"
    request = urllib.request.Request(
        f"{BACKEND}{path}", data=data, headers=headers, method=method
    )
    with urllib.request.urlopen(request, timeout=60) as response:
        return json.loads(response.read())


def call_simulate(payload):
    return request_json("POST", "/api/simulate", payload)


def current_environment():
    return request_json("GET", "/api/mounts/current")


def switch_environment(game_version, mount):
    result = request_json("POST", "/api/mounts/switch", {
        "version": game_version,
        "mount": mount,
        "persist": False,
    })
    if not result.get("ok"):
        raise RuntimeError(f"版本切换失败: {result.get('error', 'unknown')}")
    actual = current_environment()
    if actual.get("version") != game_version or actual.get("mount") != mount:
        raise RuntimeError(
            f"版本状态不一致: expected={game_version}/{mount}, "
            f"actual={actual.get('version')}/{actual.get('mount')}"
        )


def macro_request(macro_text, lite=False, overrides=None):
    payload = {
        "haste_level": 42087,
        "sequence": ["__macro__"] * 1220,
        "talents": [13090, 36058, 34912, 36205, 21281, 30769, 22897, 14838, 37239],
        "recipes": [
            1003, 1002, 1006, 1007, 2003, 2002, 2006, 2001,
            3002, 3001, 3004, 3005, 4002, 4001, 4007, 4008,
            5003, 5002, 5006, 5007, 6004, 6005, 6006, 6007,
            7004, 7005, 7001, 7002, 8008, 8001, 8002, 8004,
            9002, 9001, 9004, 9005,
        ],
        "network_delay": 0,
        "attributes": {
            "li_dao": 5027, "shen_fa": 2836, "vitality": 169932,
            "base_attack": 38466, "weapon_damage": 10986,
            "surplus_value": 28330, "crit_level": 54841,
            "crit_effect_level": 0, "overcome_level": 29480,
            "strain_level": 66031, "haste_level": 42087,
            "parry_value": 0, "parry_level": 0,
        },
        "target": {"level": 134, "defense_bonus": 0},
        "experimental": False,
        "initial_rage": 50,
        "macro_text": macro_text,
        "macro_duration": 300,
        "lite": lite,
    }
    if overrides:
        payload.update(overrides)
    return payload


JUEYUN_MACRO = (
    "#page shield\n"
    "/cast [bufftime:嗜血>8&buff:血怒·惊涌] 阵云结晦\n"
    "/cast 盾击\n"
    "/cast 盾猛\n"
    "/cast [rage>49] 业火麟光\n"
    "/cast 盾压\n"
    "/cast [rage>64&bufftime:嗜血<6|nobuff:嗜血] 盾飞\n"
    "#page blade\n"
    "/cast 斩刀\n"
    "/fcast [bufftime:狂绝<4.7&nobuff:血怒·惊涌] 血怒\n"
    "/cast [buff:天下宏愿|buff:狂绝|bufftime:嗜血>7] 绝刀\n"
    "/cast [rage<50&bufftime:嗜血<10.8] 盾回"
)

CASES = [
    ("绝云宏 standard", "jueyun_300s", JUEYUN_MACRO, None),
    ("简单循环", "simple_loop", "/cast 盾击\n/cast 盾压", None),
    ("纯刀系", "blade_only", "#page blade\n/cast 斩刀\n/cast 闪刀", None),
    ("坚铁寒甲(boss=2s)", "jueyun_jiantie_hanjia", JUEYUN_MACRO, {
        "talents": [13090, 36058, 34912, 36205, 21281, 30769, 22897, 13138, 13134],
        "boss_attack_interval": 2.0,
        "hanjia_expectation": True,
    }),
]


def canonical_sha256(value):
    encoded = json.dumps(
        value, ensure_ascii=False, sort_keys=True, separators=(",", ":")
    ).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def tree_sha256(roots):
    digest = hashlib.sha256()
    files = []
    for root in roots:
        if root.exists():
            files.extend(path for path in root.rglob("*") if path.is_file())
    key = lambda path: path.relative_to(BACKEND_ROOT).as_posix()
    for path in sorted(files, key=key):
        digest.update(key(path).encode("utf-8"))
        digest.update(b"\0")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    return digest.hexdigest()


def git_output(*args):
    try:
        return subprocess.check_output(
            ["git", *args], cwd=REPO_ROOT, text=True, stderr=subprocess.DEVNULL
        ).strip()
    except (OSError, subprocess.CalledProcessError):
        return "unknown"


def environment_metadata(environment):
    return {
        "schema_version": SCHEMA_VERSION,
        "game_version": environment["game_version"],
        "version_dir": environment["version_dir"],
        "mount": environment["mount"],
        "data_sha256": tree_sha256([
            BACKEND_ROOT / "data" / environment["version_dir"],
            BACKEND_ROOT / "src" / "scripts" / environment["script_dir"],
        ]),
        "engine_commit": git_output("rev-parse", "HEAD"),
    }


def response_values(response):
    return {
        "fingerprint": f"{response['fingerprint']:016x}",
        "dps": response["dps"],
        "total_damage": response["total_damage"],
        "skill_count": response["skill_count"],
        "fight_time": response.get("fight_time", 0.0),
    }


def fingerprint_consistent(name, payload, runs=3):
    values = [response_values(call_simulate(payload)) for _ in range(runs)]
    first = values[0]
    ok = all(value == first for value in values)
    if ok:
        print(
            f"  [OK] {name:25s} fp={first['fingerprint']}  "
            f"dps={first['dps']:.3f}  events={first['skill_count']}"
        )
    else:
        print(f"  [FAIL] {name:25s} 同请求重复结果不一致：")
        for index, value in enumerate(values):
            print(f"      run {index}: {value}")
    return ok


def lite_full_match(name, payload):
    full = response_values(call_simulate({**payload, "lite": False}))
    lite = response_values(call_simulate({**payload, "lite": True}))
    if full == lite:
        print(f"  [OK] {name:25s} Lite == Full  fp={full['fingerprint']}")
        return True
    print(f"  [FAIL] {name:25s} Lite/Full 不一致：")
    print(f"      Full: {full}")
    print(f"      Lite: {lite}")
    return False


def golden_path(environment, stem):
    return GOLDEN_ROOT / environment["slug"] / f"{stem}.json"


def load_golden(environment, stem):
    path = golden_path(environment, stem)
    if not path.exists():
        return None
    return json.loads(path.read_text(encoding="utf-8"))


def write_golden(environment, name, stem, payload, response, metadata):
    path = golden_path(environment, stem)
    path.parent.mkdir(parents=True, exist_ok=True)
    existing = load_golden(environment, stem) or {}
    document = {
        "schema_version": SCHEMA_VERSION,
        "scenario_id": stem,
        "name": name,
        "game_version": environment["game_version"],
        "version_dir": environment["version_dir"],
        "mount": environment["mount"],
        "scenario_sha256": canonical_sha256({**payload, "lite": False}),
        "data_sha256": metadata["data_sha256"],
        "engine_commit": metadata["engine_commit"],
        "captured_at": datetime.now(timezone.utc).isoformat(),
        **response_values(response),
    }
    if existing.get("notes"):
        document["notes"] = existing["notes"]
    path.write_text(
        json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    print(f"  [UPDATED] {path.relative_to(TEST_DIR)}  fp={document['fingerprint']}")


def golden_check(environment, name, stem, payload, metadata):
    actual = response_values(call_simulate(payload))
    path = golden_path(environment, stem)
    golden = load_golden(environment, stem)
    if golden is None:
        print(f"  [MISSING] {name:25s} {path.relative_to(TEST_DIR)}")
        print(f"            actual={actual}")
        return False

    expected_metadata = {
        "schema_version": SCHEMA_VERSION,
        "game_version": environment["game_version"],
        "version_dir": environment["version_dir"],
        "mount": environment["mount"],
        "scenario_sha256": canonical_sha256({**payload, "lite": False}),
        "data_sha256": metadata["data_sha256"],
    }
    metadata_diff = {
        key: {"expected": value, "golden": golden.get(key)}
        for key, value in expected_metadata.items()
        if golden.get(key) != value
    }
    value_diff = {
        key: {"golden": golden.get(key), "actual": value}
        for key, value in actual.items()
        if golden.get(key) != value
    }
    if not metadata_diff and not value_diff:
        print(
            f"  [OK] {name:25s} fp={actual['fingerprint']}  "
            f"dps={actual['dps']:.3f}"
        )
        return True

    print(f"  [FAIL] {name:25s} {path.relative_to(TEST_DIR)}")
    if metadata_diff:
        print(f"      metadata drift: {metadata_diff}")
    if value_diff:
        print(f"      value drift: {value_diff}")
    return False


def assert_update_safe(allow_dirty):
    dirty = git_output("status", "--porcelain", "--untracked-files=no")
    if dirty not in ("", "unknown") and not allow_dirty:
        print("[REFUSED] 工作区有未提交修改；先提交实现，再生成可追溯 Golden。")
        print("          如仅做临时实验，可显式传 --allow-dirty。")
        sys.exit(2)


def selected_environments(selection):
    if selection == "all":
        return [(key, ENVIRONMENTS[key]) for key in ("2025_10", "2026_04")]
    return [(selection, ENVIRONMENTS[selection])]


def parse_args():
    parser = argparse.ArgumentParser()
    parser.add_argument("--version", choices=["all", *ENVIRONMENTS], default="all")
    parser.add_argument("--update", action="store_true")
    parser.add_argument("--allow-dirty", action="store_true")
    parser.add_argument("--backend", default=BACKEND)
    return parser.parse_args()


def main():
    global BACKEND
    args = parse_args()
    BACKEND = args.backend.rstrip("/")
    if args.update:
        assert_update_safe(args.allow_dirty)

    original = current_environment()
    all_ok = True
    try:
        for _, environment in selected_environments(args.version):
            print(f"\n{'=' * 72}\n{environment['label']}\n{'=' * 72}")
            switch_environment(environment["game_version"], environment["mount"])
            metadata = environment_metadata(environment)
            print(f"data_sha256={metadata['data_sha256']}")

            if args.update:
                print("\n=== UPDATE MODE: 写入版本化 Golden ===")
                for name, stem, macro, overrides in CASES:
                    payload = macro_request(macro, overrides=overrides)
                    write_golden(
                        environment, name, stem, payload,
                        call_simulate(payload), metadata,
                    )
                continue

            print("\n=== Determinism: 完整结果重复 3 次必须一致 ===")
            for name, _, macro, overrides in CASES:
                all_ok &= fingerprint_consistent(
                    name, macro_request(macro, overrides=overrides)
                )

            print("\n=== Lite ≡ Full: 完整数值结果必须一致 ===")
            for name, _, macro, overrides in CASES:
                all_ok &= lite_full_match(
                    name, macro_request(macro, overrides=overrides)
                )

            print("\n=== Golden v2: 元数据 + fingerprint + 数值 ===")
            for name, stem, macro, overrides in CASES:
                all_ok &= golden_check(
                    environment, name, stem,
                    macro_request(macro, overrides=overrides), metadata,
                )
    finally:
        switch_environment(original["version"], original["mount"])

    print()
    if args.update:
        print("[UPDATED] Golden 已生成；必须审阅 git diff 后再提交。")
        return 0
    if all_ok:
        print("[OK] 所有版本的确定性、Lite/Full 与 Golden v2 均通过。")
        return 0
    print("[FAIL] 有检查失败；禁止静默更新 Golden。")
    return 1


if __name__ == "__main__":
    sys.exit(main())
