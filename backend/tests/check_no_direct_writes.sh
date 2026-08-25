#!/bin/bash
# 静态守卫：禁止脚本侧绕过 Player setter 直接写状态字段。
#
# 任何这些模式如果在 src/scripts/**/*.rs 或 src/macro_eval.rs 出现，CI 立即红：
#   player.rage = ...        ← 应走 player.set_rage(v) / player.add_rage(d)
#   player.block_value = ... ← player.set_block_value() / player.add_block_value()
#   inst.stacks = N          ← player.set_buff_stacks(buff_id, N)
#   player.active_cds.insert ← player.add_protect_cd(...)
#   player.active_buffs.push ← player.add_buff(...)
#
# 只在 src/scripts/ 和 src/macro_eval.rs 检查（main.rs 是 Player 实现者，允许直写）。
#
# 用法: bash backend/tests/check_no_direct_writes.sh
# 退出码: 0 = 干净, 1 = 发现违规

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BACKEND_DIR="$(dirname "$SCRIPT_DIR")"
SRC_DIR="$BACKEND_DIR/src"

violations=0

check() {
    local pattern="$1"
    local desc="$2"
    local hint="$3"
    # 只搜 scripts/ 和 macro_eval.rs；main.rs 允许直写（Player 实现）
    local found
    found=$(grep -rn -E "$pattern" "$SRC_DIR/scripts" "$SRC_DIR/macro_eval.rs" 2>/dev/null \
            | grep -v "//.*$pattern" \
            || true)
    if [ -n "$found" ]; then
        echo "[FAIL] $desc"
        echo "       $hint"
        echo "$found" | sed 's/^/       /'
        echo ""
        violations=$((violations + 1))
    fi
}

# 1. rage 直写
check 'player\.rage\s*=\s*[^=]' \
      "脚本侧直写 player.rage" \
      "→ 改用 player.set_rage(v) 或 player.add_rage(delta)"

# 2. block_value 直写
check 'player\.block_value\s*=\s*[^=]' \
      "脚本侧直写 player.block_value" \
      "→ 改用 player.set_block_value(v) 或 player.add_block_value(delta)"

# 3. inst.stacks 直写（buff 实例层数）
check 'inst\.stacks\s*=\s*[^=]' \
      "脚本侧直写 inst.stacks" \
      "→ 改用 player.set_buff_stacks(buff_id, n)"

# 4. active_cds.insert（应走 add_protect_cd / set_cd / 等 helper）
check 'player\.active_cds\.insert' \
      "脚本侧直写 active_cds" \
      "→ 改用 player.add_protect_cd(\"protect_xxx\", expires_at)"

# 5. active_buffs / target_buffs 直写
check 'player\.(active|target)_buffs\.(push|retain|clear|swap_remove)' \
      "脚本侧直接修改 active_buffs/target_buffs" \
      "→ 走 add_buff / remove_buff / add_target_buff / remove_target_buff"

# 6. charges 直写
check 'player\.charges\.insert' \
      "脚本侧直写 charges" \
      "→ 走 cast_skill / consume_charge / reduce_charge_cd（已有 helper）"

# 7. channel_end 直写
check 'player\.channel_end\s*=' \
      "脚本侧直写 channel_end" \
      "→ 走 cast_skill 内部设置；脚本不应该直改"

if [ $violations -eq 0 ]; then
    echo "[OK] 脚本侧无绕过 setter 的直写。"
    exit 0
fi

echo "─────────────────────────────────────────"
echo "共发现 $violations 个违规模式。每个直写都可能跳过 bump_decision_gen()，"
echo "导致 macro_eval 缓存与实际状态漂移（silent bug）。"
echo "请按提示替换为对应的 Player setter（已经定义好），保留 invariant。"
exit 1
