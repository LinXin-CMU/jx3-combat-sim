"""
随机 macro fuzz：随机生成 N 个 macro，每个跑两次（Full + Lite），
断言 (fingerprint, dps, total_damage) 三项都 bit-equal。

主要 catch 的 corner case：
  - bufftime 阈值刚好等于 buff duration / tick interval
  - 嵌套 and/or 条件触发解析边界
  - 罕见 buff 组合下的 next_decision_time 漂移
  - 罕见 stance switch / channel interrupt 时序

用法（先启 backend）：
  python tests/fuzz_macros.py [--count 100] [--seed 42]

退出码：0 = 全部通过；1 = 有 case 漂移
"""
import argparse
import json
import random
import sys
import urllib.request

BACKEND = "http://localhost:3005"

# 已知"安全"的技能名 + 触发条件池
SHIELD_SKILLS = ["盾击", "盾压", "盾猛", "盾飞", "盾舞", "血怒", "业火麟光", "阵云结晦", "撼地"]
BLADE_SKILLS = ["斩刀", "绝刀", "劫刀", "闪刀", "盾回"]
KNOWN_BUFFS = ["嗜血", "狂绝", "血怒", "血怒·惊涌", "天下宏愿", "援戈"]
RAGE_THRESHOLDS = [10, 25, 49, 50, 64, 75]


def gen_condition(rng):
    """生成单个条件"""
    kind = rng.choice(["rage", "buff", "nobuff", "bufftime", "rage", "rage"])  # rage 偏多
    if kind == "rage":
        op = rng.choice([">", "<", ">=", "<="])
        val = rng.choice(RAGE_THRESHOLDS)
        return f"rage{op}{val}"
    if kind == "buff":
        return f"buff:{rng.choice(KNOWN_BUFFS)}"
    if kind == "nobuff":
        return f"nobuff:{rng.choice(KNOWN_BUFFS)}"
    if kind == "bufftime":
        op = rng.choice([">", "<"])
        val = round(rng.uniform(1, 12), 1)
        return f"bufftime:{rng.choice(KNOWN_BUFFS)}{op}{val}"
    return "rage>0"


def gen_combined_condition(rng):
    """1-3 个条件用 & 或 | 组合"""
    n = rng.choice([1, 1, 2, 2, 3])  # 偏少
    parts = [gen_condition(rng) for _ in range(n)]
    if n == 1:
        return parts[0]
    op = rng.choice(["&", "|"])
    return op.join(parts)


def gen_line(rng, page):
    skills = SHIELD_SKILLS if page == "shield" else BLADE_SKILLS
    skill = rng.choice(skills)
    has_cond = rng.random() < 0.6
    cmd = rng.choice(["/cast", "/cast", "/cast", "/fcast"])  # /cast 偏多
    if has_cond:
        return f"{cmd} [{gen_combined_condition(rng)}] {skill}"
    return f"{cmd} {skill}"


def gen_macro(rng):
    """生成一个完整 stance-mode macro"""
    n_shield = rng.randint(2, 6)
    n_blade = rng.randint(2, 5)
    lines = ["#page shield"]
    for _ in range(n_shield):
        lines.append(gen_line(rng, "shield"))
    lines.append("#page blade")
    for _ in range(n_blade):
        lines.append(gen_line(rng, "blade"))
    return "\n".join(lines)


def call_simulate(payload):
    data = json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(
        f"{BACKEND}/api/simulate",
        data=data,
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.loads(r.read())


def make_payload(macro_text, lite=False):
    return {
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
        "macro_duration": 60,   # 短一点，加快 fuzz
        "lite": lite,
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--count", type=int, default=100)
    parser.add_argument("--seed", type=int, default=None)
    args = parser.parse_args()

    seed = args.seed if args.seed is not None else random.randint(0, 1_000_000)
    rng = random.Random(seed)
    print(f"seed = {seed}, count = {args.count}\n")

    failures = []
    for i in range(args.count):
        macro = gen_macro(rng)
        try:
            full = call_simulate(make_payload(macro, lite=False))
            lite = call_simulate(make_payload(macro, lite=True))
            full2 = call_simulate(make_payload(macro, lite=False))  # 同请求 2 次确认 determinism
        except Exception as e:
            print(f"  [{i+1:3d}] error: {e}")
            failures.append((i, "exception", str(e), macro))
            continue

        # 1. determinism within Full
        if full["fingerprint"] != full2["fingerprint"]:
            failures.append((i, "non-deterministic Full",
                f"fp1={full['fingerprint']:016x} fp2={full2['fingerprint']:016x}", macro))
            print(f"  [{i+1:3d}] FAIL non-deterministic Full")
            continue

        # 2. Lite ≡ Full
        if (full["fingerprint"] != lite["fingerprint"]
                or full["dps"] != lite["dps"]
                or full["total_damage"] != lite["total_damage"]):
            failures.append((i, "Lite ≠ Full",
                f"Full fp={full['fingerprint']:016x} dps={full['dps']} | Lite fp={lite['fingerprint']:016x} dps={lite['dps']}",
                macro))
            print(f"  [{i+1:3d}] FAIL Lite != Full")
            continue

        if (i + 1) % 10 == 0:
            print(f"  [{i+1:3d}/{args.count}] OK")

    print()
    if not failures:
        print(f"[OK] {args.count} 个随机 macro 全部通过 determinism + Lite/Full 验证。")
        sys.exit(0)

    print(f"[FAIL] {len(failures)} / {args.count} 个 case 失败：\n")
    for i, kind, detail, macro in failures[:5]:  # 只打前 5 个
        print(f"  case #{i}: {kind}")
        print(f"    {detail}")
        print(f"    macro:\n      {macro.replace(chr(10), chr(10) + '      ')}")
        print()
    if len(failures) > 5:
        print(f"  ...（还有 {len(failures)-5} 个 case 未列出）")
    print(f"\n复现：python tests/fuzz_macros.py --seed {seed} --count {args.count}")
    sys.exit(1)


if __name__ == "__main__":
    main()
