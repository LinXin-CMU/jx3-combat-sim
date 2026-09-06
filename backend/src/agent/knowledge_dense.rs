use fastembed::{
    EmbeddingModel, InitOptionsUserDefined, Pooling, TextEmbedding, TextInitOptions,
    TokenizerFiles, UserDefinedEmbeddingModel,
};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const DENSE_MODEL_ID: &str = "BAAI/bge-small-zh-v1.5";
pub const DENSE_DIMENSION: usize = 512;
pub const DENSE_MIN_SIMILARITY: f32 = 0.45;
const CACHE_MAGIC: &[u8; 8] = b"JX3EMB01";
const MAX_CACHE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_LOCAL_MODEL_BYTES: u64 = 256 * 1024 * 1024;
const MAX_LOCAL_TOKENIZER_BYTES: u64 = 16 * 1024 * 1024;
const EMBED_BATCH_SIZE: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DenseCacheState {
    Hit,
    Built,
}

impl DenseCacheState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hit => "hit",
            Self::Built => "built",
        }
    }
}

#[derive(Debug)]
pub struct DenseError {
    pub code: &'static str,
    detail: String,
}

impl DenseError {
    fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for DenseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.detail)
    }
}

impl std::error::Error for DenseError {}

#[derive(Debug, Clone, Copy)]
pub struct DenseHit {
    pub chunk_index: usize,
    pub similarity: f32,
}

pub struct DenseKnowledgeIndex {
    model: Mutex<TextEmbedding>,
    embeddings: Vec<Vec<f32>>,
    cache_state: DenseCacheState,
}

impl fmt::Debug for DenseKnowledgeIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DenseKnowledgeIndex")
            .field("model", &DENSE_MODEL_ID)
            .field("dimension", &DENSE_DIMENSION)
            .field("embedding_count", &self.embeddings.len())
            .field("cache_state", &self.cache_state)
            .finish()
    }
}

impl DenseKnowledgeIndex {
    pub fn load_or_build(
        corpus_hash: &str,
        texts: &[String],
        cache_root: &Path,
    ) -> Result<Self, DenseError> {
        fs::create_dir_all(cache_root)
            .map_err(|error| DenseError::new("dense_cache_create_failed", error.to_string()))?;
        let model_cache = cache_root.join("fastembed");
        fs::create_dir_all(&model_cache).map_err(|error| {
            DenseError::new("dense_model_cache_create_failed", error.to_string())
        })?;

        eprintln!("[agent][knowledge] 正在加载本地向量模型 {DENSE_MODEL_ID}");
        let mut model = load_embedding_model(&model_cache)?;

        let cache_path = cache_path(cache_root, corpus_hash);
        let (embeddings, cache_state) = match read_cache(&cache_path, corpus_hash, texts.len()) {
            Ok(embeddings) => (embeddings, DenseCacheState::Hit),
            Err(error) if error.code == "dense_cache_miss" => {
                eprintln!(
                    "[agent][knowledge] 向量缓存未命中，正在为 {} 个分块建立索引",
                    texts.len()
                );
                let mut embeddings =
                    model
                        .embed(texts, Some(EMBED_BATCH_SIZE))
                        .map_err(|error| {
                            DenseError::new("dense_index_build_failed", error.to_string())
                        })?;
                validate_and_normalize(&mut embeddings, texts.len())?;
                write_cache(&cache_path, corpus_hash, &embeddings)?;
                (embeddings, DenseCacheState::Built)
            }
            Err(error) => {
                eprintln!(
                    "[agent][knowledge] 向量缓存不可用（{}），将重建缓存",
                    error.code
                );
                let mut embeddings =
                    model
                        .embed(texts, Some(EMBED_BATCH_SIZE))
                        .map_err(|error| {
                            DenseError::new("dense_index_build_failed", error.to_string())
                        })?;
                validate_and_normalize(&mut embeddings, texts.len())?;
                write_cache(&cache_path, corpus_hash, &embeddings)?;
                (embeddings, DenseCacheState::Built)
            }
        };

        Ok(Self {
            model: Mutex::new(model),
            embeddings,
            cache_state,
        })
    }

    pub fn cache_state(&self) -> DenseCacheState {
        self.cache_state
    }

    pub fn rank(
        &self,
        query: &str,
        eligible_chunks: &[usize],
    ) -> Result<Vec<DenseHit>, DenseError> {
        let mut model = self
            .model
            .lock()
            .map_err(|_| DenseError::new("dense_model_lock_failed", "model lock poisoned"))?;
        let mut query_embeddings = model
            .embed([query], Some(1))
            .map_err(|error| DenseError::new("dense_query_failed", error.to_string()))?;
        validate_and_normalize(&mut query_embeddings, 1)?;
        let query_embedding = &query_embeddings[0];
        let mut hits = eligible_chunks
            .iter()
            .filter_map(|&chunk_index| {
                let embedding = self.embeddings.get(chunk_index)?;
                let similarity = dot(query_embedding, embedding);
                (similarity.is_finite() && similarity >= DENSE_MIN_SIMILARITY).then_some(DenseHit {
                    chunk_index,
                    similarity,
                })
            })
            .collect::<Vec<_>>();
        hits.sort_by(|left, right| {
            right
                .similarity
                .partial_cmp(&left.similarity)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.chunk_index.cmp(&right.chunk_index))
        });
        Ok(hits)
    }

}

