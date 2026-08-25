"""
回归验证 + golden fingerprint 校验。

三层断言：
  1. **Determinism**：同请求重复 3 次 fingerprint 必须一致（同一进程内）
  2. **Lite ≡ Full**：lite=true 与 lite=false 的 (fingerprint, dps, total_damage) 必须 bit-equal
  3. **Golden**：与 backend/tests/golden/*.json 里记录的"已知好"fingerprint 比对，捕获 silent 漂移

用法（先 cargo run --release 启动 backend）：
    python tests/diff_baseline.py            # 校验
    python tests/diff_baseline.py --update   # 故意改了行为？把当前 fingerprint 写回 golden

退出码：0 = 全部通过，1 = 有 case 失败
"""
import json
import os
import sys
import urllib.request

BACKEND = "http://127.0.0.1:3005"
GOLDEN_DIR = os.path.join(os.path.dirname(os.path.abspath(__file__)), "golden")


def call_simulate(payload):
    data = json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(
        f"{BACKEND}/api/simulate",
        data=data,
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.loads(r.read())


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


# 每个 case：(显示名, golden 文件名 stem, macro_text, overrides)
# overrides: dict 覆盖默认 payload 字段（如 talents、boss_attack_interval）
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
    ("简单循环", "simple_loop",
     "/cast 盾击\n/cast 盾压", None),
    ("纯刀系", "blade_only",
     "#page blade\n/cast 斩刀\n/cast 闪刀", None),
    # 坚铁+寒甲奇穴 + Boss 受击间隔 → 触发 expectation 子系统，覆盖
    # sync_expectation_buffs 的 buff_generation bump 路径（避免回归潜伏 bug）
    ("坚铁寒甲(boss=2s)", "jueyun_jiantie_hanjia",
     JUEYUN_MACRO,
     {
         "talents": [13090, 36058, 34912, 36205, 21281, 30769, 22897, 13138, 13134],
         "boss_attack_interval": 2.0,
         "hanjia_expectation": True,
     }),
]


def fingerprint_consistent(name, payload, runs=3):
    """同请求 N 次，fingerprint/dps 必须一致"""
    fps = []
    for _ in range(runs):
        r = call_simulate(payload)
        fps.append((r["fingerprint"], r["dps"], r["total_damage"], r["skill_count"]))
    first = fps[0]
    ok = all(f == first for f in fps)
    if ok:
        print(f"  [OK] {name:25s} fp={first[0]:016x}  dps={first[1]:.0f}  events={first[3]}")
        return True, first
    else:
        print(f"  [FAIL] {name:25s} 同请求重复结果不一致：")
        for i, f in enumerate(fps):
            print(f"      run {i}: fp={f[0]:016x}  dps={f[1]:.0f}  events={f[3]}")
        return False, first


def lite_full_match(name, payload):
    """Lite 与 Full 必须 (fp, dps, total) bit-equal"""
    full = call_simulate({**payload, "lite": False})
    lite = call_simulate({**payload, "lite": True})
    if (full["fingerprint"] == lite["fingerprint"]
            and full["dps"] == lite["dps"]
            and full["total_damage"] == lite["total_damage"]):
        print(f"  [OK] {name:25s} Lite == Full  fp={full['fingerprint']:016x}")
        return True
    print(f"  [FAIL] {name:25s} Lite/Full 不一致：")
    print(f"      Full: fp={full['fingerprint']:016x} dps={full['dps']} total={full['total_damage']}")
    print(f"      Lite: fp={lite['fingerprint']:016x} dps={lite['dps']} total={lite['total_damage']}")
    return False


def load_golden(stem):
    path = os.path.join(GOLDEN_DIR, f"{stem}.json")
    if not os.path.exists(path):
        return None
    with open(path, "r", encoding="utf-8") as f:
        return json.load(f)


def write_golden(stem, data):
    path = os.path.join(GOLDEN_DIR, f"{stem}.json")
    with open(path, "r", encoding="utf-8") as f:
        existing = json.load(f)
    # 只更新数值字段，保留 name/notes/captured_after
    existing.update({
        "fingerprint": f"{data['fingerprint']:016x}",
        "dps": data["dps"],
        "total_damage": data.get("total_damage", 0.0),
        "skill_count": data["skill_count"],
        "fight_time": data.get("fight_time", 0.0),
    })
    with open(path, "w", encoding="utf-8") as f:
        json.dump(existing, f, ensure_ascii=False, indent=2)
    print(f"  [UPDATED] {stem}.json: fp={existing['fingerprint']}")


def golden_check(name, stem, payload, update_mode=False):
    """与 golden 文件比对"""
    r = call_simulate(payload)
    if update_mode:
        write_golden(stem, r)
        return True
    golden = load_golden(stem)
    if golden is None:
        print(f"  [FAIL] {name:25s} golden 文件 {stem}.json 不存在")
        return False
    expected_fp = golden["fingerprint"]
    actual_fp = f"{r['fingerprint']:016x}"
    if expected_fp == actual_fp:
        print(f"  [OK] {name:25s} 匹配 golden  fp={actual_fp}")
        return True
    print(f"  [FAIL] {name:25s} fingerprint 漂了：")
    print(f"      golden: fp={expected_fp}  dps={golden['dps']}  events={golden['skill_count']}")
    print(f"      actual: fp={actual_fp}  dps={r['dps']}  events={r['skill_count']}")
    print(f"      → 如果改动是有意的，跑 `python tests/diff_baseline.py --update` 重写 golden")
    return False


def main():
    update_mode = "--update" in sys.argv

    if update_mode:
        print("\n=== UPDATE MODE: 把当前 fingerprint 写入 golden ===")
        for name, stem, macro, overrides in CASES:
            golden_check(name, stem, macro_request(macro, overrides=overrides), update_mode=True)
        print("\ngolden 文件已更新。请 git diff 检查后再 commit。")
        sys.exit(0)

    all_ok = True
    print("\n=== Determinism: 同请求 3 次 fingerprint 必须一致 ===")
    for name, _, macro, overrides in CASES:
        ok, _ = fingerprint_consistent(name, macro_request(macro, overrides=overrides))
        if not ok:
            all_ok = False

    print("\n=== Lite ≡ Full: 模式不能影响伤害 ===")
    for name, _, macro, overrides in CASES:
        if not lite_full_match(name, macro_request(macro, overrides=overrides)):
            all_ok = False

    print("\n=== Golden: 与已知好 fingerprint 比对（catch silent 漂移）===")
    for name, stem, macro, overrides in CASES:
        if not golden_check(name, stem, macro_request(macro, overrides=overrides)):
            all_ok = False

    print()
    if all_ok:
        print("[OK] 全部通过：行为与 golden 完全一致。")
        sys.exit(0)
    else:
        print("[FAIL] 有 case 失败")
        sys.exit(1)


if __name__ == "__main__":
    main()
