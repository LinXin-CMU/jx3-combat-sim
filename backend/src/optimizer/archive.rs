//! 自动存档：每次运行建独立目录，写 meta/baseline/progress/milestones/topN
//!
//! 目录结构：
//! ```text
//! backend/runs/{run_id}/
//!   meta.json        运行元信息（phase/start_time/attrs/ga_params/baseline_dps）
//!   baseline.json    用户起点 LoopConfig 原样保存
//!   progress.jsonl   每代一行 {gen, best, avg, worst, ...}
//!   milestones/      每次 best 刷新就存一份完整 LoopConfig
//!     gen_000_dps_XXXXXXX.json
//!   topN/            末代去重后的前 N 个
//!     rank_01_dps_XXXXXXX.json
//! ```

use std::fs::{File, OpenOptions, create_dir_all};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::loop_config::LoopConfig;

pub struct ArchiveWriter {
    pub run_id: String,
    pub root: PathBuf,
    progress: File,
}

impl ArchiveWriter {
    pub fn new<T: Serialize>(run_id: String, meta: &T, baseline: &LoopConfig) -> std::io::Result<Self> {
        let root = resolve_runs_root().join(&run_id);
        create_dir_all(root.join("milestones"))?;
        create_dir_all(root.join("topN"))?;

        write_json_pretty(&root.join("meta.json"), meta)?;
        write_json_pretty(&root.join("baseline.json"), baseline)?;

        let progress = OpenOptions::new()
            .create(true)
            .append(true)
            .open(root.join("progress.jsonl"))?;

        Ok(Self { run_id, root, progress })
    }

    pub fn append_progress<T: Serialize>(&mut self, entry: &T) -> std::io::Result<()> {
        let line = serde_json::to_string(entry)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        writeln!(self.progress, "{}", line)?;
        self.progress.flush()
    }

    pub fn write_milestone(&self, gen: usize, dps: f64, cfg: &LoopConfig) -> std::io::Result<String> {
        let name = format!("gen_{:04}_dps_{}.json", gen, dps.round() as i64);
        let rel = format!("milestones/{}", name);
        write_json_pretty(&self.root.join(&rel), cfg)?;
        Ok(rel)
    }

    pub fn write_topn(&self, rank: usize, dps: f64, cfg: &LoopConfig) -> std::io::Result<String> {
        let name = format!("rank_{:02}_dps_{}.json", rank, dps.round() as i64);
        let rel = format!("topN/{}", name);
        write_json_pretty(&self.root.join(&rel), cfg)?;
        Ok(rel)
    }
}

fn resolve_runs_root() -> PathBuf {
    // 优先当前目录（开发时 cargo run 的 CWD 是 backend/），否则用 backend/runs
    let a = Path::new("./runs");
    if a.exists() || Path::new("./Cargo.toml").exists() {
        return a.to_path_buf();
    }
    PathBuf::from("backend/runs")
}

fn write_json_pretty<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    if let Some(parent) = path.parent() { create_dir_all(parent)?; }
    let text = serde_json::to_string_pretty(value)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(path, text)
}

/// 列出所有已归档的运行（按 run_id 逆序）
pub fn list_runs() -> std::io::Result<Vec<String>> {
    let root = resolve_runs_root();
    if !root.exists() { return Ok(Vec::new()); }
    let mut ids = Vec::new();
    for entry in std::fs::read_dir(&root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            if let Some(name) = entry.file_name().to_str() {
                ids.push(name.to_string());
            }
        }
    }
    ids.sort_by(|a, b| b.cmp(a));
    Ok(ids)
}

/// 读取某次运行的 meta（给 /api/optimizer/runs/:id 用）
pub fn read_meta(run_id: &str) -> std::io::Result<serde_json::Value> {
    let root = resolve_runs_root().join(run_id);
    let text = std::fs::read_to_string(root.join("meta.json"))?;
    serde_json::from_str(&text)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

pub fn read_archive_file(run_id: &str, rel: &str) -> std::io::Result<String> {
    let root = resolve_runs_root().join(run_id);
    // 简单路径安全检查：不允许 .. / 绝对路径
    if rel.contains("..") || rel.starts_with('/') || rel.starts_with('\\') {
        return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "invalid path"));
    }
    std::fs::read_to_string(root.join(rel))
}

/// 列出 milestones / topN 文件
pub fn list_archive_subdir(run_id: &str, subdir: &str) -> std::io::Result<Vec<String>> {
    let root = resolve_runs_root().join(run_id).join(subdir);
    if !root.exists() { return Ok(Vec::new()); }
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&root)? {
        let entry = entry?;
        if let Some(name) = entry.file_name().to_str() {
            files.push(name.to_string());
        }
    }
    files.sort();
    Ok(files)
}