fn load_embedding_model(model_cache: &Path) -> Result<TextEmbedding, DenseError> {
    if let Ok(snapshot) = local_model_snapshot(model_cache) {
        eprintln!("[agent][knowledge] 使用已缓存的本地 BGE 模型文件");
        return load_local_embedding_model(&snapshot);
    }
    let options = TextInitOptions::new(EmbeddingModel::BGESmallZHV15)
        .with_cache_dir(model_cache.to_path_buf())
        .with_show_download_progress(false)
        .with_intra_threads(4);
    TextEmbedding::try_new(options)
        .map_err(|error| DenseError::new("dense_model_init_failed", error.to_string()))
}

fn local_model_snapshot(model_cache: &Path) -> Result<PathBuf, DenseError> {
    let snapshots = model_cache
        .join("models--Xenova--bge-small-zh-v1.5")
        .join("snapshots");
    let mut candidates = fs::read_dir(&snapshots)
        .map_err(|error| DenseError::new("dense_local_model_absent", error.to_string()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .filter(|path| {
            [
                "onnx/model.onnx",
                "tokenizer.json",
                "config.json",
                "special_tokens_map.json",
                "tokenizer_config.json",
            ]
            .iter()
            .all(|relative| path.join(relative).is_file())
        })
        .collect::<Vec<_>>();
    candidates.sort();
    candidates
        .pop()
        .ok_or_else(|| DenseError::new("dense_local_model_absent", "complete snapshot not found"))
}

fn read_bounded_model_file(
    path: &Path,
    byte_limit: u64,
    code: &'static str,
) -> Result<Vec<u8>, DenseError> {
    let metadata = fs::metadata(path).map_err(|error| DenseError::new(code, error.to_string()))?;
    if metadata.len() == 0 || metadata.len() > byte_limit {
        return Err(DenseError::new(code, "cached model file size is invalid"));
    }
    fs::read(path).map_err(|error| DenseError::new(code, error.to_string()))
}

fn load_local_embedding_model(snapshot: &Path) -> Result<TextEmbedding, DenseError> {
    let tokenizer_files = TokenizerFiles {
        tokenizer_file: read_bounded_model_file(
            &snapshot.join("tokenizer.json"),
            MAX_LOCAL_TOKENIZER_BYTES,
            "dense_local_tokenizer_invalid",
        )?,
        config_file: read_bounded_model_file(
            &snapshot.join("config.json"),
            MAX_LOCAL_TOKENIZER_BYTES,
            "dense_local_config_invalid",
        )?,
        special_tokens_map_file: read_bounded_model_file(
            &snapshot.join("special_tokens_map.json"),
            MAX_LOCAL_TOKENIZER_BYTES,
            "dense_local_special_tokens_invalid",
        )?,
        tokenizer_config_file: read_bounded_model_file(
            &snapshot.join("tokenizer_config.json"),
            MAX_LOCAL_TOKENIZER_BYTES,
            "dense_local_tokenizer_config_invalid",
        )?,
    };
    let model = UserDefinedEmbeddingModel::new(
        read_bounded_model_file(
            &snapshot.join("onnx").join("model.onnx"),
            MAX_LOCAL_MODEL_BYTES,
            "dense_local_onnx_invalid",
        )?,
        tokenizer_files,
    )
    .with_pooling(Pooling::Mean);
    TextEmbedding::try_new_from_user_defined(
        model,
        InitOptionsUserDefined::default().with_intra_threads(4),
    )
    .map_err(|error| DenseError::new("dense_local_model_init_failed", error.to_string()))
}

fn cache_path(cache_root: &Path, corpus_hash: &str) -> PathBuf {
    let short_hash = &corpus_hash[..corpus_hash.len().min(16)];
    cache_root.join(format!("bge-small-zh-v1.5-{short_hash}.bin"))
}

fn read_cache(
    path: &Path,
    corpus_hash: &str,
    expected_count: usize,
) -> Result<Vec<Vec<f32>>, DenseError> {
    if !path.is_file() {
        return Err(DenseError::new("dense_cache_miss", "cache file absent"));
    }
    let metadata = fs::metadata(path)
        .map_err(|error| DenseError::new("dense_cache_read_failed", error.to_string()))?;
    if metadata.len() > MAX_CACHE_BYTES {
        return Err(DenseError::new(
            "dense_cache_invalid",
            "cache exceeds size limit",
        ));
    }
    let mut file = File::open(path)
        .map_err(|error| DenseError::new("dense_cache_read_failed", error.to_string()))?;
    let mut magic = [0_u8; 8];
    file.read_exact(&mut magic)
        .map_err(|error| DenseError::new("dense_cache_invalid", error.to_string()))?;
    if &magic != CACHE_MAGIC {
        return Err(DenseError::new("dense_cache_invalid", "magic mismatch"));
    }
    let count = read_u32(&mut file)? as usize;
    let dimension = read_u32(&mut file)? as usize;
    let stored_corpus = read_string(&mut file, 128)?;
    let stored_model = read_string(&mut file, 128)?;
    if count != expected_count
        || dimension != DENSE_DIMENSION
        || stored_corpus != corpus_hash
        || stored_model != DENSE_MODEL_ID
    {
        return Err(DenseError::new(
            "dense_cache_invalid",
            "cache identity mismatch",
        ));
    }
    let float_count = count
        .checked_mul(dimension)
        .ok_or_else(|| DenseError::new("dense_cache_invalid", "dimension overflow"))?;
    let mut raw = vec![0_u8; float_count * 4];
    file.read_exact(&mut raw)
        .map_err(|error| DenseError::new("dense_cache_invalid", error.to_string()))?;
    let mut trailing = [0_u8; 1];
    if file.read(&mut trailing).unwrap_or(1) != 0 {
        return Err(DenseError::new(
            "dense_cache_invalid",
            "unexpected trailing bytes",
        ));
    }
    let mut embeddings = raw
        .chunks_exact(4)
        .map(|bytes| f32::from_le_bytes(bytes.try_into().expect("four bytes")))
        .collect::<Vec<_>>()
        .chunks_exact(dimension)
        .map(|row| row.to_vec())
        .collect::<Vec<_>>();
    validate_and_normalize(&mut embeddings, expected_count)?;
    Ok(embeddings)
}

fn write_cache(path: &Path, corpus_hash: &str, embeddings: &[Vec<f32>]) -> Result<(), DenseError> {
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| DenseError::new("dense_cache_write_failed", error.to_string()))?;
    let write_result = (|| -> Result<(), std::io::Error> {
        file.write_all(CACHE_MAGIC)?;
        file.write_all(&(embeddings.len() as u32).to_le_bytes())?;
        file.write_all(&(DENSE_DIMENSION as u32).to_le_bytes())?;
        write_string(&mut file, corpus_hash)?;
        write_string(&mut file, DENSE_MODEL_ID)?;
        for embedding in embeddings {
            for value in embedding {
                file.write_all(&value.to_le_bytes())?;
            }
        }
        file.sync_all()
    })();
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temporary);
        return Err(DenseError::new(
            "dense_cache_write_failed",
            error.to_string(),
        ));
    }
    if path.exists() {
        let _ = fs::remove_file(&temporary);
        return Ok(());
    }
    fs::rename(&temporary, path).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        DenseError::new("dense_cache_write_failed", error.to_string())
    })
}

