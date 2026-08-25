#!/bin/bash
# 一键回归测试：build + start backend + run diff_baseline.py + kill backend
# 用法（在仓库根或 backend/ 都行）：
#   bash backend/tests/run_regression.sh
#
# 退出码：0 = 全部通过，非 0 = 失败

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BACKEND_DIR="$(dirname "$SCRIPT_DIR")"
REPO_ROOT="$(dirname "$BACKEND_DIR")"

# 0. 静态守卫（先跑，便宜且零依赖）
echo "[0/4] 静态守卫：检查脚本侧无直写状态字段 ..."
bash "$SCRIPT_DIR/check_no_direct_writes.sh"

# 1. 编译 release + 跑 unit / property tests
echo "[1/4] cargo build --release + cargo test player_setter_tests ..."
( cd "$BACKEND_DIR" && cargo build --release ) 2>&1 | tail -3
( cd "$BACKEND_DIR" && cargo test --release player_setter_tests ) 2>&1 | tail -5

# 2. 启动 backend（后台），等就绪
echo "[2/4] 启动 backend ..."
EXE="$BACKEND_DIR/target/release/jx3-combat-sim.exe"
if [ ! -f "$EXE" ]; then EXE="$BACKEND_DIR/target/release/jx3-combat-sim"; fi
"$EXE" > /tmp/jx3-regression.log 2>&1 &
BACKEND_PID=$!
trap "kill $BACKEND_PID 2>/dev/null || true" EXIT
sleep 3
# 健康检查
if ! curl -s --max-time 5 -X POST http://localhost:3005/api/calculate \
     -H "Content-Type: application/json" \
     -d '{"li_dao":1,"shen_fa":1,"vitality":1,"base_attack":1,"weapon_damage":1,"surplus_value":1,"crit_level":1,"crit_effect_level":1,"overcome_level":1,"strain_level":1,"haste_level":1,"parry_value":0,"parry_level":0}' \
     -o /dev/null; then
  echo "[FAIL] backend 未就绪。日志："
  tail -20 /tmp/jx3-regression.log
  exit 1
fi

# 3. 跑 diff_baseline.py
echo "[3/4] 跑 fingerprint diff ..."
PYTHONIOENCODING=utf-8 python "$SCRIPT_DIR/diff_baseline.py"
DIFF_EXIT=$?

# 4. 可选：随机 fuzz（默认 30，慢；--fuzz 30/100/...）
FUZZ_COUNT=0
for arg in "$@"; do
  case "$arg" in
    --fuzz=*) FUZZ_COUNT="${arg#*=}" ;;
    --fuzz)   FUZZ_COUNT=30 ;;
  esac
done
if [ "$FUZZ_COUNT" -gt 0 ]; then
  echo "[4/4] 跑随机 fuzz ($FUZZ_COUNT 个 macro) ..."
  PYTHONIOENCODING=utf-8 python "$SCRIPT_DIR/fuzz_macros.py" --count "$FUZZ_COUNT"
  FUZZ_EXIT=$?
else
  echo "[4/4] 跳过 fuzz（用 --fuzz 或 --fuzz=N 启用）"
  FUZZ_EXIT=0
fi

if [ "$DIFF_EXIT" -ne 0 ] || [ "$FUZZ_EXIT" -ne 0 ]; then
  exit 1
fi
exit 0
