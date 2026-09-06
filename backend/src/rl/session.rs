//! RL env 会话管理：单进程内持有多个 CombatEnv 实例，按 session_id 路由。
//!
//! 用途：Python 训练侧通过 HTTP 调 `/api/rl/env/{id}/step`，每个并行 env 对应一个 session。
//! 锁粒度：每个 session 一个 `Mutex<CombatEnv>`，全局只用 RwLock 保护 HashMap 本身。

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{Mutex, RwLock};

use super::env::CombatEnv;

pub struct RlSessions {
    inner: RwLock<HashMap<String, Arc<Mutex<CombatEnv>>>>,
    counter: std::sync::atomic::AtomicU64,
}

impl RlSessions {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: RwLock::new(HashMap::new()),
            counter: std::sync::atomic::AtomicU64::new(0),
        })
    }

    pub async fn create(&self, env: CombatEnv) -> String {
        let n = self
            .counter
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let id = format!("env-{}-{}", std::process::id(), n);
        let mut map = self.inner.write().await;
        map.insert(id.clone(), Arc::new(Mutex::new(env)));
        id
    }

    pub async fn get(&self, id: &str) -> Option<Arc<Mutex<CombatEnv>>> {
        let map = self.inner.read().await;
        map.get(id).cloned()
    }

    pub async fn remove(&self, id: &str) -> bool {
        let mut map = self.inner.write().await;
        map.remove(id).is_some()
    }

    pub async fn list(&self) -> Vec<String> {
        let map = self.inner.read().await;
        map.keys().cloned().collect()
    }
}
