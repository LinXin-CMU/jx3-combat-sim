"""HTTP-RPC 包装：把后端 /api/rl/env/* 端点暴露成 gymnasium.Env。

每个 CombatEnvClient 在 __init__ 时向后端创建一个 session，析构时 close。
step() 内部自动调 /step + /advance（跳到下一个决策点），合并两段 reward。

约定：
- obs 为 float32 一维数组，维度 = backend OBS_DIM（首次创建时从 /spec 读取）
- action 为 int，0 ≤ action < ACTION_COUNT
- 非法动作由 mask 屏蔽；调用方负责在采样前应用 mask（见 ppo.py 的 masked logits）
"""

from __future__ import annotations

import atexit
import time
from dataclasses import dataclass, field
from typing import Any

import gymnasium as gym
import numpy as np
import requests
from gymnasium import spaces


@dataclass
class EnvSpec:
    obs_dim: int
    action_count: int
    action_names: list[str]


def fetch_spec(base_url: str, timeout: float = 10.0) -> EnvSpec:
    r = requests.get(f"{base_url}/api/rl/spec", timeout=timeout)
    r.raise_for_status()
    j = r.json()
    return EnvSpec(
        obs_dim=int(j["obs_dim"]),
        action_count=int(j["action_count"]),
        action_names=list(j["action_names"]),
    )


@dataclass
class EnvConfig:
    """创建后端 session 时的参数（对应 backend rl::http::CreateRequest）"""

    attributes: dict
    target: dict
    duration: float = 300.0
    haste_level: int = 0
    talents: list[int] = field(default_factory=list)
    recipes: list[int] = field(default_factory=list)
    initial_rage: int | None = None
    network_delay: int = 0
    collect_timeline: bool = False
    allowed_actions: list[int] | None = None


class CombatEnvClient(gym.Env):
    """HTTP-RPC 驱动的 jx3 战斗 RL 环境"""

    metadata = {"render_modes": []}

    def __init__(
        self,
        cfg: EnvConfig,
        spec: EnvSpec,
        base_url: str = "http://localhost:3005",
        request_timeout: float = 30.0,
        baseline_dps: float | None = None,
    ):
        super().__init__()
        self.cfg = cfg
        self.spec_meta = spec
        self.base_url = base_url.rstrip("/")
        self.request_timeout = request_timeout
        self.baseline_dps = baseline_dps  # 用于 episode_bonus；None 则不附加

        self.observation_space = spaces.Box(
            low=0.0, high=1.0, shape=(spec.obs_dim,), dtype=np.float32
        )
        self.action_space = spaces.Discrete(spec.action_count)

        self.session = requests.Session()
        self.session_id: str | None = None
        self._last_mask: np.ndarray | None = None
        self._last_obs: np.ndarray | None = None
        self._create_session()
        atexit.register(self._safe_close)

    # ─── HTTP 辅助 ───
    def _post(self, path: str, payload: dict | None = None) -> dict:
        url = f"{self.base_url}{path}"
        for attempt in range(3):
            try:
                r = self.session.post(url, json=payload, timeout=self.request_timeout)
                r.raise_for_status()
                return r.json()
            except requests.exceptions.RequestException:
                if attempt == 2:
                    raise
                time.sleep(0.05 * (attempt + 1))
        raise RuntimeError("unreachable")

    def _get(self, path: str) -> dict:
        url = f"{self.base_url}{path}"
        r = self.session.get(url, timeout=self.request_timeout)
        r.raise_for_status()
        return r.json()

    # ─── session 生命周期 ───
    def _create_session(self) -> None:
        body = {
            "attributes": self.cfg.attributes,
            "target": self.cfg.target,
            "duration": self.cfg.duration,
            "haste_level": self.cfg.haste_level,
            "talents": self.cfg.talents,
            "recipes": self.cfg.recipes,
            "initial_rage": self.cfg.initial_rage,
            "network_delay": self.cfg.network_delay,
            "collect_timeline": self.cfg.collect_timeline,
            "allowed_actions": self.cfg.allowed_actions,
        }
        resp = self._post("/api/rl/env/create", body)
        self.session_id = resp["session_id"]
        self._last_obs = np.asarray(resp["obs"], dtype=np.float32)
        self._last_mask = np.asarray(resp["mask"], dtype=bool)

    def _safe_close(self) -> None:
        if self.session_id is None:
            return
        try:
            self._post(f"/api/rl/env/{self.session_id}/close")
        except Exception:
            pass
        self.session_id = None

    def close(self) -> None:
        self._safe_close()

    # ─── gym.Env API ───
    def reset(self, *, seed: int | None = None, options: dict | None = None):
        super().reset(seed=seed)
        if self.session_id is None:
            self._create_session()
        else:
            resp = self._post(f"/api/rl/env/{self.session_id}/reset")
            self._last_obs = np.asarray(resp["obs"], dtype=np.float32)
            self._last_mask = np.asarray(resp["mask"], dtype=bool)
        # reset 后立即推进到第一个决策点
        self._advance()
        return self._last_obs.copy(), {"action_mask": self._last_mask.copy()}

    def step(self, action: int):
        # 合并的 step+advance 单次 HTTP（砍一半往返开销）
        s = self._post(
            f"/api/rl/env/{self.session_id}/step_advance", {"action": int(action)}
        )
        self._last_obs = np.asarray(s["obs"], dtype=np.float32)
        self._last_mask = np.asarray(s["mask"], dtype=bool)
        reward = float(s["reward"])
        done = bool(s["done"])
        info = {
            "cast_success": bool(s["cast_success"]),
            "step_damage": float(s.get("step_damage", 0.0)),
            "advance_damage": float(s.get("advance_damage", 0.0)),
        }

        truncated = False
        terminated = done
        if done:
            # episode_dps 总是记录（监控用），无需额外 HTTP
            ep_dps = s.get("episode_dps")
            if ep_dps is not None:
                info["episode_dps"] = float(ep_dps)
                if self.baseline_dps:
                    bonus = (float(ep_dps) - self.baseline_dps) / max(self.baseline_dps, 1.0) * 100.0
                    reward += bonus
                    info["episode_bonus"] = bonus

        info["action_mask"] = self._last_mask.copy()
        return self._last_obs.copy(), reward, terminated, truncated, info

    def _advance(self) -> dict:
        adv = self._post(f"/api/rl/env/{self.session_id}/advance")
        self._last_obs = np.asarray(adv["obs"], dtype=np.float32)
        self._last_mask = np.asarray(adv["mask"], dtype=bool)
        return adv

    # MaskablePPO/自定义 PPO 取 mask 的统一接口
    def action_masks(self) -> np.ndarray:
        if self._last_mask is None:
            return np.ones(self.spec_meta.action_count, dtype=bool)
        return self._last_mask.copy()

    def macro_decision(self, macro_text: str, last_skill: str | None = None) -> int:
        """用宏对当前状态做一次决策（不一致分析用，不改变 env 状态）"""
        body = {"macro_text": macro_text, "last_skill": last_skill}
        resp = self._post(
            f"/api/rl/env/{self.session_id}/macro_decision", body
        )
        return int(resp["action"])

    def env_info(self) -> dict:
        return self._get(f"/api/rl/env/{self.session_id}/info")


# 与 backend env.rs 中的 BASELINE_PER_HIT 保持一致（reward 归一化基准）
BASELINE_PER_HIT = 500_000.0
