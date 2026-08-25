"""不一致分析：加载训练好的 ckpt，逐决策点对比 RL 决策 vs 宏决策。

两种调用方式：
1. CLI:
     python analyze.py --ckpt ... --macro-text macro.txt --duration 300
2. spec-json（前端 "RL 分析" tab 用）：
     python analyze.py --spec-json /path/to/spec.json

stdout 事件（Rust 后端 SSE 转发）：
  start       开始分析
  progress    每 N 步进度
  macro_dps   宏基线 DPS 跑完
  done        分析完成（含不一致统计 / top-K 替换 / 产物路径）
  error       异常
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from collections import Counter
from pathlib import Path

# 与 train.py 一致：强制 UTF-8 stdout/stderr，避免 Windows GBK 编码中文崩溃
if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")  # type: ignore[attr-defined]
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")  # type: ignore[attr-defined]

import numpy as np
import requests
import torch

from env_client import CombatEnvClient, EnvConfig, fetch_spec
from ppo import ActorCritic


# ─────────────────────────────────────────────────────────────────────────────
# 事件输出 / 日志（与 train.py 保持一致的协议）
# ─────────────────────────────────────────────────────────────────────────────

def emit(event: str, **kwargs):
    payload = {"event": event, "ts": time.time(), **kwargs}
    sys.stdout.write(json.dumps(payload, ensure_ascii=False) + "\n")
    sys.stdout.flush()


def log(msg: str):
    sys.stderr.write(msg + "\n")
    sys.stderr.flush()


# ─────────────────────────────────────────────────────────────────────────────
# 默认属性（与 train.py 对齐；可被 spec/CLI 覆盖）
# ─────────────────────────────────────────────────────────────────────────────

DEFAULT_ATTRIBUTES = {
    "li_dao": 100,
    "shen_fa": 24000,
    "base_attack": 30000,
    "base_magical_attack": 0,
    "weapon_damage": 6000,
    "surplus_value": 12000,
    "crit_level": 25000,
    "crit_effect_level": 25000,
    "overcome_level": 30000,
    "strain_level": 25000,
    "haste_level": 5000,
}
DEFAULT_TARGET = {"level": 134, "shield_base": 0, "shield_percent": 0, "damage_cof": 0.0}


# ─────────────────────────────────────────────────────────────────────────────
# 模型加载
# ─────────────────────────────────────────────────────────────────────────────

def load_model(ckpt_path: str, obs_dim: int, action_dim: int, device: str) -> ActorCritic:
    model = ActorCritic(obs_dim, action_dim).to(device)
    # trainer.save 用的 pickle 含 PpoConfig dataclass，需要 weights_only=False
    ckpt = torch.load(ckpt_path, map_location=device, weights_only=False)
    model.load_state_dict(ckpt["model"])
    model.eval()
    return model


@torch.no_grad()
def rl_action(model: ActorCritic, obs: np.ndarray, mask: np.ndarray, device: str) -> int:
    obs_t = torch.as_tensor(obs, dtype=torch.float32, device=device).unsqueeze(0)
    mask_t = torch.as_tensor(mask, dtype=torch.bool, device=device).unsqueeze(0)
    a, _, _, _ = model.get_action_and_value(obs_t, mask_t, deterministic=True)
    return int(a.item())


# ─────────────────────────────────────────────────────────────────────────────
# 参数解析
# ─────────────────────────────────────────────────────────────────────────────

def build_argparser() -> argparse.ArgumentParser:
    ap = argparse.ArgumentParser()
    ap.add_argument("--spec-json", default=None, help="一次性 JSON 配置（前端模式用）")
    ap.add_argument("--base-url", default="http://localhost:3005")
    ap.add_argument("--ckpt", type=str, default=None, help="训练好的 PPO 权重路径")
    ap.add_argument("--macro-text", type=str, default=None,
                    help="宏文本内容（CLI 可直接填路径到文件）")
    ap.add_argument("--duration", type=float, default=300.0)
    ap.add_argument("--initial-rage", type=int, default=None)
    ap.add_argument("--network-delay", type=int, default=0)
    ap.add_argument("--device", default="cuda" if torch.cuda.is_available() else "cpu")
    ap.add_argument("--out-dir", default="./analysis",
                    help="产物输出目录（写 analysis.json / rl_actions.json）")
    ap.add_argument("--run-id", type=str, default=None)
    return ap


def build_args_from_spec(spec_path: str, ap: argparse.ArgumentParser) -> argparse.Namespace:
    with open(spec_path, "r", encoding="utf-8") as f:
        spec = json.load(f)
    args = ap.parse_args([])
    for k, v in spec.items():
        if k in ("attributes", "target", "talents", "recipes"):
            continue
        attr = k.replace("-", "_")
        if hasattr(args, attr):
            setattr(args, attr, v)
    args.attributes = {**DEFAULT_ATTRIBUTES, **spec.get("attributes", {})}
    args.target = {**DEFAULT_TARGET, **spec.get("target", {})}
    args.talents = list(spec.get("talents", []))
    args.recipes = list(spec.get("recipes", []))
    return args


# ─────────────────────────────────────────────────────────────────────────────
# 主流程
# ─────────────────────────────────────────────────────────────────────────────

def main():
    ap = build_argparser()
    raw = ap.parse_args()

    if raw.spec_json:
        args = build_args_from_spec(raw.spec_json, ap)
    else:
        args = raw
        args.attributes = DEFAULT_ATTRIBUTES.copy()
        args.target = DEFAULT_TARGET.copy()
        args.talents = []
        args.recipes = []
        if args.macro_text and Path(args.macro_text).is_file():
            args.macro_text = Path(args.macro_text).read_text(encoding="utf-8")

    if not args.ckpt:
        emit("error", message="缺少 --ckpt")
        sys.exit(1)
    if not args.macro_text:
        emit("error", message="缺少 macro_text（spec-json 的 macro_text 字段 或 CLI --macro-text）")
        sys.exit(1)
    if args.run_id is None:
        args.run_id = f"ana_{int(time.time())}"

    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)

    try:
        log(f"[init] fetching spec from {args.base_url}")
        spec = fetch_spec(args.base_url)

        cfg = EnvConfig(
            attributes=args.attributes,
            target=args.target,
            duration=args.duration,
            haste_level=int(args.attributes.get("haste_level", 0)),
            talents=args.talents,
            recipes=args.recipes,
            initial_rage=args.initial_rage,
            network_delay=args.network_delay,
            collect_timeline=False,
        )
        env = CombatEnvClient(cfg, spec, base_url=args.base_url)
        log(f"[init] loading ckpt {args.ckpt}")
        model = load_model(args.ckpt, spec.obs_dim, spec.action_count, args.device)

        emit("start",
             run_id=args.run_id,
             ckpt=args.ckpt,
             duration=args.duration,
             device=args.device,
             obs_dim=spec.obs_dim,
             action_count=spec.action_count,
             action_names=spec.action_names,
             out_dir=str(out_dir))

        obs, info = env.reset()
        mask = info["action_mask"]
        last_skill: str | None = None
        records = []
        step_idx = 0
        t0 = time.time()
        emit_every = 64

        while True:
            rl_a = rl_action(model, obs, mask, args.device)
            macro_a = env.macro_decision(args.macro_text, last_skill=last_skill)
            records.append({
                "step": step_idx,
                "rl": rl_a,
                "macro": macro_a,
                "agree": rl_a == macro_a,
                "rage": float(obs[0] * 100),
            })
            obs, reward, term, trunc, info = env.step(rl_a)
            mask = info["action_mask"]
            if rl_a > 0:
                last_skill = spec.action_names[rl_a].split("·")[0]
            step_idx += 1
            if step_idx % emit_every == 0:
                n_disagree = sum(1 for r in records if not r["agree"])
                env_info = env.env_info()
                emit("progress",
                     step=step_idx,
                     elapsed=time.time() - t0,
                     sim_elapsed=float(env_info.get("elapsed", 0.0)),
                     duration=args.duration,
                     disagree_rate=n_disagree / len(records),
                     current_dps=float(env_info.get("dps", 0.0)))
            if term or trunc:
                break

        env_info = env.env_info()
        rl_dps = float(env_info.get("dps", 0.0))
        rl_total = float(env_info.get("total_damage", 0.0))

        # 纯宏 baseline rollout（用 /api/rl/rollout）
        log("[analyze] running macro baseline rollout")
        macro_resp = requests.post(
            f"{args.base_url}/api/rl/rollout",
            json={
                "policy": {"kind": "macro", "macro_text": args.macro_text},
                "attributes": args.attributes,
                "target": args.target,
                "duration": args.duration,
                "haste_level": int(args.attributes.get("haste_level", 0)),
                "talents": args.talents,
                "recipes": args.recipes,
                "initial_rage": args.initial_rage,
                "network_delay": args.network_delay,
            },
            timeout=180,
        )
        macro_resp.raise_for_status()
        macro_dps = float(macro_resp.json()["dps"])
        emit("macro_dps", macro_dps=macro_dps)

        n_total = len(records)
        n_disagree = sum(1 for r in records if not r["agree"])
        delta_pct = (rl_dps - macro_dps) / max(macro_dps, 1) * 100
        patterns = Counter((r["macro"], r["rl"]) for r in records if not r["agree"])
        top_patterns = [
            {
                "macro": int(m),
                "rl": int(rl),
                "macro_name": spec.action_names[m],
                "rl_name": spec.action_names[rl],
                "count": int(c),
            }
            for (m, rl), c in patterns.most_common(30)
        ]

        # 写出
        analysis_path = out_dir / "analysis.json"
        analysis_path.write_text(json.dumps({
            "rl_dps": rl_dps,
            "macro_dps": macro_dps,
            "delta_pct": delta_pct,
            "rl_total_damage": rl_total,
            "n_total_decisions": n_total,
            "n_disagree": n_disagree,
            "disagree_rate": n_disagree / n_total if n_total else 0.0,
            "patterns": top_patterns,
            "records": records,
        }, ensure_ascii=False, indent=2), encoding="utf-8")

        actions_path = out_dir / "rl_actions.json"
        actions_path.write_text(json.dumps({
            "policy": "actions",
            "actions": [int(r["rl"]) for r in records],
            "duration": args.duration,
            "haste_level": int(args.attributes.get("haste_level", 0)),
            "talents": args.talents,
            "recipes": args.recipes,
            "initial_rage": args.initial_rage,
            "network_delay": args.network_delay,
            "attributes": args.attributes,
            "target": args.target,
            "meta": {"rl_dps": rl_dps, "macro_dps": macro_dps, "delta_pct": delta_pct, "ckpt": args.ckpt},
        }, ensure_ascii=False, indent=2), encoding="utf-8")

        emit("done",
             rl_dps=rl_dps,
             macro_dps=macro_dps,
             delta_pct=delta_pct,
             rl_total_damage=rl_total,
             n_total_decisions=n_total,
             n_disagree=n_disagree,
             disagree_rate=n_disagree / n_total if n_total else 0.0,
             patterns=top_patterns,
             analysis_path=str(analysis_path),
             actions_path=str(actions_path))

        env.close()
    except KeyboardInterrupt:
        emit("error", message="interrupted by SIGINT")
    except Exception as e:
        import traceback
        emit("error", message=str(e), traceback=traceback.format_exc())
        raise


if __name__ == "__main__":
    main()
