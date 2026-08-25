"""行为克隆（BC）预训练：用宏策略当专家，监督学习一个初始策略。

输出 ckpt 与 PPO trainer.save 同格式，可被 RL 训练 tab "续训" 加载。

事件协议（同 train.py）：
  start          收集开始
  collect_progress  采集进度
  train_progress    训练进度（每 epoch）
  checkpoint     保存
  done           完成
  error          异常
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")  # type: ignore[attr-defined]
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")  # type: ignore[attr-defined]

import numpy as np
import torch
import torch.nn.functional as F

from env_client import CombatEnvClient, EnvConfig, EnvSpec, fetch_spec
from ppo import ActorCritic, PpoConfig


DEFAULT_ATTRIBUTES = {
    "li_dao": 100, "shen_fa": 24000, "base_attack": 30000, "base_magical_attack": 0,
    "weapon_damage": 6000, "surplus_value": 12000,
    "crit_level": 25000, "crit_effect_level": 25000,
    "overcome_level": 30000, "strain_level": 25000, "haste_level": 5000,
}
DEFAULT_TARGET = {"level": 134, "shield_base": 0, "shield_percent": 0, "damage_cof": 0.0}


def emit(event: str, **kwargs):
    payload = {"event": event, "ts": time.time(), **kwargs}
    sys.stdout.write(json.dumps(payload, ensure_ascii=False) + "\n")
    sys.stdout.flush()


def log(msg: str):
    sys.stderr.write(msg + "\n")
    sys.stderr.flush()


def build_argparser() -> argparse.ArgumentParser:
    ap = argparse.ArgumentParser()
    ap.add_argument("--spec-json", default=None)
    ap.add_argument("--base-url", default="http://localhost:3005")
    ap.add_argument("--macro-text", type=str, default=None)
    ap.add_argument("--duration", type=float, default=120.0)
    ap.add_argument("--initial-rage", type=int, default=0)
    ap.add_argument("--network-delay", type=int, default=0)
    ap.add_argument("--n-envs", type=int, default=4, help="并行采集 env 数")
    ap.add_argument("--n-samples", type=int, default=50000, help="总样本数")
    ap.add_argument("--n-epochs", type=int, default=10)
    ap.add_argument("--batch-size", type=int, default=512)
    ap.add_argument("--lr", type=float, default=1e-3)
    ap.add_argument("--hidden", type=int, default=256)
    ap.add_argument("--device", default="cuda" if torch.cuda.is_available() else "cpu")
    ap.add_argument("--save-dir", default="./pretrains")
    ap.add_argument("--run-id", type=str, default=None)
    ap.add_argument("--seed", type=int, default=42)
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


def make_env(args, spec: EnvSpec) -> CombatEnvClient:
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
    return CombatEnvClient(cfg, spec, base_url=args.base_url)


def collect_one_env(env: CombatEnvClient, macro_text: str, n_target: int,
                    action_names: list[str]) -> tuple[list, int]:
    """单 env 跑宏策略，收 n_target 条 (obs, mask, macro_action) 三元组"""
    data: list[tuple[np.ndarray, np.ndarray, int]] = []
    obs, info = env.reset()
    mask = info["action_mask"]
    last_skill: str | None = None
    n_episodes = 0
    while len(data) < n_target:
        macro_a = env.macro_decision(macro_text, last_skill=last_skill)
        # 跳过非法动作（理论上 macro 给出的应是合法的，但保险起见）
        if macro_a < len(mask) and mask[macro_a]:
            data.append((obs.copy(), mask.copy(), int(macro_a)))
        obs, _, term, trunc, info = env.step(macro_a)
        mask = info["action_mask"]
        if macro_a > 0 and macro_a < len(action_names):
            base_name = action_names[macro_a].split('·')[0]
            last_skill = base_name
        if term or trunc:
            obs, info = env.reset()
            mask = info["action_mask"]
            last_skill = None
            n_episodes += 1
    return data, n_episodes


def train_bc(model: ActorCritic, dataset, n_epochs: int, batch_size: int, lr: float,
             device: str, log_every_batches: int = 50):
    n = len(dataset)
    obs_arr = np.stack([d[0] for d in dataset]).astype(np.float32)
    mask_arr = np.stack([d[1] for d in dataset]).astype(bool)
    target_arr = np.array([d[2] for d in dataset], dtype=np.int64)

    obs_t = torch.from_numpy(obs_arr).to(device)
    mask_t = torch.from_numpy(mask_arr).to(device)
    target_t = torch.from_numpy(target_arr).to(device)

    optimizer = torch.optim.Adam(model.parameters(), lr=lr)
    model.train()

    for epoch in range(1, n_epochs + 1):
        perm = torch.randperm(n, device=device)
        total_loss = 0.0
        total_correct = 0
        total_seen = 0
        for start in range(0, n, batch_size):
            idx = perm[start:start + batch_size]
            logits, _ = model.forward(obs_t[idx])
            masked = logits.masked_fill(~mask_t[idx], float("-inf"))
            tgt = target_t[idx]
            loss = F.cross_entropy(masked, tgt)
            optimizer.zero_grad()
            loss.backward()
            torch.nn.utils.clip_grad_norm_(model.parameters(), 0.5)
            optimizer.step()

            with torch.no_grad():
                pred = masked.argmax(dim=-1)
                correct = (pred == tgt).sum().item()
            total_loss += loss.item() * tgt.numel()
            total_correct += correct
            total_seen += tgt.numel()

        avg_loss = total_loss / max(total_seen, 1)
        acc = total_correct / max(total_seen, 1)
        emit("train_progress",
             epoch=epoch,
             total_epochs=n_epochs,
             loss=avg_loss,
             top1_acc=acc,
             n_samples=total_seen)


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

    if not args.macro_text:
        emit("error", message="缺少 macro_text")
        sys.exit(1)
    if args.run_id is None:
        args.run_id = f"bc_{int(time.time())}"

    np.random.seed(args.seed)
    torch.manual_seed(args.seed)

    save_dir = Path(args.save_dir)
    save_dir.mkdir(parents=True, exist_ok=True)

    try:
        log(f"[init] fetching spec from {args.base_url}")
        spec = fetch_spec(args.base_url)
        log(f"[init] obs_dim={spec.obs_dim} action_count={spec.action_count}")

        log(f"[init] creating {args.n_envs} env sessions")
        pool = ThreadPoolExecutor(max_workers=args.n_envs)
        envs = list(pool.map(lambda _: make_env(args, spec), range(args.n_envs)))

        emit("start",
             run_id=args.run_id,
             obs_dim=spec.obs_dim,
             action_count=spec.action_count,
             n_envs=args.n_envs,
             n_samples=args.n_samples,
             n_epochs=args.n_epochs,
             batch_size=args.batch_size,
             lr=args.lr,
             device=args.device,
             save_dir=str(save_dir))

        # ── 阶段 1：并行采集 ──
        per_env_target = (args.n_samples + args.n_envs - 1) // args.n_envs
        emit("phase", name="collect_begin", per_env=per_env_target)

        # 逐步采，每 200 个样本 emit progress（用 future 的 done 判断）
        # 用 simple sequential per-env，但用线程池并发
        results = list(pool.map(
            lambda e: collect_one_env(e, args.macro_text, per_env_target, spec.action_names),
            envs
        ))
        all_data = []
        total_eps = 0
        for d, neps in results:
            all_data.extend(d)
            total_eps += neps
        emit("phase", name="collect_done",
             n_samples=len(all_data),
             n_episodes=total_eps)

        # 统计每个动作的占比（看宏策略的偏好）
        from collections import Counter
        action_dist = Counter(d[2] for d in all_data)
        dist_summary = {
            spec.action_names[a]: int(c)
            for a, c in sorted(action_dist.items(), key=lambda x: -x[1])
        }
        emit("action_distribution", dist=dist_summary, total=len(all_data))

        # ── 阶段 2：监督学习 ──
        emit("phase", name="train_begin", n_samples=len(all_data))
        model = ActorCritic(spec.obs_dim, spec.action_count, hidden=args.hidden).to(args.device)
        n_params = sum(p.numel() for p in model.parameters())
        log(f"[init] model params={n_params}")

        train_bc(model, all_data, args.n_epochs, args.batch_size, args.lr, args.device)
        emit("phase", name="train_done")

        # ── 保存：与 PPO ckpt 兼容（含 cfg 占位）──
        ckpt_path = save_dir / "pretrained.pt"
        # PpoConfig 用默认值；后续 PPO 续训时会被新 ctx 的 PpoConfig 覆盖
        ckpt = {
            "model": model.state_dict(),
            "global_step": 0,
            "cfg": PpoConfig(),
            "extra": {
                "source": "behavioral_cloning",
                "run_id": args.run_id,
                "n_samples": len(all_data),
                "n_epochs": args.n_epochs,
                "action_distribution": dist_summary,
            },
        }
        torch.save(ckpt, str(ckpt_path))
        emit("checkpoint", path=str(ckpt_path), step=0)

        emit("done",
             ckpt=str(ckpt_path),
             n_samples=len(all_data),
             n_episodes=total_eps,
             n_params=n_params)

        for e in envs:
            try: e.close()
            except Exception: pass

    except KeyboardInterrupt:
        emit("error", message="interrupted by SIGINT")
    except Exception as e:
        import traceback
        emit("error", message=str(e), traceback=traceback.format_exc())
        raise


if __name__ == "__main__":
    main()