fn validate_and_normalize(
    embeddings: &mut [Vec<f32>],
    expected_count: usize,
) -> Result<(), DenseError> {
    if embeddings.len() != expected_count {
        return Err(DenseError::new(
            "dense_vector_invalid",
            "vector count mismatch",
        ));
    }
    for embedding in embeddings {
        if embedding.len() != DENSE_DIMENSION || embedding.iter().any(|value| !value.is_finite()) {
            return Err(DenseError::new(
                "dense_vector_invalid",
                "invalid vector shape or value",
            ));
        }
        let norm = embedding
            .iter()
            .map(|value| value * value)
            .sum::<f32>()
            .sqrt();
        if !norm.is_finite() || norm <= f32::EPSILON {
            return Err(DenseError::new("dense_vector_invalid", "zero vector"));
        }
        for value in embedding {
            *value /= norm;
        }
    }
    Ok(())
}

fn dot(left: &[f32], right: &[f32]) -> f32 {
    left.iter()
        .zip(right)
        .map(|(left, right)| left * right)
        .sum()
}

fn read_u32(file: &mut File) -> Result<u32, DenseError> {
    let mut bytes = [0_u8; 4];
    file.read_exact(&mut bytes)
        .map_err(|error| DenseError::new("dense_cache_invalid", error.to_string()))?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_string(file: &mut File, max_length: usize) -> Result<String, DenseError> {
    let length = read_u32(file)? as usize;
    if length > max_length {
        return Err(DenseError::new(
            "dense_cache_invalid",
            "string exceeds limit",
        ));
    }
    let mut bytes = vec![0_u8; length];
    file.read_exact(&mut bytes)
        .map_err(|error| DenseError::new("dense_cache_invalid", error.to_string()))?;
    String::from_utf8(bytes)
        .map_err(|error| DenseError::new("dense_cache_invalid", error.to_string()))
}

fn write_string(file: &mut File, value: &str) -> Result<(), std::io::Error> {
    file.write_all(&(value.len() as u32).to_le_bytes())?;
    file.write_all(value.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_path_does_not_expose_full_corpus_identity() {
        let root = Path::new("cache");
        let path = cache_path(root, &"a".repeat(64));
        assert_eq!(path, root.join("bge-small-zh-v1.5-aaaaaaaaaaaaaaaa.bin"));
    }

    #[test]
    fn normalization_rejects_invalid_vectors() {
        let mut zero = vec![vec![0.0; DENSE_DIMENSION]];
        assert_eq!(
            validate_and_normalize(&mut zero, 1).unwrap_err().code,
            "dense_vector_invalid"
        );
    }
}
