"""PyTorch PPO + invalid action masking。

设计：
- ActorCritic：MLP 256-256，两个 head（policy logits、value）
- 动作采样前对非法动作 logit 置 -inf（softmax 后概率严格为 0）
- GAE(λ) 优势估计
- minibatch SGD + clip 目标
- gradient clip + 标准 PPO trick

CUDA：传入 device='cuda' 即可，所有 tensor/model 在该 device 上。
"""

from __future__ import annotations

from dataclasses import dataclass

import numpy as np
import torch
import torch.nn as nn
import torch.nn.functional as F
from torch.distributions import Categorical


# ─────────────────────────────────────────────────────────────────────────────
# 模型
# ─────────────────────────────────────────────────────────────────────────────

class ActorCritic(nn.Module):
    def __init__(self, obs_dim: int, action_dim: int, hidden: int = 256):
        super().__init__()
        self.shared = nn.Sequential(
            nn.Linear(obs_dim, hidden),
            nn.Tanh(),
            nn.Linear(hidden, hidden),
            nn.Tanh(),
        )
        self.policy_head = nn.Linear(hidden, action_dim)
        self.value_head = nn.Linear(hidden, 1)
        self._init_weights()

    def _init_weights(self):
        for m in self.modules():
            if isinstance(m, nn.Linear):
                nn.init.orthogonal_(m.weight, gain=np.sqrt(2))
                nn.init.zeros_(m.bias)
        # policy/value head 用更小 gain
        nn.init.orthogonal_(self.policy_head.weight, gain=0.01)
        nn.init.orthogonal_(self.value_head.weight, gain=1.0)

    def forward(self, obs: torch.Tensor):
        h = self.shared(obs)
        return self.policy_head(h), self.value_head(h).squeeze(-1)

    def get_action_and_value(
        self,
        obs: torch.Tensor,
        mask: torch.Tensor,
        action: torch.Tensor | None = None,
        deterministic: bool = False,
    ):
        logits, value = self.forward(obs)
        # 非法动作 -> -inf
        masked_logits = logits.masked_fill(~mask, float("-inf"))
        # 防止全部非法（理论上不会，等待动作永远合法）
        if torch.isinf(masked_logits).all(dim=-1).any():
            raise RuntimeError("所有动作都被 mask 屏蔽了；检查 obs/mask 的对齐")
        dist = Categorical(logits=masked_logits)
        if action is None:
            action = dist.probs.argmax(dim=-1) if deterministic else dist.sample()
        log_prob = dist.log_prob(action)
        entropy = dist.entropy()
        return action, log_prob, entropy, value


# ─────────────────────────────────────────────────────────────────────────────
# Rollout 缓冲
# ─────────────────────────────────────────────────────────────────────────────

@dataclass
class RolloutBuffer:
    n_steps: int
    n_envs: int
    obs_dim: int
    action_dim: int
    device: str

    def __post_init__(self):
        s, n, d, a = self.n_steps, self.n_envs, self.obs_dim, self.action_dim
        dev = self.device
        self.obs = torch.zeros((s, n, d), dtype=torch.float32, device=dev)
        self.masks = torch.zeros((s, n, a), dtype=torch.bool, device=dev)
        self.actions = torch.zeros((s, n), dtype=torch.long, device=dev)
        self.log_probs = torch.zeros((s, n), dtype=torch.float32, device=dev)
        self.rewards = torch.zeros((s, n), dtype=torch.float32, device=dev)
        self.dones = torch.zeros((s, n), dtype=torch.float32, device=dev)
        self.values = torch.zeros((s, n), dtype=torch.float32, device=dev)

    def add(
        self,
        step: int,
        obs: np.ndarray,
        mask: np.ndarray,
        action: np.ndarray,
        log_prob: np.ndarray,
        reward: np.ndarray,
        done: np.ndarray,
        value: np.ndarray,
    ):
        self.obs[step] = torch.as_tensor(obs, device=self.device)
        self.masks[step] = torch.as_tensor(mask, device=self.device, dtype=torch.bool)
        self.actions[step] = torch.as_tensor(action, device=self.device, dtype=torch.long)
        self.log_probs[step] = torch.as_tensor(log_prob, device=self.device)
        self.rewards[step] = torch.as_tensor(reward, device=self.device)
        self.dones[step] = torch.as_tensor(done, device=self.device, dtype=torch.float32)
        self.values[step] = torch.as_tensor(value, device=self.device)

    def compute_gae(
        self,
        last_values: torch.Tensor,
        last_done: torch.Tensor,
        gamma: float,
        gae_lambda: float,
    ):
        advantages = torch.zeros_like(self.rewards)
        last_adv = torch.zeros(self.n_envs, device=self.device)
        for t in reversed(range(self.n_steps)):
            if t == self.n_steps - 1:
                next_nonterm = 1.0 - last_done
                next_value = last_values
            else:
                next_nonterm = 1.0 - self.dones[t + 1]
                next_value = self.values[t + 1]
            delta = (
                self.rewards[t] + gamma * next_value * next_nonterm - self.values[t]
            )
            last_adv = delta + gamma * gae_lambda * next_nonterm * last_adv
            advantages[t] = last_adv
        returns = advantages + self.values
        return advantages, returns


