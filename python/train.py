"""训练入口：N 个 CombatEnvClient 并行采样 + PyTorch PPO 更新。

两种调用方式：
1. CLI（开发调试）：
     python train.py --total-steps 5_000_000 --n-envs 8 --duration 60 --device cuda

2. spec-json（前端"RL 训练"tab 用）：
     python train.py --spec-json /path/to/spec.json
   spec.json 包含全部参数 + attributes/target/talents/recipes/macro_text 等。

事件输出（前端 SSE 转发）：
- stdout：每个事件一行 JSON `{"event": "...", ...}`，以 \n 分隔，每行 flush
- stderr：人类可读日志（tqdm 等），后端忽略

事件类型：
  start            训练开始
  update           每个 update 完成
  checkpoint       存盘
  episode          单条 episode 完成
  done             训练完成
  error            异常
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time

# Windows subprocess 下 stdout/stderr 默认编码可能是 GBK → 中文 emit 会 UnicodeEncodeError
# 必须在任何 print/emit 之前强制 UTF-8
if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")  # type: ignore[attr-defined]
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")  # type: ignore[attr-defined]
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import numpy as np
import torch
from torch.utils.tensorboard import SummaryWriter

from env_client import CombatEnvClient, EnvConfig, EnvSpec, fetch_spec
from ppo import PpoConfig, PpoTrainer


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
DEFAULT_TARGET = {
    "level": 134,
    "shield_base": 0,
    "shield_percent": 0,
    "damage_cof": 0.0,
}


# ─────────────────────────────────────────────────────────────────────────────
# 事件输出
# ─────────────────────────────────────────────────────────────────────────────

def emit(event: str, **kwargs):
    """单行 JSON 写 stdout，立即 flush。Rust 后端按行 parse 后 SSE 广播给前端。"""
    payload = {"event": event, "ts": time.time(), **kwargs}
    sys.stdout.write(json.dumps(payload, ensure_ascii=False) + "\n")
    sys.stdout.flush()


def log(msg: str):
    """人类日志走 stderr，不污染事件流"""
    sys.stderr.write(msg + "\n")
    sys.stderr.flush()


# ─────────────────────────────────────────────────────────────────────────────
# spec.json 解析
# ─────────────────────────────────────────────────────────────────────────────

def build_args_from_spec(spec_path: str, ap: argparse.ArgumentParser) -> argparse.Namespace:
    with open(spec_path, "r", encoding="utf-8") as f:
        spec = json.load(f)
    args = ap.parse_args([])  # 取 defaults
    for k, v in spec.items():
        if k in ("attributes", "target", "talents", "recipes"):
            continue
        # 把下划线 / 横线 都接受
        attr = k.replace("-", "_")
        if hasattr(args, attr):
            setattr(args, attr, v)
    args.attributes = {**DEFAULT_ATTRIBUTES, **spec.get("attributes", {})}
    args.target = {**DEFAULT_TARGET, **spec.get("target", {})}
    args.talents = list(spec.get("talents", []))
    args.recipes = list(spec.get("recipes", []))
    args.allowed_actions = spec.get("allowed_actions")
    return args


# ─────────────────────────────────────────────────────────────────────────────
# 环境管理
# ─────────────────────────────────────────────────────────────────────────────

def fetch_runtime_params(base_url: str) -> dict | None:
    """从后端拉最新的可调参数（前端可能改动）；失败返回 None"""
    try:
        import requests as _r
        r = _r.get(f"{base_url}/api/rl/train/params", timeout=3)
        if r.ok:
            return r.json()
    except Exception:
        pass
    return None


def run_eval_rollout(trainer, eval_env: CombatEnvClient, args, spec: EnvSpec) -> dict | None:
    """跑一局确定性策略，调 /api/rl/rollout 拿带伤害的时间轴；返回 rollout 响应"""
    import requests as _r
    obs, info = eval_env.reset()
    mask = info["action_mask"]
    actions: list[int] = []
    while True:
        a, _, _ = trainer.select_action(obs[None], mask[None], deterministic=True)
        action = int(a[0])
        actions.append(action)
        obs, _r_, term, trunc, info = eval_env.step(action)
        mask = info["action_mask"]
        if term or trunc:
            break
        if len(actions) > 50000:
            break
    try:
        resp = _r.post(
            f"{args.base_url}/api/rl/rollout",
            json={
                "policy": {"kind": "actions", "actions": actions},
                "attributes": args.attributes,
                "target": args.target,
                "duration": args.eval_duration,
                "haste_level": int(args.attributes.get("haste_level", 0)),
                "talents": args.talents,
                "recipes": args.recipes,
                "initial_rage": args.initial_rage,
                "network_delay": args.network_delay,
            },
            timeout=120,
        )
        resp.raise_for_status()
        return resp.json()
    except Exception as e:
        log(f"[eval] rollout failed: {e}")
        return None


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
        allowed_actions=getattr(args, "allowed_actions", None),
    )
    return CombatEnvClient(
        cfg, spec, base_url=args.base_url, baseline_dps=args.baseline_dps
    )


def parallel_reset(envs, pool):
    results = list(pool.map(lambda e: e.reset(), envs))
    obs = np.stack([r[0] for r in results])
    masks = np.stack([r[1]["action_mask"] for r in results])
    return obs, masks


def parallel_step(envs, pool, actions):
    def _one(args):
        env, a = args
        return env.step(int(a))
    results = list(pool.map(_one, zip(envs, actions)))
    obs = np.stack([r[0] for r in results])
    rewards = np.array([r[1] for r in results], dtype=np.float32)
    terms = np.array([r[2] for r in results], dtype=bool)
    truncs = np.array([r[3] for r in results], dtype=bool)
    masks = np.stack([r[4]["action_mask"] for r in results])
    dones = terms | truncs
    return obs, rewards, dones, masks, [r[4] for r in results]


def maybe_reset_done(envs, pool, dones, obs, masks):
    if not dones.any():
        return obs, masks
    idx = np.where(dones)[0]

    def _r(i):
        o, info = envs[i].reset()
        return i, o, info["action_mask"]

    for i, o, m in pool.map(_r, idx):
        obs[i] = o
        masks[i] = m
    return obs, masks


# ─────────────────────────────────────────────────────────────────────────────
# 主流程
# ─────────────────────────────────────────────────────────────────────────────

def build_argparser() -> argparse.ArgumentParser:
    ap = argparse.ArgumentParser()
    ap.add_argument("--spec-json", default=None,
                    help="一次性 JSON 配置（前端模式用）；提供后忽略其他 CLI 参数（除 spec 内容）")
    ap.add_argument("--base-url", default="http://localhost:3005")
    ap.add_argument("--eval-every", type=int, default=10,
                    help="后期每 N 次 update 跑一局 eval；0 = 关闭")
    ap.add_argument("--eval-warmup-updates", type=int, default=20,
                    help="前 N 个 update 用 warmup 间隔（短间隔）")
    ap.add_argument("--eval-warmup-every", type=int, default=2,
                    help="warmup 期间的 eval 间隔")
    ap.add_argument("--eval-duration", type=float, default=60.0,
                    help="eval 的 episode 时长（秒），独立于训练 duration")
    ap.add_argument("--total-steps", type=int, default=5_000_000)
    ap.add_argument("--n-envs", type=int, default=8)
    ap.add_argument("--n-steps", type=int, default=2048)
    ap.add_argument("--n-epochs", type=int, default=10)
    ap.add_argument("--minibatch-size", type=int, default=512)
    ap.add_argument("--lr", type=float, default=3e-4)
    ap.add_argument("--gamma", type=float, default=0.999)
    ap.add_argument("--ent-coef", type=float, default=0.01)
    ap.add_argument("--duration", type=float, default=60.0)
    ap.add_argument("--initial-rage", type=int, default=None)
    ap.add_argument("--network-delay", type=int, default=0)
    ap.add_argument("--baseline-dps", type=float, default=None)
    ap.add_argument("--device", default="cuda" if torch.cuda.is_available() else "cpu")
    ap.add_argument("--save-dir", default="./checkpoints")
    ap.add_argument("--log-dir", default="./tb_logs")
    ap.add_argument("--save-every", type=int, default=20)
    ap.add_argument("--seed", type=int, default=42)
    ap.add_argument("--resume", type=str, default=None)
    ap.add_argument("--reset-optimizer", action="store_true")
    ap.add_argument("--run-id", type=str, default=None,
                    help="标识本次训练（前端 SSE 关联）；不指定时自动生成")
    return ap


def _raw_spec_dict(spec_path: str) -> dict:
    with open(spec_path, "r", encoding="utf-8") as f:
        return json.load(f)


def _override_args(args: argparse.Namespace, overrides: dict):
    """把 stage 覆盖应用到 args（只允许受支持字段，避免误写）"""
    allowed = {
        "duration", "total_steps", "n_envs", "n_steps", "n_epochs",
        "minibatch_size", "lr", "gamma", "ent_coef", "baseline_dps",
        "save_every", "initial_rage", "network_delay",
    }
    for k, v in overrides.items():
        attr = k.replace("-", "_")
        if attr in allowed:
            setattr(args, attr, v)


def _run_stages(args: argparse.Namespace, stages: list[dict]):
    """课程学习：按顺序跑每个 stage，前一阶段 final.pt 作为下一阶段 resume

    路径布局：<save_dir>/<stage_name>/ppo_*.pt，<log_dir>/<stage_name>/
    """
    base_run_id = args.run_id
    base_save_dir = args.save_dir
    base_log_dir = args.log_dir
    prev_ckpt: str | None = args.resume
    for i, stage in enumerate(stages):
        stage_name = stage.get("name", f"stage{i+1}")
        args.run_id = f"{base_run_id}/{stage_name}"
        args.save_dir = str(Path(base_save_dir) / stage_name)
        args.log_dir = str(Path(base_log_dir) / stage_name)
        _override_args(args, stage)
        if prev_ckpt:
            args.resume = prev_ckpt
        emit("stage_start", index=i + 1, total=len(stages), name=stage_name,
             duration=args.duration, total_steps=args.total_steps,
             lr=args.lr, resume=prev_ckpt,
             save_dir=args.save_dir)
        final = _run_single(args)
        if final is None:
            emit("stage_abort", index=i + 1, name=stage_name)
            return
        prev_ckpt = final
    emit("curriculum_done", final_ckpt=prev_ckpt)


def _run_single(args: argparse.Namespace) -> str | None:
    """单阶段训练，返回 final ckpt 路径（失败返回 None）

    `args.save_dir` / `args.log_dir` 视为本次 run 的最终目录（已由调用方/后端拼好，
    不再追加 run_id，避免双层嵌套）。
    """
    np.random.seed(args.seed)
    torch.manual_seed(args.seed)

    save_dir = Path(args.save_dir)
    save_dir.mkdir(parents=True, exist_ok=True)
    log_dir = Path(args.log_dir)
    log_dir.mkdir(parents=True, exist_ok=True)
    writer = SummaryWriter(str(log_dir))

    try:
        log(f"[init] fetching spec from {args.base_url}")
        spec = fetch_spec(args.base_url)
        log(f"[init] obs_dim={spec.obs_dim} action_count={spec.action_count}")
        log(f"[init] creating {args.n_envs} env sessions")
        pool = ThreadPoolExecutor(max_workers=args.n_envs)
        envs = list(pool.map(lambda _: make_env(args, spec), range(args.n_envs)))

        cfg = PpoConfig(
            n_steps=args.n_steps,
            n_envs=args.n_envs,
            n_epochs=args.n_epochs,
            minibatch_size=args.minibatch_size,
            learning_rate=args.lr,
            gamma=args.gamma,
            ent_coef=args.ent_coef,
        )
        trainer = PpoTrainer(spec.obs_dim, spec.action_count, cfg, device=args.device)
        n_params = sum(p.numel() for p in trainer.model.parameters())
        log(f"[init] PPO on {args.device}; params={n_params}")

        if args.resume:
            trainer.load(args.resume, load_optimizer=not args.reset_optimizer)
            log(f"[resume] {args.resume}; global_step={trainer.global_step}")

        emit("start",
             run_id=args.run_id,
             obs_dim=spec.obs_dim,
             action_count=spec.action_count,
             action_names=spec.action_names,
             total_steps=args.total_steps,
             n_envs=args.n_envs,
             n_steps=args.n_steps,
             device=args.device,
             n_params=n_params,
             save_dir=str(save_dir),
             tb_log_dir=str(log_dir),
             baseline_dps=args.baseline_dps,
             resumed_step=trainer.global_step)

        # 串行 reset 而不是并行——便于定位卡哪个 env，并且 reset 只发生一次性能不重要
        emit("phase", name="reset_envs_begin", n=args.n_envs)
        obs_list = []
        mask_list = []
        for i, e in enumerate(envs):
            t0 = time.time()
            o, info = e.reset()
            dt = time.time() - t0
            emit("phase", name="env_reset_done", index=i, elapsed=dt)
            obs_list.append(o)
            mask_list.append(info["action_mask"])
        obs = np.stack(obs_list)
        masks = np.stack(mask_list)
        emit("phase", name="reset_envs_done")

        # 独立的 eval env：仅用于周期性确定性 rollout 演示
        eval_env = None
        if args.eval_every > 0:
            eval_env = make_env(args, spec)
            emit("phase", name="eval_env_ready", eval_duration=args.eval_duration, eval_every=args.eval_every)

        steps_per_update = args.n_steps * args.n_envs
        n_updates_total = args.total_steps // steps_per_update
        n_updates_done = trainer.global_step // steps_per_update
        n_updates = max(n_updates_total - n_updates_done, 0)
        if n_updates == 0:
            emit("done", reason="already_at_total_steps", final_ckpt=None,
                 total_steps=trainer.global_step)
            return None

        ep_returns = np.zeros(args.n_envs, dtype=np.float32)
        ep_lengths = np.zeros(args.n_envs, dtype=np.int32)
        completed_returns: list[float] = []
        completed_dps: list[float] = []
        completed_lengths: list[int] = []

        log(f"[train] {n_updates} updates × {steps_per_update} steps/update")
        t_start = time.time()
        progress_every = max(args.n_steps // 16, 32)
        for update in range(1, n_updates + 1):
            update_t0 = time.time()
            step_t0 = time.time()
            emit("rollout_begin", update=update, total_updates=n_updates,
                 global_step=trainer.global_step, n_steps=args.n_steps)
            for step in range(args.n_steps):
                tick_t0 = time.time()
                actions, log_probs, values = trainer.select_action(obs, masks)
                sel_ms = (time.time() - tick_t0) * 1000
                http_t0 = time.time()
                new_obs, rewards, dones, new_masks, infos = parallel_step(
                    envs, pool, actions
                )
                http_ms = (time.time() - http_t0) * 1000
                trainer.buffer.add(step, obs, masks, actions, log_probs, rewards, dones, values)
                ep_returns += rewards
                ep_lengths += 1
                if step == 0 or (step + 1) % progress_every == 0:
                    emit("rollout_progress",
                         update=update,
                         step=step + 1,
                         n_steps=args.n_steps,
                         elapsed=time.time() - step_t0,
                         sel_ms=sel_ms,
                         http_ms=http_ms,
                         global_step=trainer.global_step)
                for i, d in enumerate(dones):
                    if d:
                        ret = float(ep_returns[i])
                        length = int(ep_lengths[i])
                        completed_returns.append(ret)
                        completed_lengths.append(length)
                        ep_dps = float(infos[i].get("episode_dps", 0.0))
                        # 即便 0 也记录，防止策略 collapse 时 dps 指标空白
                        completed_dps.append(ep_dps)
                        emit("episode",
                             env_idx=i,
                             ret=ret,
                             length=length,
                             dps=ep_dps,
                             global_step=trainer.global_step + (step + 1) * args.n_envs)
                        ep_returns[i] = 0.0
                        ep_lengths[i] = 0
                new_obs, new_masks = maybe_reset_done(envs, pool, dones, new_obs, new_masks)
                obs, masks = new_obs, new_masks
                trainer.global_step += args.n_envs

            last_dones = np.zeros(args.n_envs, dtype=np.float32)
            metrics = trainer.update(obs, masks, last_dones)
            update_dt = time.time() - update_t0
            sps = steps_per_update / max(update_dt, 1e-6)
            steps = trainer.global_step

            for k, v in metrics.items():
                writer.add_scalar(k, v, steps)
            ret_mean = float(np.mean(completed_returns[-50:])) if completed_returns else 0.0
            dps_mean = float(np.mean(completed_dps[-50:])) if completed_dps else 0.0
            len_mean = float(np.mean(completed_lengths[-50:])) if completed_lengths else 0.0
            writer.add_scalar("episode/return_mean", ret_mean, steps)
            writer.add_scalar("episode/dps_mean", dps_mean, steps)
            writer.add_scalar("episode/length_mean", len_mean, steps)
            writer.add_scalar("perf/steps_per_sec", sps, steps)

            emit("update",
                 update=update,
                 total_updates=n_updates,
                 global_step=steps,
                 sps=sps,
                 elapsed=time.time() - t_start,
                 metrics=metrics,
                 ep_return_mean=ret_mean,
                 ep_dps_mean=dps_mean,
                 ep_length_mean=len_mean,
                 n_completed_episodes=len(completed_returns))

            if update % args.save_every == 0:
                ckpt = save_dir / f"ppo_step{steps}.pt"
                trainer.save(str(ckpt))
                emit("checkpoint", step=steps, path=str(ckpt))

            # 拉一次最新的运行时参数（前端可能改了 eval_every / eval_duration）
            rp = fetch_runtime_params(args.base_url)
            if rp:
                new_every = int(rp.get("eval_every", args.eval_every))
                new_dur = float(rp.get("eval_duration", args.eval_duration))
                if new_every != args.eval_every or abs(new_dur - args.eval_duration) > 1e-6:
                    emit("runtime_params",
                         eval_every=new_every, eval_duration=new_dur,
                         prev_eval_every=args.eval_every, prev_eval_duration=args.eval_duration)
                    args.eval_every = new_every
                    args.eval_duration = new_dur

            # 周期性 eval rollout：跑一局确定性策略并把时间轴推给前端
            if eval_env is not None and args.eval_every > 0 and update % args.eval_every == 0:
                eval_t0 = time.time()
                roll = run_eval_rollout(trainer, eval_env, args, spec)
                if roll is not None:
                    emit("eval_rollout",
                         update=update,
                         global_step=steps,
                         eval_elapsed=time.time() - eval_t0,
                         dps=float(roll.get("dps", 0.0)),
                         total_damage=float(roll.get("total_damage", 0.0)),
                         fight_time=float(roll.get("fight_time", 0.0)),
                         skill_count=int(roll.get("skill_count", 0)),
                         timeline=roll.get("timeline", []))

        final = save_dir / "ppo_final.pt"
        trainer.save(str(final))
        elapsed = time.time() - t_start
        log(f"[done] elapsed {elapsed:.1f}s; final → {final}")
        emit("done",
             final_ckpt=str(final),
             total_steps=trainer.global_step,
             elapsed=elapsed,
             best_return=max(completed_returns) if completed_returns else 0.0,
             best_dps=max(completed_dps) if completed_dps else 0.0)

        for e in envs:
            try: e.close()
            except Exception: pass
        if eval_env is not None:
            try: eval_env.close()
            except Exception: pass
        writer.close()
        return str(final)
    except KeyboardInterrupt:
        emit("error", message="interrupted by SIGINT")
        return None
    except Exception as e:
        import traceback
        emit("error", message=str(e), traceback=traceback.format_exc())
        raise


def main():
    ap = build_argparser()
    raw = ap.parse_args()
    if raw.spec_json:
        args = build_args_from_spec(raw.spec_json, ap)
        raw_spec = _raw_spec_dict(raw.spec_json)
        stages = raw_spec.get("stages") or []
    else:
        args = raw
        args.attributes = DEFAULT_ATTRIBUTES.copy()
        args.target = DEFAULT_TARGET.copy()
        args.talents = []
        args.recipes = []
        stages = []
    if args.run_id is None:
        args.run_id = f"rl_{int(time.time())}"

    # CLI 模式把 run_id 拼进 save_dir（spec-json 模式由后端已经拼好）
    if not raw.spec_json:
        args.save_dir = str(Path(args.save_dir) / args.run_id)
        args.log_dir = str(Path(args.log_dir) / args.run_id)

    if stages:
        _run_stages(args, stages)
    else:
        _run_single(args)


if __name__ == "__main__":
    main()