# ─────────────────────────────────────────────────────────────────────────────
# PPO trainer
# ─────────────────────────────────────────────────────────────────────────────

@dataclass
class PpoConfig:
    n_steps: int = 2048
    n_envs: int = 8
    n_epochs: int = 10
    minibatch_size: int = 512
    learning_rate: float = 3e-4
    gamma: float = 0.999
    gae_lambda: float = 0.95
    clip_range: float = 0.2
    ent_coef: float = 0.01
    vf_coef: float = 0.5
    max_grad_norm: float = 0.5
    target_kl: float | None = 0.02  # None = 不早停


class PpoTrainer:
    def __init__(
        self,
        obs_dim: int,
        action_dim: int,
        cfg: PpoConfig,
        device: str = "cuda",
        hidden: int = 256,
    ):
        self.cfg = cfg
        self.device = device
        self.model = ActorCritic(obs_dim, action_dim, hidden).to(device)
        self.optimizer = torch.optim.Adam(
            self.model.parameters(), lr=cfg.learning_rate, eps=1e-5
        )
        self.buffer = RolloutBuffer(
            cfg.n_steps, cfg.n_envs, obs_dim, action_dim, device
        )
        self.global_step = 0

    @torch.no_grad()
    def select_action(
        self, obs: np.ndarray, mask: np.ndarray, deterministic: bool = False
    ):
        obs_t = torch.as_tensor(obs, dtype=torch.float32, device=self.device)
        mask_t = torch.as_tensor(mask, dtype=torch.bool, device=self.device)
        if obs_t.dim() == 1:
            obs_t = obs_t.unsqueeze(0)
            mask_t = mask_t.unsqueeze(0)
        action, log_prob, _, value = self.model.get_action_and_value(
            obs_t, mask_t, deterministic=deterministic
        )
        return (
            action.cpu().numpy(),
            log_prob.cpu().numpy(),
            value.cpu().numpy(),
        )

    def update(self, last_obs: np.ndarray, last_mask: np.ndarray, last_done: np.ndarray) -> dict:
        # 估算最后状态价值
        with torch.no_grad():
            obs_t = torch.as_tensor(last_obs, dtype=torch.float32, device=self.device)
            mask_t = torch.as_tensor(last_mask, dtype=torch.bool, device=self.device)
            _, _, _, last_values = self.model.get_action_and_value(obs_t, mask_t)
        last_done_t = torch.as_tensor(last_done, dtype=torch.float32, device=self.device)

        advantages, returns = self.buffer.compute_gae(
            last_values, last_done_t, self.cfg.gamma, self.cfg.gae_lambda
        )

        # flatten
        b_obs = self.buffer.obs.reshape(-1, self.buffer.obs_dim)
        b_masks = self.buffer.masks.reshape(-1, self.buffer.action_dim)
        b_actions = self.buffer.actions.reshape(-1)
        b_log_probs = self.buffer.log_probs.reshape(-1)
        b_advantages = advantages.reshape(-1)
        b_returns = returns.reshape(-1)
        b_values = self.buffer.values.reshape(-1)

        n_samples = b_obs.size(0)
        idx = np.arange(n_samples)
        clipfracs, pg_losses, v_losses, ent_losses, kls = [], [], [], [], []

        for epoch in range(self.cfg.n_epochs):
            np.random.shuffle(idx)
            early_stop = False
            for start in range(0, n_samples, self.cfg.minibatch_size):
                end = start + self.cfg.minibatch_size
                mb = idx[start:end]
                mb_t = torch.as_tensor(mb, device=self.device, dtype=torch.long)

                _, new_log_prob, entropy, new_value = self.model.get_action_and_value(
                    b_obs[mb_t], b_masks[mb_t], action=b_actions[mb_t]
                )
                logratio = new_log_prob - b_log_probs[mb_t]
                ratio = logratio.exp()

                with torch.no_grad():
                    approx_kl = ((ratio - 1) - logratio).mean()
                    clipfracs.append(
                        ((ratio - 1.0).abs() > self.cfg.clip_range).float().mean().item()
                    )

                mb_adv = b_advantages[mb_t]
                mb_adv = (mb_adv - mb_adv.mean()) / (mb_adv.std() + 1e-8)

                pg_loss1 = -mb_adv * ratio
                pg_loss2 = -mb_adv * torch.clamp(
                    ratio, 1.0 - self.cfg.clip_range, 1.0 + self.cfg.clip_range
                )
                pg_loss = torch.max(pg_loss1, pg_loss2).mean()

                # value clip
                v_clipped = b_values[mb_t] + torch.clamp(
                    new_value - b_values[mb_t],
                    -self.cfg.clip_range,
                    self.cfg.clip_range,
                )
                v_loss = 0.5 * torch.max(
                    (new_value - b_returns[mb_t]).pow(2),
                    (v_clipped - b_returns[mb_t]).pow(2),
                ).mean()

                ent_loss = entropy.mean()
                loss = pg_loss + self.cfg.vf_coef * v_loss - self.cfg.ent_coef * ent_loss

                self.optimizer.zero_grad()
                loss.backward()
                nn.utils.clip_grad_norm_(self.model.parameters(), self.cfg.max_grad_norm)
                self.optimizer.step()

                pg_losses.append(pg_loss.item())
                v_losses.append(v_loss.item())
                ent_losses.append(ent_loss.item())
                kls.append(approx_kl.item())

            if self.cfg.target_kl is not None and np.mean(kls[-len(idx) // self.cfg.minibatch_size :]) > self.cfg.target_kl:
                early_stop = True
            if early_stop:
                break

        return {
            "loss/policy": float(np.mean(pg_losses)),
            "loss/value": float(np.mean(v_losses)),
            "loss/entropy": float(np.mean(ent_losses)),
            "stats/approx_kl": float(np.mean(kls)),
            "stats/clipfrac": float(np.mean(clipfracs)),
            "stats/advantages_mean": float(b_advantages.mean().item()),
            "stats/returns_mean": float(b_returns.mean().item()),
        }

    def save(self, path: str, extra: dict | None = None):
        ckpt = {
            "model": self.model.state_dict(),
            "optimizer": self.optimizer.state_dict(),
            "global_step": self.global_step,
            "cfg": self.cfg,
        }
        if extra:
            ckpt["extra"] = extra
        torch.save(ckpt, path)

    def load(self, path: str, load_optimizer: bool = True) -> dict:
        """加载 ckpt，返回 extra 字典（如果保存时附了的话）

        ckpt 由我们自己的 trainer.save 生成（信任源），用 weights_only=False
        以便反序列化 PpoConfig dataclass。
        """
        ckpt = torch.load(path, map_location=self.device, weights_only=False)
        self.model.load_state_dict(ckpt["model"])
        if load_optimizer and "optimizer" in ckpt:
            self.optimizer.load_state_dict(ckpt["optimizer"])
        if "global_step" in ckpt:
            self.global_step = int(ckpt["global_step"])
        return ckpt.get("extra", {})
