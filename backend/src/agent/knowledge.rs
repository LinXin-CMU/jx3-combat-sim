use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::env;
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use super::domain::{
    derive_domain_claims, domain_index_hash, domain_relations, DomainChunkContext, DomainClaimV1,
    DomainRelationV1,
};
use super::knowledge_dense::{DenseKnowledgeIndex, DENSE_MODEL_ID};
use crate::GameVersion;

pub const KNOWLEDGE_INDEX_SCHEMA_V1: &str = "agent-knowledge-index/v1";
pub const KNOWLEDGE_SEARCH_SCHEMA_V1: &str = "agent-knowledge-search/v1";
pub const KNOWLEDGE_ROOT_ENV: &str = "JX3_KNOWLEDGE_ROOT";
pub const KNOWLEDGE_RETRIEVAL_ENV: &str = "JX3_KNOWLEDGE_RETRIEVAL";
pub const KNOWLEDGE_CACHE_ENV: &str = "JX3_KNOWLEDGE_CACHE_DIR";
const MANIFEST_FILE: &str = "_migration-manifest.json";
const MAX_DOCUMENTS: usize = 5_000;
const MAX_DOCUMENT_BYTES: u64 = 8 * 1024 * 1024;
const MAX_CORPUS_BYTES: u64 = 64 * 1024 * 1024;
const MAX_QUERY_CHARACTERS: usize = 200;
pub const MAX_KNOWLEDGE_RESULTS: usize = 8;
const CHUNK_CHARACTERS: usize = 1_200;
const CHUNK_OVERLAP: usize = 120;
const SNIPPET_CHARACTERS: usize = 420;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KnowledgeIndexError {
    NotConfigured,
    Io(String),
    InvalidManifest(String),
    UnsafePath(String),
    CorpusLimit(&'static str),
    InvalidQuery(&'static str),
    UnknownSeason(String),
    UnknownCategory(String),
    VersionConflict { requested: String, current: String },
    CrossVersionIntentRequired,
}

impl fmt::Display for KnowledgeIndexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotConfigured => write!(f, "{KNOWLEDGE_ROOT_ENV} is not configured"),
            Self::Io(error) => write!(f, "knowledge I/O error: {error}"),
            Self::InvalidManifest(error) => write!(f, "invalid knowledge manifest: {error}"),
            Self::UnsafePath(path) => write!(f, "unsafe knowledge path: {path}"),
            Self::CorpusLimit(limit) => write!(f, "knowledge corpus exceeds {limit}"),
            Self::InvalidQuery(reason) => write!(f, "invalid knowledge query: {reason}"),
            Self::UnknownSeason(season) => write!(f, "unknown knowledge season: {season}"),
            Self::UnknownCategory(category) => write!(f, "unknown knowledge category: {category}"),
            Self::VersionConflict { requested, current } => {
                write!(
                    f,
                    "query requests {requested}, but current scope is {current}"
                )
            }
            Self::CrossVersionIntentRequired => {
                write!(
                    f,
                    "cross-version retrieval requires explicit comparison intent"
                )
            }
        }
    }
}

impl std::error::Error for KnowledgeIndexError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeQuality {
    FullText,
    MetadataOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeVersionMatch {
    CurrentExact,
    TestServerExact,
    HistoricalExplicit,
    CrossVersion,
    ReferenceOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum KnowledgeVersionScope {
    CurrentOnly,
    SpecificSeason { season: String },
    CrossVersion,
    ReferenceLookup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeClientScope {
    Flagship,
    Wujie,
    Any,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeMountScope {
    Fenshanjin,
    Tieguyi,
}

/// Product-level retrieval boundary. The calculator always has a selected
/// flagship mount, so an otherwise ambiguous question inherits that context.
/// Wujie is opt-in and never leaks into a flagship answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeAudience {
    pub client: KnowledgeClientScope,
    pub mount: Option<KnowledgeMountScope>,
}

impl KnowledgeAudience {
    pub fn from_question(question: &str, fallback_mount: Option<KnowledgeMountScope>) -> Self {
        let normalized = question.to_lowercase();
        let mentions_wujie = normalized.contains("无界")
            || normalized.contains("分山劲·悟")
            || normalized.contains("分山劲・悟")
            || normalized.contains("wujie");
        let mentions_flagship = normalized.contains("旗舰")
            || normalized.contains("端游")
            || normalized.contains("旗舰端");
        let mentions_fenshan = normalized.contains("分山劲") || normalized.contains("分山");
        let mentions_tiegu = normalized.contains("铁骨衣") || normalized.contains("铁骨");

        Self {
            client: match (mentions_wujie, mentions_flagship) {
                (true, true) => KnowledgeClientScope::Any,
                (true, false) => KnowledgeClientScope::Wujie,
                _ => KnowledgeClientScope::Flagship,
            },
            mount: match (mentions_fenshan, mentions_tiegu) {
                (true, false) => Some(KnowledgeMountScope::Fenshanjin),
                (false, true) => Some(KnowledgeMountScope::Tieguyi),
                _ => fallback_mount,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeVersionContext {
    pub game_version: GameVersion,
    pub current_season: &'static str,
    pub required_category: Option<&'static str>,
}

impl KnowledgeVersionContext {
    pub fn from_game_version(game_version: GameVersion) -> Self {
        match game_version {
            GameVersion::AnYingQianJi => Self {
                game_version,
                current_season: "暗影千机（2026）",
                required_category: None,
            },
            GameVersion::ShanHaiYuanLiu => Self {
                game_version,
                current_season: "山海源流（2025）",
                required_category: None,
            },
            GameVersion::AnYingQianJiTest => Self {
                game_version,
                current_season: "体服（2021-2025）",
                required_category: Some("130级"),
            },
        }
    }

    fn accepts_query_season(&self, season: &str) -> bool {
        season == self.current_season
            || (self.game_version == GameVersion::AnYingQianJiTest && season == "暗影千机（2026）")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeSearchQuery {
    pub query: String,
    pub version_scope: KnowledgeVersionScope,
    pub category: Option<String>,
    pub top_k: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeSearchResult {
    pub document_id: String,
    pub title: String,
    pub season: String,
    pub category: String,
    pub heading: String,
    pub snippet: String,
    pub source_url: String,
    pub yuque_url: String,
    pub source_site: String,
    pub source_updated_at: String,
    pub quality: KnowledgeQuality,
    pub fact_eligible: bool,
    pub version_match: KnowledgeVersionMatch,
    pub version_warning: Option<String>,
    pub document_hash: String,
    pub chunk_hash: String,
    pub score: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lexical_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dense_similarity: Option<f64>,
    pub exact_phrase_match: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reference_entities: Vec<KnowledgeReferenceEntity>,
    /// Curated, source-bound claims derived from this exact chunk. Claims never
    /// grant access to another document and retain the original source hashes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub domain_claims: Vec<DomainClaimV1>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub domain_relations: Vec<DomainRelationV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KnowledgeRetrievalInfo {
    pub requested_mode: String,
    pub active_mode: String,
    pub dense_model: Option<String>,
    pub cache_state: String,
    pub fallback_code: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeQueryIntent {
    Rotation,
    Macro,
    Equipment,
    Encounter,
    Mechanism,
    General,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeSourceRole {
    Whitepaper,
    Practical,
    Mechanism,
    Macro,
    General,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeSelectionInfo {
    pub strategy: String,
    pub intent: KnowledgeQueryIntent,
    pub confidence: String,
    pub candidate_documents: usize,
    pub returned_documents: usize,
    pub source_roles: Vec<KnowledgeSourceRole>,
    pub decision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KnowledgeReferenceEntity {
    pub relation: String,
    pub name: String,
    pub basis: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeSearchResponse {
    pub schema_version: String,
    pub corpus_hash: String,
    pub domain_index_hash: String,
    pub current_season: String,
    pub requested_scope: KnowledgeVersionScope,
    pub audience: KnowledgeAudience,
    pub retrieval: KnowledgeRetrievalInfo,
    pub selection: KnowledgeSelectionInfo,
    pub results: Vec<KnowledgeSearchResult>,
}

#[derive(Debug, Deserialize)]
struct MigrationManifest {
    entries: Vec<MigrationEntry>,
}

#[derive(Debug, Deserialize)]
struct MigrationEntry {
    title: String,
    season: String,
    category: String,
    kind: String,
    source: String,
    output: String,
    #[serde(default)]
    source_site: String,
    #[serde(default)]
    yuque_url: String,
    #[serde(default)]
    updated_at: String,
    #[serde(default)]
    source_updated_at: String,
    #[serde(default)]
    yuque_uuid: String,
    #[serde(default)]
    mirror_status: String,
    #[serde(default)]
    version_policy: KnowledgeVersionPolicy,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum KnowledgeVersionPolicy {
    #[default]
    TitleBound,
    RollingCurrent,
}

#[derive(Debug, Clone)]
struct KnowledgeChunk {
    document_id: String,
    title: String,
    season: String,
    category: String,
    heading: String,
    text: String,
    source_url: String,
    yuque_url: String,
    source_site: String,
    source_updated_at: String,
    quality: KnowledgeQuality,
    version_warning: Option<String>,
    document_hash: String,
    chunk_hash: String,
    term_frequency: HashMap<String, usize>,
    token_count: usize,
    domain_claims: Vec<DomainClaimV1>,
}

#[derive(Debug, Clone)]
pub struct KnowledgeIndex {
    schema_version: &'static str,
    corpus_hash: String,
    domain_index_hash: String,
    chunks: Vec<KnowledgeChunk>,
    document_frequency: HashMap<String, usize>,
    average_document_length: f64,
    seasons: BTreeSet<String>,
    categories: BTreeSet<String>,
    dense: Option<Arc<DenseKnowledgeIndex>>,
    requested_retrieval: String,
    dense_fallback_code: Option<String>,
}

impl KnowledgeIndex {
    pub fn from_env() -> Result<Self, KnowledgeIndexError> {
        let root = env::var_os(KNOWLEDGE_ROOT_ENV).ok_or(KnowledgeIndexError::NotConfigured)?;
        let mut index = Self::load(Path::new(&root))?;
        let requested = env::var(KNOWLEDGE_RETRIEVAL_ENV)
            .unwrap_or_else(|_| "embedded".to_string())
            .trim()
            .to_ascii_lowercase();
        index.requested_retrieval = requested.clone();
        match requested.as_str() {
            "bm25" => {}
            "embedded" => {
                let cache_root = env::var_os(KNOWLEDGE_CACHE_ENV)
                    .map(PathBuf::from)
                    .or_else(|| {
                        env::var_os("JX3_USERDATA_DIR")
                            .map(PathBuf::from)
                            .map(|root| root.join("knowledge_index").join("v1"))
                    })
                    .unwrap_or_else(|| {
                        PathBuf::from("userdata").join("knowledge_index").join("v1")
                    });
                let texts = index
                    .chunks
                    .iter()
                    .map(dense_document_text)
                    .collect::<Vec<_>>();
                // Contextual prefixes and curated relations affect embedding identity even
                // though the immutable Markdown corpus hash remains unchanged.
                let dense_identity = sha256(
                    format!(
                        "{}\0{}\0domain-context/v1",
                        index.corpus_hash, index.domain_index_hash
                    )
                    .as_bytes(),
                );
                match DenseKnowledgeIndex::load_or_build(&dense_identity, &texts, &cache_root) {
                    Ok(dense) => index.dense = Some(Arc::new(dense)),
                    Err(error) => {
                        eprintln!("[agent][knowledge] Embedded 初始化失败（{error}），已降级 BM25");
                        index.dense_fallback_code = Some(error.code.to_string());
                    }
                }
            }
            _ => {
                eprintln!(
                    "[agent][knowledge] 未知检索模式，已降级 BM25；请设置 {KNOWLEDGE_RETRIEVAL_ENV}=embedded|bm25"
                );
                index.dense_fallback_code = Some("invalid_retrieval_mode".to_string());
            }
        }
        Ok(index)
    }

    pub fn load(root: &Path) -> Result<Self, KnowledgeIndexError> {
        let root = root
            .canonicalize()
            .map_err(|error| KnowledgeIndexError::Io(error.to_string()))?;
        let manifest_path = root.join(MANIFEST_FILE);
        let raw_manifest = fs::read_to_string(&manifest_path)
            .map_err(|error| KnowledgeIndexError::Io(error.to_string()))?;
        let manifest: MigrationManifest = serde_json::from_str(&raw_manifest)
            .map_err(|error| KnowledgeIndexError::InvalidManifest(error.to_string()))?;
        if manifest.entries.len() > MAX_DOCUMENTS {
            return Err(KnowledgeIndexError::CorpusLimit("document limit"));
        }

        let mut entries = manifest.entries;
        entries.sort_by(|left, right| left.output.cmp(&right.output));
        let mut chunks = Vec::new();
        let mut corpus_bytes = 0_u64;
        let mut corpus_hasher = Sha256::new();
        let mut seasons = BTreeSet::new();
        let mut categories = BTreeSet::new();

        for entry in entries {
            let quality = match entry.kind.as_str() {
                "yuque_document" => KnowledgeQuality::FullText,
                "external_mirror" if entry.mirror_status == "full" => KnowledgeQuality::FullText,
                "external_mirror" if entry.mirror_status == "metadata_only" => {
                    KnowledgeQuality::MetadataOnly
                }
                _ => continue,
            };
            let relative = safe_relative_path(&entry.output)?;
            let document_path = root.join(relative);
            let canonical = document_path
                .canonicalize()
                .map_err(|error| KnowledgeIndexError::Io(error.to_string()))?;
            if !canonical.starts_with(&root) {
                return Err(KnowledgeIndexError::UnsafePath(entry.output));
            }
            let metadata = fs::metadata(&canonical)
                .map_err(|error| KnowledgeIndexError::Io(error.to_string()))?;
            if metadata.len() > MAX_DOCUMENT_BYTES {
                return Err(KnowledgeIndexError::CorpusLimit("per-document byte limit"));
            }
            corpus_bytes = corpus_bytes.saturating_add(metadata.len());
            if corpus_bytes > MAX_CORPUS_BYTES {
                return Err(KnowledgeIndexError::CorpusLimit("total byte limit"));
            }
            let markdown = fs::read_to_string(&canonical)
                .map_err(|error| KnowledgeIndexError::Io(error.to_string()))?;
            let body = strip_frontmatter(&markdown);
            let searchable = match quality {
                KnowledgeQuality::FullText => clean_markdown_for_index(body),
                KnowledgeQuality::MetadataOnly => {
                    format!("{} {} {}", entry.title, entry.season, entry.category)
                }
            };
            let document_hash = sha256(searchable.as_bytes());
            let document_id = if entry.yuque_uuid.trim().is_empty() {
                sha256(entry.output.as_bytes())
            } else {
                entry.yuque_uuid.clone()
            };
            let source_site = if entry.source_site.trim().is_empty() {
                source_host(&entry.source)
            } else {
                entry.source_site.clone()
            };
            let yuque_url = if entry.yuque_url.trim().is_empty() {
                entry.source.clone()
            } else {
                entry.yuque_url.clone()
            };
            let source_updated_at = if entry.source_updated_at.trim().is_empty() {
                entry.updated_at.clone()
            } else {
                entry.source_updated_at.clone()
            };
            let version_warning =
                title_season_warning(&entry.title, &entry.season, entry.version_policy);
            seasons.insert(entry.season.clone());
            categories.insert(entry.category.clone());
            corpus_hasher.update(entry.output.as_bytes());
            corpus_hasher.update([0]);
            corpus_hasher.update(document_hash.as_bytes());
            corpus_hasher.update([0]);
            corpus_hasher.update(entry.season.as_bytes());
            corpus_hasher.update([0]);
            corpus_hasher.update(entry.category.as_bytes());
            corpus_hasher.update([0]);
            corpus_hasher.update(match entry.version_policy {
                KnowledgeVersionPolicy::TitleBound => b"title_bound".as_slice(),
                KnowledgeVersionPolicy::RollingCurrent => b"rolling_current".as_slice(),
            });
            corpus_hasher.update([0]);

            for (heading, text) in split_markdown_chunks(&searchable) {
                let chunk_hash = sha256(text.as_bytes());
                let mut tokens = lexical_tokens(&text);
                let title_tokens = lexical_tokens(&entry.title);
                let heading_tokens = lexical_tokens(&heading);
                for _ in 0..3 {
                    tokens.extend(title_tokens.iter().cloned());
                }
                for _ in 0..2 {
                    tokens.extend(heading_tokens.iter().cloned());
                }
                tokens.extend(lexical_tokens(&entry.category));
                tokens.extend(lexical_tokens(&entry.season));
                let mut term_frequency = HashMap::new();
                for token in tokens {
                    *term_frequency.entry(token).or_insert(0) += 1;
                }
                let token_count = term_frequency.values().sum();
                chunks.push(KnowledgeChunk {
                    document_id: document_id.clone(),
                    title: entry.title.clone(),
                    season: entry.season.clone(),
                    category: entry.category.clone(),
                    heading,
                    text,
                    source_url: entry.source.clone(),
                    yuque_url: yuque_url.clone(),
                    source_site: source_site.clone(),
                    source_updated_at: source_updated_at.clone(),
                    quality,
                    version_warning: version_warning.clone(),
                    document_hash: document_hash.clone(),
                    chunk_hash,
                    term_frequency,
                    token_count,
                    domain_claims: Vec::new(),
                });
            }
        }

        for chunk in &mut chunks {
            chunk.domain_claims = derive_domain_claims(DomainChunkContext {
                document_id: &chunk.document_id,
                title: &chunk.title,
                season: &chunk.season,
                heading: &chunk.heading,
                text: &chunk.text,
                source_url: &chunk.source_url,
                yuque_url: &chunk.yuque_url,
                source_updated_at: &chunk.source_updated_at,
                document_hash: &chunk.document_hash,
                chunk_hash: &chunk.chunk_hash,
            });
        }
        let derived_domain_hash =
            domain_index_hash(chunks.iter().flat_map(|chunk| chunk.domain_claims.iter()));

        let mut document_frequency = HashMap::new();
        for chunk in &chunks {
            for token in chunk.term_frequency.keys() {
                *document_frequency.entry(token.clone()).or_insert(0) += 1;
            }
        }
        let average_document_length = if chunks.is_empty() {
            1.0
        } else {
            chunks.iter().map(|chunk| chunk.token_count).sum::<usize>() as f64 / chunks.len() as f64
        };

        Ok(Self {
            schema_version: KNOWLEDGE_INDEX_SCHEMA_V1,
            corpus_hash: format!("{:x}", corpus_hasher.finalize()),
            domain_index_hash: derived_domain_hash,
            chunks,
            document_frequency,
            average_document_length,
            seasons,
            categories,
            dense: None,
            requested_retrieval: "bm25".to_string(),
            dense_fallback_code: None,
        })
    }

    pub fn schema_version(&self) -> &'static str {
        self.schema_version
    }

    pub fn corpus_hash(&self) -> &str {
        &self.corpus_hash
    }

    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    pub fn document_count(&self) -> usize {
        self.chunks
            .iter()
            .map(|chunk| chunk.document_id.as_str())
            .collect::<HashSet<_>>()
            .len()
    }

    pub fn seasons(&self) -> impl Iterator<Item = &str> {
        self.seasons.iter().map(String::as_str)
    }

    pub fn categories(&self) -> impl Iterator<Item = &str> {
        self.categories.iter().map(String::as_str)
    }

    pub fn retrieval_info(&self) -> KnowledgeRetrievalInfo {
        self.retrieval_info_with_fallback(self.dense_fallback_code.clone())
    }

    pub fn search(
        &self,
        context: &KnowledgeVersionContext,
        query: KnowledgeSearchQuery,
    ) -> Result<KnowledgeSearchResponse, KnowledgeIndexError> {
        let audience = if matches!(&query.version_scope, KnowledgeVersionScope::ReferenceLookup) {
            KnowledgeAudience {
                client: KnowledgeClientScope::Any,
                mount: None,
            }
        } else {
            KnowledgeAudience::from_question(&query.query, None)
        };
        self.search_with_audience(context, query, audience)
    }

    pub fn search_with_audience(
        &self,
        context: &KnowledgeVersionContext,
        query: KnowledgeSearchQuery,
        audience: KnowledgeAudience,
    ) -> Result<KnowledgeSearchResponse, KnowledgeIndexError> {
        validate_query(&query)?;
        if let Some(category) = query.category.as_ref() {
            if !self.categories.contains(category) {
                return Err(KnowledgeIndexError::UnknownCategory(category.clone()));
            }
        }
        self.validate_version_scope(context, &query)?;

        let query_tokens = lexical_tokens(&query.query);
        if query_tokens.is_empty() {
            return Err(KnowledgeIndexError::InvalidQuery(
                "query has no searchable terms",
            ));
        }
        let query_terms = query_tokens.into_iter().collect::<BTreeSet<_>>();
        let raw_query = query.query.trim().to_lowercase();
        let query_intent = knowledge_query_intent(&query.query);
        let test_server_release_hint = match query.version_scope {
            KnowledgeVersionScope::CurrentOnly
                if context.game_version == GameVersion::AnYingQianJiTest
                    && query_mentions_season(&query.query, "暗影千机（2026）") =>
            {
                Some("暗影千机")
            }
            _ => None,
        };
        let mut eligible = Vec::new();
        let mut lexical_ranked = Vec::new();
        for (chunk_index, chunk) in self.chunks.iter().enumerate() {
            if !matches!(&query.version_scope, KnowledgeVersionScope::ReferenceLookup)
                && !audience_accepts_chunk(audience, chunk)
            {
                continue;
            }
            let Some(version_match) = version_match(
                context,
                &query.version_scope,
                query.category.as_deref(),
                chunk,
            ) else {
                continue;
            };
            if query
                .category
                .as_ref()
                .is_some_and(|category| category != &chunk.category)
            {
                continue;
            }
            if test_server_release_hint.is_some_and(|release| !chunk.title.contains(release)) {
                continue;
            }
            eligible.push((chunk_index, version_match));
            let lexical_score = self.bm25_score(chunk, &query_terms);
            if lexical_score <= 0.0 || !lexical_score.is_finite() {
                continue;
            }
            let mut score = lexical_score;
            if chunk.title.to_lowercase().contains(&raw_query) {
                score += 5.0;
            }
            if chunk.text.to_lowercase().contains(&raw_query) {
                score += 1.5;
            }
            if matches!(query.version_scope, KnowledgeVersionScope::CurrentOnly)
                && chunk
                    .title
                    .contains(current_release_label(context.game_version))
            {
                score += 8.0;
            }
            if chunk.quality == KnowledgeQuality::MetadataOnly {
                score *= 0.35;
            }
            if chunk.version_warning.is_some() {
                score *= 0.75;
            }
            lexical_ranked.push((chunk_index, score, version_match));
        }
        lexical_ranked.sort_by(|left, right| {
            fact_eligible(&self.chunks[right.0])
                .cmp(&fact_eligible(&self.chunks[left.0]))
                .then_with(|| right.1.partial_cmp(&left.1).unwrap_or(Ordering::Equal))
                .then_with(|| self.chunks[left.0].title.cmp(&self.chunks[right.0].title))
                .then_with(|| {
                    self.chunks[left.0]
                        .heading
                        .cmp(&self.chunks[right.0].heading)
                })
        });

        let lexical_scores = lexical_ranked
            .iter()
            .map(|(index, score, _)| (*index, *score))
            .collect::<HashMap<_, _>>();
        let lexical_ranks = lexical_ranked
            .iter()
            .take(100)
            .enumerate()
            .map(|(rank, (index, _, _))| (*index, rank + 1))
            .collect::<HashMap<_, _>>();
        let version_matches = eligible.iter().copied().collect::<HashMap<_, _>>();

        let mut query_fallback = self.dense_fallback_code.clone();
        let dense_hits = if let Some(dense) = &self.dense {
            let eligible_indices = eligible.iter().map(|(index, _)| *index).collect::<Vec<_>>();
            match dense.rank(&query.query, &eligible_indices) {
                Ok(hits) => hits,
                Err(error) => {
                    eprintln!(
                        "[agent][knowledge] Embedded 查询失败（{}），本次检索降级 BM25",
                        error.code
                    );
                    query_fallback = Some(error.code.to_string());
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };
        let dense_scores = dense_hits
            .iter()
            .take(100)
            .map(|hit| (hit.chunk_index, hit.similarity as f64))
            .collect::<HashMap<_, _>>();
        let dense_ranks = dense_hits
            .iter()
            .take(100)
            .enumerate()
            .map(|(rank, hit)| (hit.chunk_index, rank + 1))
            .collect::<HashMap<_, _>>();

        let hybrid_active = self.dense.is_some() && query_fallback.is_none();
        let mut candidate_indices = lexical_ranks.keys().copied().collect::<HashSet<_>>();
        if hybrid_active {
            candidate_indices.extend(dense_ranks.keys().copied().filter(|chunk_index| {
                lexical_ranks.contains_key(chunk_index)
                    || dense_scores
                        .get(chunk_index)
                        .is_some_and(|similarity| *similarity >= 0.55)
            }));
        }
        let mut ranked = candidate_indices
            .into_iter()
            .filter_map(|chunk_index| {
                let version_match = *version_matches.get(&chunk_index)?;
                let score = if hybrid_active {
                    reciprocal_rank_fusion(
                        lexical_ranks.get(&chunk_index).copied(),
                        dense_ranks.get(&chunk_index).copied(),
                    )
                } else {
                    *lexical_scores.get(&chunk_index).unwrap_or(&0.0)
                };
                (score > 0.0).then_some((chunk_index, score, version_match))
            })
            .collect::<Vec<_>>();
        ranked.sort_by(|left, right| {
            fact_eligible(&self.chunks[right.0])
                .cmp(&fact_eligible(&self.chunks[left.0]))
                .then_with(|| {
                    adaptive_rank_score(query_intent, &self.chunks[right.0], right.1)
                        .partial_cmp(&adaptive_rank_score(
                            query_intent,
                            &self.chunks[left.0],
                            left.1,
                        ))
                        .unwrap_or(Ordering::Equal)
                })
                .then_with(|| self.chunks[left.0].title.cmp(&self.chunks[right.0].title))
                .then_with(|| {
                    self.chunks[left.0]
                        .heading
                        .cmp(&self.chunks[right.0].heading)
                })
        });

        let mut seen_documents = HashSet::new();
        let ranked_documents = ranked
            .into_iter()
            .filter(|(chunk_index, _, _)| {
                seen_documents.insert(self.chunks[*chunk_index].document_id.clone())
            })
            .take(query.top_k.min(MAX_KNOWLEDGE_RESULTS))
            .collect::<Vec<_>>();
        let (selected_documents, selection) = select_adaptive_documents(
            query_intent,
            &raw_query,
            &ranked_documents,
            &self.chunks,
            &lexical_scores,
            &dense_scores,
        );
        let results = selected_documents
            .into_iter()
            .map(|(chunk_index, score, version_match)| {
                let chunk = &self.chunks[chunk_index];
                let exact_phrase_match = chunk.text.to_lowercase().contains(&raw_query)
                    || chunk.title.to_lowercase().contains(&raw_query);
                let reference_entities =
                    if matches!(&query.version_scope, KnowledgeVersionScope::ReferenceLookup)
                        && exact_phrase_match
                    {
                        extract_reference_entities(&chunk.text)
                    } else {
                        Vec::new()
                    };
                KnowledgeSearchResult {
                    document_id: chunk.document_id.clone(),
                    title: chunk.title.clone(),
                    season: chunk.season.clone(),
                    category: chunk.category.clone(),
                    heading: chunk.heading.clone(),
                    snippet: snippet(&chunk.text),
                    source_url: chunk.source_url.clone(),
                    yuque_url: chunk.yuque_url.clone(),
                    source_site: chunk.source_site.clone(),
                    source_updated_at: chunk.source_updated_at.clone(),
                    quality: chunk.quality,
                    fact_eligible: fact_eligible(chunk),
                    version_match,
                    version_warning: chunk.version_warning.clone(),
                    document_hash: chunk.document_hash.clone(),
                    chunk_hash: chunk.chunk_hash.clone(),
                    score: round_score(score),
                    lexical_score: lexical_scores.get(&chunk_index).copied().map(round_score),
                    dense_similarity: dense_scores.get(&chunk_index).copied().map(round_score),
                    exact_phrase_match,
                    reference_entities,
                    domain_relations: domain_relations(&chunk.domain_claims),
                    domain_claims: chunk.domain_claims.clone(),
                }
            })
            .collect();

        Ok(KnowledgeSearchResponse {
            schema_version: KNOWLEDGE_SEARCH_SCHEMA_V1.to_string(),
            corpus_hash: self.corpus_hash.clone(),
            domain_index_hash: self.domain_index_hash.clone(),
            current_season: context.current_season.to_string(),
            requested_scope: query.version_scope,
            audience,
            retrieval: self.retrieval_info_with_fallback(query_fallback),
            selection,
            results,
        })
    }

    fn retrieval_info_with_fallback(
        &self,
        fallback_code: Option<String>,
    ) -> KnowledgeRetrievalInfo {
        let active = self.dense.is_some() && fallback_code.is_none();
        KnowledgeRetrievalInfo {
            requested_mode: self.requested_retrieval.clone(),
            active_mode: if active { "hybrid_rrf" } else { "bm25" }.to_string(),
            dense_model: active.then(|| DENSE_MODEL_ID.to_string()),
            cache_state: self
                .dense
                .as_ref()
                .map(|dense| dense.cache_state().as_str())
                .unwrap_or(if self.requested_retrieval == "bm25" {
                    "disabled"
                } else {
                    "unavailable"
                })
                .to_string(),
            fallback_code,
        }
    }

    fn validate_version_scope(
        &self,
        context: &KnowledgeVersionContext,
        query: &KnowledgeSearchQuery,
    ) -> Result<(), KnowledgeIndexError> {
        let mentioned = self
            .seasons
            .iter()
            .filter(|season| query_mentions_season(&query.query, season))
            .cloned()
            .collect::<Vec<_>>();
        match &query.version_scope {
            KnowledgeVersionScope::CurrentOnly => {
                if let Some(conflict) = mentioned
                    .iter()
                    .find(|season| !context.accepts_query_season(season))
                {
                    return Err(KnowledgeIndexError::VersionConflict {
                        requested: conflict.clone(),
                        current: context.current_season.to_string(),
                    });
                }
            }
            KnowledgeVersionScope::SpecificSeason { season } => {
                if !self.seasons.contains(season) {
                    return Err(KnowledgeIndexError::UnknownSeason(season.clone()));
                }
                if let Some(conflict) = mentioned.iter().find(|mentioned| *mentioned != season) {
                    return Err(KnowledgeIndexError::VersionConflict {
                        requested: conflict.clone(),
                        current: season.clone(),
                    });
                }
            }
            KnowledgeVersionScope::CrossVersion => {
                if !has_cross_version_intent(&query.query) {
                    return Err(KnowledgeIndexError::CrossVersionIntentRequired);
                }
            }
            KnowledgeVersionScope::ReferenceLookup => {}
        }
        Ok(())
    }

    fn bm25_score(&self, chunk: &KnowledgeChunk, terms: &BTreeSet<String>) -> f64 {
        const K1: f64 = 1.2;
        const B: f64 = 0.75;
        let total = self.chunks.len() as f64;
        let length = chunk.token_count as f64;
        terms
            .iter()
            .filter_map(|term| {
                let tf = *chunk.term_frequency.get(term)? as f64;
                let df = *self.document_frequency.get(term)? as f64;
                let idf = ((total - df + 0.5) / (df + 0.5) + 1.0).ln();
                let denominator =
                    tf + K1 * (1.0 - B + B * length / self.average_document_length.max(1.0));
                Some(idf * (tf * (K1 + 1.0)) / denominator)
            })
            .sum()
    }
}

fn audience_accepts_chunk(audience: KnowledgeAudience, chunk: &KnowledgeChunk) -> bool {
    let (chunk_client, chunk_mount) = classify_chunk_audience(chunk);
    let client_matches = match audience.client {
        KnowledgeClientScope::Flagship => chunk_client != KnowledgeClientScope::Wujie,
        KnowledgeClientScope::Wujie => chunk_client == KnowledgeClientScope::Wujie,
        KnowledgeClientScope::Any => true,
    };
    let mount_matches = match (audience.mount, chunk_mount) {
        (Some(requested), Some(actual)) => requested == actual,
        _ => true,
    };
    client_matches && mount_matches
}

fn classify_chunk_audience(
    chunk: &KnowledgeChunk,
) -> (KnowledgeClientScope, Option<KnowledgeMountScope>) {
    let title = chunk.title.to_lowercase();
    let is_wujie = title.contains("无界")
        || title.contains("分山劲·悟")
        || title.contains("分山劲・悟")
        || title.contains("wujie");
    let mentions_fenshan = title.contains("分山劲") || title.contains("分山");
    let mentions_tiegu = title.contains("铁骨衣") || title.contains("铁骨");
    let mount = match (mentions_fenshan, mentions_tiegu) {
        (true, false) => Some(KnowledgeMountScope::Fenshanjin),
        (false, true) => Some(KnowledgeMountScope::Tieguyi),
        _ => None,
    };
    (
        if is_wujie {
            KnowledgeClientScope::Wujie
        } else {
            KnowledgeClientScope::Flagship
        },
        mount,
    )
}

fn knowledge_query_intent(query: &str) -> KnowledgeQueryIntent {
    let normalized = query.to_lowercase();
    if ["一键宏", "宏", "按键", "键位"]
        .iter()
        .any(|keyword| normalized.contains(keyword))
    {
        KnowledgeQueryIntent::Macro
    } else if ["配装", "装备", "属性", "破招", "加速阈值"]
        .iter()
        .any(|keyword| normalized.contains(keyword))
    {
        KnowledgeQueryIntent::Equipment
    } else if ["副本", "实战", "首领", "boss", "秘境", "打法"]
        .iter()
        .any(|keyword| normalized.contains(keyword))
    {
        KnowledgeQueryIntent::Encounter
    } else if ["循环", "空转", "手法", "盾飞", "劫刀", "流血", "节奏"]
        .iter()
        .any(|keyword| normalized.contains(keyword))
    {
        KnowledgeQueryIntent::Rotation
    } else if ["机制", "系数", "概率", "技改", "伤害", "重置"]
        .iter()
        .any(|keyword| normalized.contains(keyword))
    {
        KnowledgeQueryIntent::Mechanism
    } else {
        KnowledgeQueryIntent::General
    }
}

fn knowledge_source_role(chunk: &KnowledgeChunk) -> KnowledgeSourceRole {
    if chunk.category.contains("白皮书") || chunk.title.contains("白皮书") {
        KnowledgeSourceRole::Whitepaper
    } else if chunk.category.contains("实战")
        || ["实战", "副本", "攻略", "打法"]
            .iter()
            .any(|keyword| chunk.title.contains(keyword))
    {
        KnowledgeSourceRole::Practical
    } else if chunk.category.contains("宏") || chunk.title.contains('宏') {
        KnowledgeSourceRole::Macro
    } else if ["机制", "系数", "技改", "推导", "基础", "通用"]
        .iter()
        .any(|keyword| chunk.category.contains(keyword) || chunk.title.contains(keyword))
    {
        KnowledgeSourceRole::Mechanism
    } else {
        KnowledgeSourceRole::General
    }
}

fn adaptive_rank_score(
    intent: KnowledgeQueryIntent,
    chunk: &KnowledgeChunk,
    retrieval_score: f64,
) -> f64 {
    let role = knowledge_source_role(chunk);
    let multiplier = match (intent, role) {
        (KnowledgeQueryIntent::Rotation, KnowledgeSourceRole::Whitepaper) => 1.18,
        (KnowledgeQueryIntent::Rotation, KnowledgeSourceRole::Practical) => 1.14,
        (KnowledgeQueryIntent::Rotation, KnowledgeSourceRole::Mechanism) => 1.05,
        (KnowledgeQueryIntent::Rotation, KnowledgeSourceRole::Macro) => 0.94,
        (KnowledgeQueryIntent::Macro, KnowledgeSourceRole::Macro) => 1.22,
        (KnowledgeQueryIntent::Macro, KnowledgeSourceRole::Whitepaper) => 1.04,
        (KnowledgeQueryIntent::Equipment, KnowledgeSourceRole::Whitepaper) => 1.16,
        (KnowledgeQueryIntent::Equipment, KnowledgeSourceRole::Mechanism) => 1.08,
        (KnowledgeQueryIntent::Encounter, KnowledgeSourceRole::Practical) => 1.20,
        (KnowledgeQueryIntent::Encounter, KnowledgeSourceRole::Whitepaper) => 1.06,
        (KnowledgeQueryIntent::Mechanism, KnowledgeSourceRole::Mechanism) => 1.18,
        (KnowledgeQueryIntent::Mechanism, KnowledgeSourceRole::Whitepaper) => 1.08,
        _ => 1.0,
    };
    retrieval_score * multiplier
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdaptiveConfidence {
    High,
    Medium,
    Low,
}

impl AdaptiveConfidence {
    fn as_str(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
        }
    }
}

type RankedKnowledgeDocument = (usize, f64, KnowledgeVersionMatch);

fn select_adaptive_documents(
    intent: KnowledgeQueryIntent,
    raw_query: &str,
    ranked: &[RankedKnowledgeDocument],
    chunks: &[KnowledgeChunk],
    lexical_scores: &HashMap<usize, f64>,
    dense_scores: &HashMap<usize, f64>,
) -> (Vec<RankedKnowledgeDocument>, KnowledgeSelectionInfo) {
    if ranked.is_empty() {
        return (
            Vec::new(),
            KnowledgeSelectionInfo {
                strategy: "adaptive_evidence/v1".to_string(),
                intent,
                confidence: "none".to_string(),
                candidate_documents: 0,
                returned_documents: 0,
                source_roles: Vec::new(),
                decision: "no_match".to_string(),
            },
        );
    }

    let first_index = ranked[0].0;
    let first = &chunks[first_index];
    let exact_phrase = first.title.to_lowercase().contains(raw_query)
        || first.text.to_lowercase().contains(raw_query);
    let lexical = lexical_scores.contains_key(&first_index);
    let dense = dense_scores.get(&first_index).copied();
    let confidence = if exact_phrase || (lexical && dense.is_some_and(|score| score >= 0.60)) {
        AdaptiveConfidence::High
    } else if lexical || dense.is_some_and(|score| score >= 0.55) {
        AdaptiveConfidence::Medium
    } else {
        AdaptiveConfidence::Low
    };

    let mut target = match confidence {
        AdaptiveConfidence::High => 3,
        AdaptiveConfidence::Medium => 4,
        AdaptiveConfidence::Low => 6,
    }
    .min(ranked.len());
    if matches!(intent, KnowledgeQueryIntent::General) && target < ranked.len() {
        target += 1;
    }

    let mut selected = ranked.iter().take(target).copied().collect::<Vec<_>>();
    let mut roles = selected
        .iter()
        .map(|(index, _, _)| knowledge_source_role(&chunks[*index]))
        .collect::<HashSet<_>>();
    let desired_role_count = match intent {
        KnowledgeQueryIntent::Rotation
        | KnowledgeQueryIntent::Equipment
        | KnowledgeQueryIntent::Encounter
        | KnowledgeQueryIntent::Mechanism => 2,
        KnowledgeQueryIntent::Macro | KnowledgeQueryIntent::General => 1,
    };
    let initial_target = target;
    if roles.len() < desired_role_count {
        for candidate in ranked.iter().skip(target) {
            let role = knowledge_source_role(&chunks[candidate.0]);
            if roles.insert(role) {
                selected.push(*candidate);
                if roles.len() >= desired_role_count || selected.len() >= MAX_KNOWLEDGE_RESULTS {
                    break;
                }
            }
        }
    }
    roles = selected
        .iter()
        .map(|(index, _, _)| knowledge_source_role(&chunks[*index]))
        .collect();
    let mut source_roles = roles.into_iter().collect::<Vec<_>>();
    source_roles.sort_by_key(|role| match role {
        KnowledgeSourceRole::Whitepaper => 0,
        KnowledgeSourceRole::Practical => 1,
        KnowledgeSourceRole::Mechanism => 2,
        KnowledgeSourceRole::Macro => 3,
        KnowledgeSourceRole::General => 4,
    });
    let decision = if selected.len() > initial_target {
        "expanded_for_source_coverage"
    } else {
        match confidence {
            AdaptiveConfidence::High => "stopped_on_high_confidence",
            AdaptiveConfidence::Medium => "balanced_evidence",
            AdaptiveConfidence::Low => "expanded_for_low_confidence",
        }
    };
    let selection = KnowledgeSelectionInfo {
        strategy: "adaptive_evidence/v1".to_string(),
        intent,
        confidence: confidence.as_str().to_string(),
        candidate_documents: ranked.len(),
        returned_documents: selected.len(),
        source_roles,
        decision: decision.to_string(),
    };
    (selected, selection)
}

fn dense_document_text(chunk: &KnowledgeChunk) -> String {
    let relations = chunk
        .domain_claims
        .iter()
        .map(|claim| {
            format!(
                "{} {} {}",
                claim.subject.name, claim.relation, claim.object.name
            )
        })
        .collect::<Vec<_>>()
        .join("；");
    let boundaries = chunk
        .domain_claims
        .iter()
        .flat_map(|claim| claim.verification.boundary_codes.iter())
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join("；");
    format!(
        "标题：{}\n赛季：{}\n分类：{}\n章节：{}\n领域关系：{}\n验证边界：{}\n{}",
        chunk.title, chunk.season, chunk.category, chunk.heading, relations, boundaries, chunk.text
    )
}

fn reciprocal_rank_fusion(lexical_rank: Option<usize>, dense_rank: Option<usize>) -> f64 {
    const RRF_K: f64 = 60.0;
    const DENSE_WEIGHT: f64 = 0.7;
    let lexical = lexical_rank
        .map(|rank| 1.0 / (RRF_K + rank as f64))
        .unwrap_or(0.0);
    let dense = dense_rank
        .map(|rank| DENSE_WEIGHT / (RRF_K + rank as f64))
        .unwrap_or(0.0);
    (lexical + dense) * 1_000.0
}

fn fact_eligible(chunk: &KnowledgeChunk) -> bool {
    chunk.quality == KnowledgeQuality::FullText && chunk.version_warning.is_none()
}

fn validate_query(query: &KnowledgeSearchQuery) -> Result<(), KnowledgeIndexError> {
    let length = query.query.chars().count();
    if length == 0 || query.query.trim().is_empty() {
        return Err(KnowledgeIndexError::InvalidQuery("query is empty"));
    }
    if length > MAX_QUERY_CHARACTERS {
        return Err(KnowledgeIndexError::InvalidQuery("query is too long"));
    }
    if query.query.chars().any(char::is_control) {
        return Err(KnowledgeIndexError::InvalidQuery(
            "query contains control characters",
        ));
    }
    if !(1..=MAX_KNOWLEDGE_RESULTS).contains(&query.top_k) {
        return Err(KnowledgeIndexError::InvalidQuery(
            "top_k must be within 1..=8",
        ));
    }
    Ok(())
}

fn version_match(
    context: &KnowledgeVersionContext,
    scope: &KnowledgeVersionScope,
    requested_category: Option<&str>,
    chunk: &KnowledgeChunk,
) -> Option<KnowledgeVersionMatch> {
    match scope {
        KnowledgeVersionScope::CurrentOnly => {
            if chunk.season != context.current_season {
                return None;
            }
            if let Some(category) = context.required_category {
                if chunk.category != category {
                    return None;
                }
                Some(KnowledgeVersionMatch::TestServerExact)
            } else {
                Some(KnowledgeVersionMatch::CurrentExact)
            }
        }
        KnowledgeVersionScope::SpecificSeason { season } => {
            if &chunk.season != season {
                None
            } else if season == context.current_season {
                if let Some(category) = context.required_category {
                    if chunk.category == category {
                        Some(KnowledgeVersionMatch::TestServerExact)
                    } else if requested_category == Some(chunk.category.as_str()) {
                        Some(KnowledgeVersionMatch::HistoricalExplicit)
                    } else {
                        None
                    }
                } else {
                    Some(KnowledgeVersionMatch::CurrentExact)
                }
            } else {
                Some(KnowledgeVersionMatch::HistoricalExplicit)
            }
        }
        KnowledgeVersionScope::CrossVersion => Some(
            if chunk.season == context.current_season
                && context
                    .required_category
                    .is_none_or(|category| chunk.category == category)
            {
                if context.required_category.is_some() {
                    KnowledgeVersionMatch::TestServerExact
                } else {
                    KnowledgeVersionMatch::CurrentExact
                }
            } else {
                KnowledgeVersionMatch::CrossVersion
            },
        ),
        KnowledgeVersionScope::ReferenceLookup => Some(KnowledgeVersionMatch::ReferenceOnly),
    }
}

fn safe_relative_path(value: &str) -> Result<PathBuf, KnowledgeIndexError> {
    let normalized = value.replace('\\', "/");
    let path = Path::new(&normalized);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(KnowledgeIndexError::UnsafePath(value.to_string()));
    }
    Ok(path.to_path_buf())
}

fn strip_frontmatter(markdown: &str) -> &str {
    let normalized = markdown.strip_prefix('\u{feff}').unwrap_or(markdown);
    if !normalized.starts_with("---\n") && !normalized.starts_with("---\r\n") {
        return normalized;
    }
    let mut offset = 0;
    let mut opening_seen = false;
    for line in normalized.split_inclusive('\n') {
        offset += line.len();
        if line.trim_end_matches(['\r', '\n']) == "---" {
            if opening_seen {
                return &normalized[offset..];
            }
            opening_seen = true;
        }
    }
    normalized
}

fn clean_markdown_for_index(markdown: &str) -> String {
    let mut output = String::new();
    for line in markdown.lines() {
        let trimmed = line.trim();
        if trimmed == "## 来源与溯源" {
            break;
        }
        if trimmed.contains("data:image") || trimmed.starts_with("<img") {
            continue;
        }
        let cleaned = strip_link_destinations(trimmed);
        if !cleaned.trim().is_empty() {
            output.push_str(cleaned.trim());
            output.push('\n');
        }
    }
    output
}

fn strip_link_destinations(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if index + 1 < bytes.len() && bytes[index] == b']' && bytes[index + 1] == b'(' {
            output.push(']');
            index += 2;
            let mut depth = 1_u32;
            while index < bytes.len() && depth > 0 {
                match bytes[index] {
                    b'(' => depth += 1,
                    b')' => depth -= 1,
                    _ => {}
                }
                index += 1;
            }
            continue;
        }
        let ch = value[index..].chars().next().expect("valid UTF-8 boundary");
        output.push(ch);
        index += ch.len_utf8();
    }
    output.replace("![", "[")
}

fn split_markdown_chunks(markdown: &str) -> Vec<(String, String)> {
    let mut sections = Vec::new();
    let mut heading = "正文".to_string();
    let mut body = String::new();
    for line in markdown.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            if !body.trim().is_empty() {
                push_bounded_chunks(&mut sections, &heading, body.trim());
                body.clear();
            }
            heading = trimmed.trim_start_matches('#').trim().to_string();
        } else {
            body.push_str(trimmed);
            body.push('\n');
        }
    }
    if !body.trim().is_empty() {
        push_bounded_chunks(&mut sections, &heading, body.trim());
    }
    if sections.is_empty() && !markdown.trim().is_empty() {
        push_bounded_chunks(&mut sections, &heading, markdown.trim());
    }
    sections
}

fn push_bounded_chunks(output: &mut Vec<(String, String)>, heading: &str, text: &str) {
    let characters = text.chars().collect::<Vec<_>>();
    if characters.len() <= CHUNK_CHARACTERS {
        output.push((heading.to_string(), text.to_string()));
        return;
    }
    let mut start = 0;
    while start < characters.len() {
        let end = (start + CHUNK_CHARACTERS).min(characters.len());
        output.push((
            heading.to_string(),
            characters[start..end].iter().collect::<String>(),
        ));
        if end == characters.len() {
            break;
        }
        start = end.saturating_sub(CHUNK_OVERLAP);
    }
}

fn lexical_tokens(value: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut ascii = String::new();
    let mut cjk = Vec::new();
    let flush_ascii = |buffer: &mut String, output: &mut Vec<String>| {
        if !buffer.is_empty() {
            output.push(buffer.to_lowercase());
            buffer.clear();
        }
    };
    let flush_cjk = |buffer: &mut Vec<char>, output: &mut Vec<String>| {
        if buffer.is_empty() {
            return;
        }
        output.extend(buffer.iter().map(char::to_string));
        if buffer.len() >= 2 {
            output.extend(buffer.windows(2).map(|window| window.iter().collect()));
        }
        if buffer.len() >= 3 {
            output.extend(buffer.windows(3).map(|window| window.iter().collect()));
        }
        if buffer.len() <= 8 {
            output.push(buffer.iter().collect());
        }
        buffer.clear();
    };
    for character in value.chars() {
        if is_cjk(character) {
            flush_ascii(&mut ascii, &mut tokens);
            cjk.push(character);
        } else if character.is_ascii_alphanumeric() || matches!(character, '_' | '+' | '.') {
            flush_cjk(&mut cjk, &mut tokens);
            ascii.push(character.to_ascii_lowercase());
        } else {
            flush_ascii(&mut ascii, &mut tokens);
            flush_cjk(&mut cjk, &mut tokens);
        }
    }
    flush_ascii(&mut ascii, &mut tokens);
    flush_cjk(&mut cjk, &mut tokens);
    tokens
}

fn extract_reference_entities(text: &str) -> Vec<KnowledgeReferenceEntity> {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut entities = Vec::new();
    for marker in ["视频作者"] {
        let mut remainder = collapsed.as_str();
        while let Some(position) = remainder.find(marker) {
            remainder = &remainder[position + marker.len()..];
            let candidate = remainder
                .trim_start_matches(|character: char| {
                    character.is_whitespace()
                        || matches!(character, ':' | '：' | '-' | '—' | '[' | '【')
                })
                .chars()
                .take_while(|character| {
                    character.is_ascii_alphanumeric()
                        || matches!(character, '_' | '-' | '.')
                        || is_cjk(*character)
                })
                .take(64)
                .collect::<String>();
            if !candidate.is_empty()
                && candidate != "简介"
                && !entities
                    .iter()
                    .any(|entity: &KnowledgeReferenceEntity| entity.name == candidate)
            {
                entities.push(KnowledgeReferenceEntity {
                    relation: "video_author".to_string(),
                    name: candidate,
                    basis: "explicit_label_in_matched_passage".to_string(),
                });
            }
        }
    }
    entities
}

fn is_cjk(character: char) -> bool {
    matches!(
        character as u32,
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF
    )
}

fn query_mentions_season(query: &str, season: &str) -> bool {
    let short = season.split(['（', '(']).next().unwrap_or(season);
    query.contains(season) || (!short.is_empty() && query.contains(short))
}

fn current_release_label(game_version: GameVersion) -> &'static str {
    match game_version {
        GameVersion::AnYingQianJi | GameVersion::AnYingQianJiTest => "暗影千机",
        GameVersion::ShanHaiYuanLiu => "山海源流",
    }
}

fn has_cross_version_intent(query: &str) -> bool {
    [
        "历史",
        "历次",
        "演变",
        "版本对比",
        "跨版本",
        "技改对比",
        "前后版本",
    ]
    .iter()
    .any(|keyword| query.contains(keyword))
}

fn title_season_warning(
    title: &str,
    season: &str,
    version_policy: KnowledgeVersionPolicy,
) -> Option<String> {
    if version_policy == KnowledgeVersionPolicy::RollingCurrent {
        return None;
    }
    let title_years = years_in(title);
    let season_years = years_in(season);
    if title_years.is_empty()
        || season_years.is_empty()
        || title_years.iter().any(|year| season_years.contains(year))
    {
        None
    } else {
        Some(format!(
            "title years {} differ from catalog season {}",
            title_years
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(","),
            season
        ))
    }
}

fn years_in(value: &str) -> BTreeSet<u16> {
    let digits = value.chars().collect::<Vec<_>>();
    let mut years = BTreeSet::new();
    for window in digits.windows(4) {
        if window.iter().all(char::is_ascii_digit) {
            if let Ok(year) = window.iter().collect::<String>().parse::<u16>() {
                if (2010..=2099).contains(&year) {
                    years.insert(year);
                }
            }
        }
    }
    years
}

fn source_host(url: &str) -> String {
    url.split_once("://")
        .map(|(_, rest)| rest.split('/').next().unwrap_or_default())
        .unwrap_or_default()
        .to_lowercase()
}

fn snippet(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut result = collapsed
        .chars()
        .take(SNIPPET_CHARACTERS)
        .collect::<String>();
    if collapsed.chars().count() > SNIPPET_CHARACTERS {
        result.push('…');
    }
    result
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn round_score(score: f64) -> f64 {
    (score * 1_000_000.0).round() / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{knowledge_prefetch, select_analysis_plan, AgentRuntime};
    use serde::Deserialize;
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn create() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = env::temp_dir().join(format!("jx3-knowledge-{nonce}"));
            fs::create_dir_all(&root).unwrap();
            let entries = vec![
                fixture_entry(
                    &root,
                    "暗影千机（2026）/基础/当前循环.md",
                    "当前循环",
                    "暗影千机（2026）",
                    "基础",
                    "yuque_document",
                    "",
                    "# 当前循环\n\n盾飞阶段需要关注劫刀数量，并避免流血中断。",
                ),
                fixture_entry(
                    &root,
                    "暗影千机（2026）/白皮书/分山劲白皮书.md",
                    "暗影千机_ 分山劲白皮书",
                    "暗影千机（2026）",
                    "白皮书",
                    "yuque_document",
                    "",
                    "# 旗舰分山循环\n\n旗舰端分山劲通过稳定盾飞和劫刀节奏减少空转。",
                ),
                fixture_entry(
                    &root,
                    "暗影千机（2026）/白皮书/分山劲悟白皮书.md",
                    "暗影千机_ 分山劲·悟白皮书",
                    "暗影千机（2026）",
                    "白皮书",
                    "yuque_document",
                    "",
                    "# 无界分山循环\n\n无界端分山劲·悟通过自己的技能循环减少空转。",
                ),
                fixture_entry(
                    &root,
                    "暗影千机（2026）/白皮书/铁骨衣白皮书.md",
                    "暗影千机_ 铁骨衣白皮书",
                    "暗影千机（2026）",
                    "白皮书",
                    "yuque_document",
                    "",
                    "# 旗舰铁骨循环\n\n旗舰端铁骨衣使用防御循环处理技能空转。",
                ),
                fixture_entry(
                    &root,
                    "山海源流（2025）/基础/旧版循环.md",
                    "旧版循环（2025）",
                    "山海源流（2025）",
                    "基础",
                    "external_mirror",
                    "full",
                    "# 旧版循环\n\n大家好，世一苍回来了。视频作者 author_a，修改自过崽攻略。山海源流的盾飞循环使用旧版劫刀节奏。",
                ),
                fixture_entry(
                    &root,
                    "体服（2021-2025）/130级/体服改动.md",
                    "暗影千机体服改动",
                    "体服（2021-2025）",
                    "130级",
                    "yuque_document",
                    "",
                    "# 体服改动\n\n暗影千机测试服调整盾飞循环。",
                ),
                fixture_entry(
                    &root,
                    "体服（2021-2025）/130级/旧赛季体服改动.md",
                    "山海源流体服改动",
                    "体服（2021-2025）",
                    "130级",
                    "yuque_document",
                    "",
                    "# 体服改动\n\n山海源流测试服也调整过盾飞循环。",
                ),
                fixture_entry(
                    &root,
                    "体服（2021-2025）/120级/旧体服.md",
                    "旧体服改动",
                    "体服（2021-2025）",
                    "120级",
                    "yuque_document",
                    "",
                    "# 旧体服\n\n旧等级测试服盾飞资料。",
                ),
                fixture_entry(
                    &root,
                    "暗影千机（2026）/计算器/仅链接.md",
                    "在线计算器",
                    "暗影千机（2026）",
                    "计算器",
                    "external_mirror",
                    "metadata_only",
                    "# 在线计算器\n\n安全验证页面。",
                ),
            ];
            let manifest = json!({"entries": entries});
            fs::write(
                root.join(MANIFEST_FILE),
                serde_json::to_vec_pretty(&manifest).unwrap(),
            )
            .unwrap();
            Self { root }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let expected_root = env::temp_dir();
            assert!(self.root.starts_with(&expected_root));
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn fixture_entry(
        root: &Path,
        relative: &str,
        title: &str,
        season: &str,
        category: &str,
        kind: &str,
        mirror_status: &str,
        body: &str,
    ) -> serde_json::Value {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, format!("---\ntitle: {title}\n---\n\n{body}\n")).unwrap();
        json!({
            "title": title,
            "season": season,
            "category": category,
            "kind": kind,
            "source": format!("https://example.com/{title}"),
            "output": relative,
            "source_site": "example.com",
            "yuque_url": "https://www.yuque.com/sgyxy/cangyun",
            "updated_at": "2026-08-26T00:00:00Z",
            "yuque_uuid": "",
            "mirror_status": mirror_status
        })
    }

    fn query(scope: KnowledgeVersionScope, text: &str) -> KnowledgeSearchQuery {
        KnowledgeSearchQuery {
            query: text.to_string(),
            version_scope: scope,
            category: None,
            top_k: 5,
        }
    }

    #[test]
    fn current_scope_never_silently_returns_an_old_season() {
        let fixture = Fixture::create();
        let index = KnowledgeIndex::load(&fixture.root).unwrap();
        let response = index
            .search(
                &KnowledgeVersionContext::from_game_version(GameVersion::AnYingQianJi),
                query(KnowledgeVersionScope::CurrentOnly, "盾飞 劫刀 流血"),
            )
            .unwrap();

        assert!(!response.results.is_empty());
        assert!(response
            .results
            .iter()
            .all(|result| result.season == "暗影千机（2026）"));
        assert!(response
            .results
            .iter()
            .all(|result| { result.version_match == KnowledgeVersionMatch::CurrentExact }));
    }

    #[test]
    fn ambiguous_questions_default_to_flagship_and_selected_mount() {
        let fixture = Fixture::create();
        let index = KnowledgeIndex::load(&fixture.root).unwrap();
        let audience =
            KnowledgeAudience::from_question("怎样减少空转", Some(KnowledgeMountScope::Fenshanjin));
        let response = index
            .search_with_audience(
                &KnowledgeVersionContext::from_game_version(GameVersion::AnYingQianJi),
                query(KnowledgeVersionScope::CurrentOnly, "怎样减少空转"),
                audience,
            )
            .unwrap();

        assert_eq!(response.audience.client, KnowledgeClientScope::Flagship);
        assert_eq!(
            response.audience.mount,
            Some(KnowledgeMountScope::Fenshanjin)
        );
        assert!(response
            .results
            .iter()
            .any(|result| result.title.contains("分山劲白皮书")));
        assert!(response
            .results
            .iter()
            .all(|result| !result.title.contains("分山劲·悟") && !result.title.contains("铁骨衣")));
    }

    #[test]
    fn explicit_wujie_question_only_returns_wujie_material() {
        let fixture = Fixture::create();
        let index = KnowledgeIndex::load(&fixture.root).unwrap();
        let response = index
            .search(
                &KnowledgeVersionContext::from_game_version(GameVersion::AnYingQianJi),
                query(
                    KnowledgeVersionScope::CurrentOnly,
                    "无界分山劲·悟怎样减少空转",
                ),
            )
            .unwrap();

        assert_eq!(response.audience.client, KnowledgeClientScope::Wujie);
        assert!(!response.results.is_empty());
        assert!(response
            .results
            .iter()
            .all(|result| result.title.contains("分山劲·悟")));
    }

    #[test]
    fn explicit_flagship_mount_overrides_selected_mount_for_knowledge() {
        let audience = KnowledgeAudience::from_question(
            "旗舰端铁骨衣循环怎么处理",
            Some(KnowledgeMountScope::Fenshanjin),
        );
        assert_eq!(audience.client, KnowledgeClientScope::Flagship);
        assert_eq!(audience.mount, Some(KnowledgeMountScope::Tieguyi));
    }

    #[test]
    fn adaptive_selection_stops_early_on_strong_evidence_and_expands_when_weak() {
        let fixture = Fixture::create();
        let index = KnowledgeIndex::load(&fixture.root).unwrap();
        let mut seen = HashSet::new();
        let ranked = index
            .chunks
            .iter()
            .enumerate()
            .filter(|(_, chunk)| seen.insert(chunk.document_id.clone()))
            .take(MAX_KNOWLEDGE_RESULTS)
            .enumerate()
            .map(|(rank, (index, _))| {
                (
                    index,
                    20.0 - rank as f64,
                    KnowledgeVersionMatch::CurrentExact,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(ranked.len(), MAX_KNOWLEDGE_RESULTS);

        let first = ranked[0].0;
        let mut lexical = HashMap::new();
        lexical.insert(first, 12.0);
        let mut dense = HashMap::new();
        dense.insert(first, 0.72);
        let (strong, strong_info) = select_adaptive_documents(
            KnowledgeQueryIntent::Rotation,
            &index.chunks[first].title.to_lowercase(),
            &ranked,
            &index.chunks,
            &lexical,
            &dense,
        );
        let (weak, weak_info) = select_adaptive_documents(
            KnowledgeQueryIntent::Rotation,
            "语义模糊的问题",
            &ranked,
            &index.chunks,
            &HashMap::new(),
            &HashMap::new(),
        );

        assert_eq!(strong_info.confidence, "high");
        assert!(matches!(
            strong_info.decision.as_str(),
            "stopped_on_high_confidence" | "expanded_for_source_coverage"
        ));
        assert!(strong.len() < weak.len());
        assert_eq!(weak_info.confidence, "low");
        assert_eq!(weak_info.decision, "expanded_for_low_confidence");
        assert!(weak.len() <= MAX_KNOWLEDGE_RESULTS);
    }

    #[test]
    fn source_roles_are_soft_preferences_instead_of_fixed_quotas() {
        let fixture = Fixture::create();
        let index = KnowledgeIndex::load(&fixture.root).unwrap();
        let whitepaper = index
            .chunks
            .iter()
            .find(|chunk| chunk.title == "暗影千机_ 分山劲白皮书")
            .unwrap();
        let general_source = index
            .chunks
            .iter()
            .find(|chunk| chunk.title == "在线计算器")
            .unwrap();
        assert!(
            adaptive_rank_score(KnowledgeQueryIntent::Rotation, whitepaper, 10.0)
                > adaptive_rank_score(KnowledgeQueryIntent::Rotation, general_source, 10.0)
        );
        assert_eq!(
            knowledge_source_role(whitepaper),
            KnowledgeSourceRole::Whitepaper
        );
    }

    #[test]
    fn reference_lookup_can_find_an_identity_without_treating_it_as_current_gameplay() {
        let fixture = Fixture::create();
        let index = KnowledgeIndex::load(&fixture.root).unwrap();
        let response = index
            .search(
                &KnowledgeVersionContext::from_game_version(GameVersion::AnYingQianJi),
                query(KnowledgeVersionScope::ReferenceLookup, "世一苍"),
            )
            .unwrap();

        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].season, "山海源流（2025）");
        assert_eq!(
            response.results[0].version_match,
            KnowledgeVersionMatch::ReferenceOnly
        );
        assert!(response.results[0].fact_eligible);
        assert!(response.results[0].exact_phrase_match);
        assert_eq!(
            response.results[0].reference_entities,
            vec![KnowledgeReferenceEntity {
                relation: "video_author".to_string(),
                name: "author_a".to_string(),
                basis: "explicit_label_in_matched_passage".to_string(),
            }]
        );
    }

    #[test]
    fn explicit_historical_scope_is_labeled_and_bounded_to_that_season() {
        let fixture = Fixture::create();
        let index = KnowledgeIndex::load(&fixture.root).unwrap();
        let response = index
            .search(
                &KnowledgeVersionContext::from_game_version(GameVersion::AnYingQianJi),
                query(
                    KnowledgeVersionScope::SpecificSeason {
                        season: "山海源流（2025）".to_string(),
                    },
                    "山海源流盾飞循环",
                ),
            )
            .unwrap();

        assert_eq!(response.results.len(), 1);
        assert_eq!(
            response.results[0].version_match,
            KnowledgeVersionMatch::HistoricalExplicit
        );
    }

    #[test]
    fn current_scope_rejects_a_question_naming_another_season() {
        let fixture = Fixture::create();
        let index = KnowledgeIndex::load(&fixture.root).unwrap();
        let error = index
            .search(
                &KnowledgeVersionContext::from_game_version(GameVersion::AnYingQianJi),
                query(KnowledgeVersionScope::CurrentOnly, "山海源流盾飞循环"),
            )
            .unwrap_err();

        assert!(matches!(error, KnowledgeIndexError::VersionConflict { .. }));
    }

    #[test]
    fn cross_version_scope_requires_explicit_comparison_intent() {
        let fixture = Fixture::create();
        let index = KnowledgeIndex::load(&fixture.root).unwrap();
        let context = KnowledgeVersionContext::from_game_version(GameVersion::AnYingQianJi);
        assert_eq!(
            index
                .search(
                    &context,
                    query(KnowledgeVersionScope::CrossVersion, "盾飞循环"),
                )
                .unwrap_err(),
            KnowledgeIndexError::CrossVersionIntentRequired
        );
        let response = index
            .search(
                &context,
                query(
                    KnowledgeVersionScope::CrossVersion,
                    "盾飞循环的历史版本对比",
                ),
            )
            .unwrap();
        assert!(response
            .results
            .iter()
            .any(|result| { result.version_match == KnowledgeVersionMatch::CrossVersion }));
    }

    #[test]
    fn test_server_scope_excludes_older_level_categories() {
        let fixture = Fixture::create();
        let index = KnowledgeIndex::load(&fixture.root).unwrap();
        let response = index
            .search(
                &KnowledgeVersionContext::from_game_version(GameVersion::AnYingQianJiTest),
                query(KnowledgeVersionScope::CurrentOnly, "体服盾飞改动"),
            )
            .unwrap();

        assert!(!response.results.is_empty());
        assert!(response.results.iter().all(|result| {
            result.category == "130级"
                && result.version_match == KnowledgeVersionMatch::TestServerExact
        }));

        let base_season_wording = index
            .search(
                &KnowledgeVersionContext::from_game_version(GameVersion::AnYingQianJiTest),
                query(KnowledgeVersionScope::CurrentOnly, "暗影千机体服盾飞改动"),
            )
            .unwrap();
        assert!(base_season_wording
            .results
            .iter()
            .all(|result| result.category == "130级" && result.title.contains("暗影千机")));
    }

    #[test]
    fn test_server_history_requires_an_explicit_old_level_category() {
        let fixture = Fixture::create();
        let index = KnowledgeIndex::load(&fixture.root).unwrap();
        let context = KnowledgeVersionContext::from_game_version(GameVersion::AnYingQianJiTest);
        let scope = KnowledgeVersionScope::SpecificSeason {
            season: "体服（2021-2025）".to_string(),
        };
        let default = index
            .search(&context, query(scope.clone(), "体服盾飞资料"))
            .unwrap();
        assert!(default
            .results
            .iter()
            .all(|result| result.category == "130级"));

        let mut historical = query(scope, "旧等级体服盾飞资料");
        historical.category = Some("120级".to_string());
        let historical = index.search(&context, historical).unwrap();
        assert!(!historical.results.is_empty());
        assert!(historical.results.iter().all(|result| {
            result.category == "120级"
                && result.version_match == KnowledgeVersionMatch::HistoricalExplicit
        }));
    }

    #[test]
    fn metadata_only_entries_are_never_fact_eligible() {
        let fixture = Fixture::create();
        let index = KnowledgeIndex::load(&fixture.root).unwrap();
        let response = index
            .search(
                &KnowledgeVersionContext::from_game_version(GameVersion::AnYingQianJi),
                query(KnowledgeVersionScope::CurrentOnly, "在线计算器"),
            )
            .unwrap();
        let result = response
            .results
            .iter()
            .find(|result| result.title == "在线计算器")
            .unwrap();
        assert_eq!(result.quality, KnowledgeQuality::MetadataOnly);
        assert!(!result.fact_eligible);
    }

    #[test]
    fn current_release_boost_never_creates_a_zero_overlap_answer() {
        let fixture = Fixture::create();
        let index = KnowledgeIndex::load(&fixture.root).unwrap();
        let response = index
            .search(
                &KnowledgeVersionContext::from_game_version(GameVersion::AnYingQianJi),
                query(
                    KnowledgeVersionScope::CurrentOnly,
                    "zzqvxyw nonexistentterm",
                ),
            )
            .unwrap();

        assert!(response.results.is_empty());
    }

    #[test]
    fn title_year_conflicts_are_visible_and_not_fact_eligible() {
        let fixture = Fixture::create();
        let path = fixture.root.join("暗影千机（2026）/通用/旧标题.md");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "---\n---\n# 苍云机制（2025）\n\n盾飞机制说明。\n").unwrap();
        let manifest_path = fixture.root.join(MANIFEST_FILE);
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["entries"].as_array_mut().unwrap().push(json!({
            "title": "苍云机制（2025）",
            "season": "暗影千机（2026）",
            "category": "通用",
            "kind": "yuque_document",
            "source": "https://example.com/old-title",
            "output": "暗影千机（2026）/通用/旧标题.md",
            "yuque_url": "https://www.yuque.com/sgyxy/cangyun/old-title"
        }));
        fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let index = KnowledgeIndex::load(&fixture.root).unwrap();
        let response = index
            .search(
                &KnowledgeVersionContext::from_game_version(GameVersion::AnYingQianJi),
                query(KnowledgeVersionScope::CurrentOnly, "苍云机制2025盾飞"),
            )
            .unwrap();
        let result = response
            .results
            .iter()
            .find(|result| result.title == "苍云机制（2025）")
            .unwrap();
        assert!(result.version_warning.is_some());
        assert!(!result.fact_eligible);
    }

    #[test]
    fn rolling_current_policy_allows_an_explicitly_maintained_evergreen_document() {
        assert!(title_season_warning(
            "苍云进阶机制（2025）",
            "暗影千机（2026）",
            KnowledgeVersionPolicy::TitleBound,
        )
        .is_some());
        assert!(title_season_warning(
            "苍云进阶机制（2025）",
            "暗影千机（2026）",
            KnowledgeVersionPolicy::RollingCurrent,
        )
        .is_none());
    }

    #[test]
    fn current_fact_eligible_results_rank_before_title_year_conflicts() {
        let fixture = Fixture::create();
        let path = fixture.root.join("暗影千机（2026）/通用/旧标题.md");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            "---\n---\n# 绝刀机制（2025）\n\n绝刀伤害机制说明。\n",
        )
        .unwrap();
        let manifest_path = fixture.root.join(MANIFEST_FILE);
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["entries"].as_array_mut().unwrap().push(json!({
            "title": "绝刀机制（2025）",
            "season": "暗影千机（2026）",
            "category": "通用",
            "kind": "yuque_document",
            "source": "https://example.com/old-skill",
            "output": "暗影千机（2026）/通用/旧标题.md",
            "yuque_url": "https://www.yuque.com/sgyxy/cangyun/old-skill"
        }));
        fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let index = KnowledgeIndex::load(&fixture.root).unwrap();
        let response = index
            .search(
                &KnowledgeVersionContext::from_game_version(GameVersion::AnYingQianJi),
                query(KnowledgeVersionScope::CurrentOnly, "绝刀伤害机制盾飞"),
            )
            .unwrap();

        assert!(response.results[0].fact_eligible);
        let conflicted = response
            .results
            .iter()
            .find(|result| result.title == "绝刀机制（2025）")
            .unwrap();
        assert!(!conflicted.fact_eligible);
    }

    #[test]
    fn unsafe_manifest_paths_are_rejected() {
        let fixture = Fixture::create();
        let manifest_path = fixture.root.join(MANIFEST_FILE);
        let manifest = json!({"entries": [{
            "title": "escape",
            "season": "暗影千机（2026）",
            "category": "通用",
            "kind": "yuque_document",
            "source": "https://example.com/escape",
            "output": "../escape.md"
        }]});
        fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            KnowledgeIndex::load(&fixture.root).unwrap_err(),
            KnowledgeIndexError::UnsafePath(_)
        ));
    }

    #[test]
    fn configured_vault_obeys_real_version_boundaries() {
        let Some(root) = env::var_os(KNOWLEDGE_ROOT_ENV) else {
            return;
        };
        let index = KnowledgeIndex::load(Path::new(&root)).unwrap();
        assert!(index.chunk_count() > 100);
        assert_eq!(index.corpus_hash().len(), 64);

        for (version, season, text) in [
            (
                GameVersion::AnYingQianJi,
                "暗影千机（2026）",
                "盾飞 劫刀 流血",
            ),
            (GameVersion::ShanHaiYuanLiu, "山海源流（2025）", "盾飞 循环"),
        ] {
            let response = index
                .search(
                    &KnowledgeVersionContext::from_game_version(version),
                    query(KnowledgeVersionScope::CurrentOnly, text),
                )
                .unwrap();
            assert!(!response.results.is_empty(), "no result for {season}");
            assert!(response
                .results
                .iter()
                .all(|result| result.season == season));
        }

        let test_server = index
            .search(
                &KnowledgeVersionContext::from_game_version(GameVersion::AnYingQianJiTest),
                query(KnowledgeVersionScope::CurrentOnly, "暗影千机 体服 改动"),
            )
            .unwrap();
        assert!(!test_server.results.is_empty());
        assert!(test_server
            .results
            .iter()
            .all(|result| result.category == "130级"));

        let conflict = index
            .search(
                &KnowledgeVersionContext::from_game_version(GameVersion::AnYingQianJi),
                query(KnowledgeVersionScope::CurrentOnly, "山海源流盾飞循环"),
            )
            .unwrap_err();
        assert!(matches!(
            conflict,
            KnowledgeIndexError::VersionConflict { .. }
        ));
    }

    #[test]
    fn configured_vault_extracts_explicit_reference_entity() {
        let Some(root) = env::var_os(KNOWLEDGE_ROOT_ENV) else {
            return;
        };
        let index = KnowledgeIndex::load(Path::new(&root)).unwrap();
        let response = index
            .search(
                &KnowledgeVersionContext::from_game_version(GameVersion::AnYingQianJi),
                query(KnowledgeVersionScope::ReferenceLookup, "世一苍"),
            )
            .unwrap();
        let result = response.results.first().expect("reference result");

        assert!(result.title.contains("万灵当歌_ 苍云分山PVE指南"));
        assert!(result.exact_phrase_match);
        assert!(result
            .reference_entities
            .iter()
            .any(|entity| { entity.relation == "video_author" && entity.name == "dereck365" }));
    }

    #[test]
    fn configured_vault_emits_source_bound_domain_claims() {
        let Some(root) = env::var_os(KNOWLEDGE_ROOT_ENV) else {
            return;
        };
        let index = KnowledgeIndex::load(Path::new(&root)).unwrap();
        let indexed_claim_ids = index
            .chunks
            .iter()
            .flat_map(|chunk| {
                chunk
                    .domain_claims
                    .iter()
                    .map(|claim| claim.claim_id.as_str())
            })
            .collect::<BTreeSet<_>>();
        println!(
            "DOMAIN_CLAIMS {}",
            indexed_claim_ids
                .iter()
                .copied()
                .collect::<Vec<_>>()
                .join(",")
        );
        assert!(indexed_claim_ids.contains("fs-cw-002"));
        assert!(indexed_claim_ids.contains("fs-cw-002-patch"));
        assert!(indexed_claim_ids.contains("fs-charge-001"));
        let response = index
            .search_with_audience(
                &KnowledgeVersionContext::from_game_version(GameVersion::AnYingQianJi),
                KnowledgeSearchQuery {
                    query: "天下宏愿 裂伤 持续伤害 最多叠加3层".to_string(),
                    version_scope: KnowledgeVersionScope::CurrentOnly,
                    category: Some("白皮书".to_string()),
                    top_k: MAX_KNOWLEDGE_RESULTS,
                },
                KnowledgeAudience::from_question(
                    "旗舰端分山劲天下宏愿",
                    Some(KnowledgeMountScope::Fenshanjin),
                ),
            )
            .unwrap();

        assert_eq!(response.domain_index_hash.len(), 64);
        let (result, claim) = response
            .results
            .iter()
            .find_map(|result| {
                result
                    .domain_claims
                    .iter()
                    .find(|claim| claim.claim_id == "fs-cw-002")
                    .map(|claim| (result, claim))
            })
            .expect("current orange-weapon claim should be retrieved from the matching chunk");
        assert_eq!(claim.source.chunk_hash, result.chunk_hash);
        assert_eq!(claim.source.document_hash, result.document_hash);
        assert!(result
            .domain_relations
            .iter()
            .any(|relation| relation.claim_id == claim.claim_id));
    }

    #[test]
    fn configured_vault_domain_prefetches_recall_current_fact_evidence() {
        let Some(root) = env::var_os(KNOWLEDGE_ROOT_ENV) else {
            return;
        };
        let index = KnowledgeIndex::load(Path::new(&root)).unwrap();
        let runtime = AgentRuntime::fixture();
        let scenario = runtime.fixture_scenario();
        for question in [
            "分析当前循环输出基线。",
            "为什么这里空转？先定位断档再分析原因。",
            "水特效一键宏用206还是14156加速？",
            "橙武天下宏愿为什么少伤害，和业火怎么对轴？",
            "帮我分析当前一键宏的判定和手动循环相比牺牲了什么。",
            "英雄阆风悬城老四的业火应该怎么交？",
            "无界分山劲·悟循环怎么打？",
            "盾压重置率怎么算，这个公式是官方的吗？",
        ] {
            let plan = select_analysis_plan(question, &scenario);
            let prefetch = knowledge_prefetch(&plan, question).expect("domain prefetch");
            let response = index
                .search_with_audience(
                    &KnowledgeVersionContext::from_game_version(GameVersion::AnYingQianJi),
                    KnowledgeSearchQuery {
                        query: prefetch.query,
                        version_scope: KnowledgeVersionScope::CurrentOnly,
                        category: prefetch.category,
                        top_k: MAX_KNOWLEDGE_RESULTS,
                    },
                    KnowledgeAudience::from_question(
                        question,
                        Some(KnowledgeMountScope::Fenshanjin),
                    ),
                )
                .unwrap();
            assert!(
                response.results.iter().any(|result| result.fact_eligible),
                "domain prefetch returned no current fact evidence for {question}"
            );
            let recalled_text = response
                .results
                .iter()
                .map(|result| format!("{} {} {}", result.heading, result.title, result.snippet))
                .collect::<Vec<_>>()
                .join("\n");
            if question.contains("老四") {
                assert!(recalled_text.contains("提前倒数10秒"));
            }
            if question.contains("无界") {
                assert!(recalled_text.contains("手打血劫") || recalled_text.contains("劫刀×9"));
            }
        }
    }

    #[test]
    fn configured_vault_prints_versioned_preview_queries() {
        let Some(root) = env::var_os(KNOWLEDGE_ROOT_ENV) else {
            return;
        };
        let index = KnowledgeIndex::load(Path::new(&root)).unwrap();
        println!(
            "KNOWLEDGE_INDEX {}",
            serde_json::to_string(&json!({
                "schema_version": index.schema_version(),
                "corpus_hash": index.corpus_hash(),
                "documents": index.document_count(),
                "chunks": index.chunk_count(),
                "seasons": index.seasons().collect::<Vec<_>>(),
                "categories": index.categories().collect::<Vec<_>>()
            }))
            .unwrap()
        );
        let cases = vec![
            (
                "current_rotation",
                GameVersion::AnYingQianJi,
                KnowledgeVersionScope::CurrentOnly,
                "盾飞期间劫刀数量和断流血风险",
                None,
            ),
            (
                "current_equipment",
                GameVersion::AnYingQianJi,
                KnowledgeVersionScope::CurrentOnly,
                "当前赛季苍云配装破招",
                Some("白皮书"),
            ),
            (
                "current_tank_raid",
                GameVersion::AnYingQianJi,
                KnowledgeVersionScope::CurrentOnly,
                "铁骨衣主流副本打法",
                Some("实战"),
            ),
            (
                "current_latency",
                GameVersion::AnYingQianJi,
                KnowledgeVersionScope::CurrentOnly,
                "网络延迟按键FPS设置",
                Some("通用"),
            ),
            (
                "current_skill",
                GameVersion::AnYingQianJi,
                KnowledgeVersionScope::CurrentOnly,
                "绝刀伤害机制",
                None,
            ),
            (
                "shanhai_rotation",
                GameVersion::ShanHaiYuanLiu,
                KnowledgeVersionScope::CurrentOnly,
                "山海源流苍云紫武木桩循环",
                Some("基础"),
            ),
            (
                "explicit_taiji",
                GameVersion::AnYingQianJi,
                KnowledgeVersionScope::SpecificSeason {
                    season: "太极秘录（2025）".to_string(),
                },
                "太极秘录分山劲白皮书",
                Some("白皮书"),
            ),
            (
                "cross_version_changes",
                GameVersion::AnYingQianJi,
                KnowledgeVersionScope::CrossVersion,
                "苍云历次技改历史演变",
                None,
            ),
            (
                "test_server_changes",
                GameVersion::AnYingQianJiTest,
                KnowledgeVersionScope::CurrentOnly,
                "暗影千机体服改动",
                Some("130级"),
            ),
            (
                "current_macro",
                GameVersion::AnYingQianJi,
                KnowledgeVersionScope::CurrentOnly,
                "暗影千机分山一键宏",
                Some("宏"),
            ),
        ];
        for (id, version, scope, text, category) in cases {
            let response = index
                .search(
                    &KnowledgeVersionContext::from_game_version(version),
                    KnowledgeSearchQuery {
                        query: text.to_string(),
                        version_scope: scope,
                        category: category.map(str::to_string),
                        top_k: 3,
                    },
                )
                .unwrap();
            assert!(
                !response.results.is_empty(),
                "no result for preview case {id}"
            );
            let expected_top = match id {
                "current_rotation" | "current_equipment" | "current_skill" => "白皮书",
                "current_tank_raid" => "铁骨主流",
                "current_latency" => "低延迟",
                "shanhai_rotation" => "山海源流",
                "explicit_taiji" => "太极秘录",
                "cross_version_changes" => "历次技改",
                "test_server_changes" => "暗影千机",
                "current_macro" => "一键宏",
                _ => unreachable!(),
            };
            assert!(
                response.results[0].title.contains(expected_top),
                "unexpected top result for {id}: {}",
                response.results[0].title
            );
            assert_eq!(
                response.results[0].fact_eligible,
                id != "shanhai_rotation",
                "unexpected fact eligibility for {id}"
            );
            println!(
                "KNOWLEDGE_PREVIEW {}",
                serde_json::to_string(&json!({
                    "id": id,
                    "query": text,
                    "current_season": response.current_season,
                    "results": response.results.iter().map(|result| json!({
                        "title": result.title,
                        "season": result.season,
                        "category": result.category,
                        "version_match": result.version_match,
                        "fact_eligible": result.fact_eligible,
                        "source_url": result.source_url,
                        "score": result.score
                    })).collect::<Vec<_>>()
                }))
                .unwrap()
            );
        }
    }

    #[derive(Debug, Deserialize)]
    struct KnowledgeEvalSuite {
        schema_version: String,
        minimum_recall_at_5: f64,
        cases: Vec<KnowledgeEvalCase>,
    }

    #[derive(Debug, Deserialize)]
    struct KnowledgeEvalCase {
        id: String,
        game_version: String,
        scope: String,
        query: String,
        #[serde(default)]
        season: Option<String>,
        #[serde(default)]
        category: Option<String>,
        #[serde(default)]
        expected_titles_any: Vec<String>,
        #[serde(default)]
        expected_version_match: Option<String>,
        #[serde(default)]
        expected_fact_eligible: Option<bool>,
        #[serde(default)]
        allowed_seasons: Vec<String>,
        #[serde(default)]
        allowed_categories: Vec<String>,
        #[serde(default)]
        forbidden_title_contains: Vec<String>,
        #[serde(default)]
        expected_empty: bool,
        #[serde(default)]
        expect_error: Option<String>,
    }

    #[test]
    fn configured_vault_fixed_retrieval_eval() {
        let Some(root) = env::var_os(KNOWLEDGE_ROOT_ENV) else {
            return;
        };
        let index = if env::var(KNOWLEDGE_RETRIEVAL_ENV)
            .is_ok_and(|mode| mode.eq_ignore_ascii_case("embedded"))
        {
            KnowledgeIndex::from_env().unwrap()
        } else {
            KnowledgeIndex::load(Path::new(&root)).unwrap()
        };
        let suite: KnowledgeEvalSuite =
            serde_json::from_str(include_str!("../../tests/agent_knowledge_eval/cases.json"))
                .unwrap();
        assert_eq!(suite.schema_version, "agent-knowledge-eval/v1");

        let mut recall_cases = 0usize;
        let mut recall_hits = 0usize;
        let mut safety_cases = 0usize;
        for case in &suite.cases {
            let context =
                KnowledgeVersionContext::from_game_version(match case.game_version.as_str() {
                    "anying" => GameVersion::AnYingQianJi,
                    "shanhai" => GameVersion::ShanHaiYuanLiu,
                    "anying_test" => GameVersion::AnYingQianJiTest,
                    other => panic!("unknown game_version {other} in {}", case.id),
                });
            let version_scope = match case.scope.as_str() {
                "current_only" => KnowledgeVersionScope::CurrentOnly,
                "specific_season" => KnowledgeVersionScope::SpecificSeason {
                    season: case
                        .season
                        .clone()
                        .unwrap_or_else(|| panic!("missing season in {}", case.id)),
                },
                "cross_version" => KnowledgeVersionScope::CrossVersion,
                "reference_lookup" => KnowledgeVersionScope::ReferenceLookup,
                other => panic!("unknown scope {other} in {}", case.id),
            };
            let outcome = index.search(
                &context,
                KnowledgeSearchQuery {
                    query: case.query.clone(),
                    version_scope,
                    category: case.category.clone(),
                    top_k: 5,
                },
            );

            if let Some(expected_error) = case.expect_error.as_deref() {
                safety_cases += 1;
                let actual = outcome
                    .as_ref()
                    .err()
                    .map(knowledge_eval_error_code)
                    .unwrap_or("none");
                assert_eq!(actual, expected_error, "unexpected error in {}", case.id);
                println!("KNOWLEDGE_EVAL_CASE {} error={actual} pass=true", case.id);
                continue;
            }

            let response = outcome.unwrap_or_else(|error| {
                panic!("unexpected search failure in {}: {error}", case.id)
            });
            if case.expected_empty {
                safety_cases += 1;
                assert!(
                    response.results.is_empty(),
                    "expected no answer in {}, got {}",
                    case.id,
                    response
                        .results
                        .iter()
                        .map(|result| format!(
                            "{} (lexical={:?}, dense={:?}, fused={})",
                            result.title,
                            result.lexical_score,
                            result.dense_similarity,
                            result.score
                        ))
                        .collect::<Vec<_>>()
                        .join(" | ")
                );
            }
            if !case.allowed_seasons.is_empty() {
                safety_cases += 1;
                assert!(response
                    .results
                    .iter()
                    .all(|result| case.allowed_seasons.contains(&result.season)));
            }
            if !case.allowed_categories.is_empty() {
                safety_cases += 1;
                assert!(response
                    .results
                    .iter()
                    .all(|result| case.allowed_categories.contains(&result.category)));
            }
            for forbidden in &case.forbidden_title_contains {
                safety_cases += 1;
                assert!(
                    response
                        .results
                        .iter()
                        .all(|result| !result.title.contains(forbidden)),
                    "forbidden result in {}: {forbidden}",
                    case.id
                );
            }
            assert!(response.results.iter().all(|result| {
                safe_knowledge_eval_url(&result.source_url)
                    && safe_knowledge_eval_url(&result.yuque_url)
            }));

            if !case.expected_titles_any.is_empty() {
                recall_cases += 1;
                let matched = response.results.iter().find(|result| {
                    case.expected_titles_any
                        .iter()
                        .any(|expected| result.title.contains(expected))
                });
                if let Some(result) = matched {
                    recall_hits += 1;
                    if let Some(expected) = case.expected_version_match.as_deref() {
                        assert_eq!(
                            knowledge_eval_version_match(result.version_match),
                            expected,
                            "version mismatch in {}",
                            case.id
                        );
                    }
                    if let Some(expected) = case.expected_fact_eligible {
                        assert_eq!(
                            result.fact_eligible, expected,
                            "fact eligibility mismatch in {}",
                            case.id
                        );
                    }
                    println!(
                        "KNOWLEDGE_EVAL_CASE {} rank={} title={} pass=true",
                        case.id,
                        response
                            .results
                            .iter()
                            .position(|candidate| candidate.document_id == result.document_id)
                            .unwrap()
                            + 1,
                        result.title
                    );
                } else {
                    println!(
                        "KNOWLEDGE_EVAL_CASE {} pass=false returned={}",
                        case.id,
                        response
                            .results
                            .iter()
                            .map(|result| result.title.as_str())
                            .collect::<Vec<_>>()
                            .join(" | ")
                    );
                }
            } else {
                println!("KNOWLEDGE_EVAL_CASE {} pass=true", case.id);
            }
        }

        let recall_at_5 = recall_hits as f64 / recall_cases as f64;
        println!(
            "KNOWLEDGE_EVAL_SUMMARY {}",
            serde_json::to_string(&json!({
                "schema_version": suite.schema_version,
                "corpus_hash": index.corpus_hash(),
                "documents": index.document_count(),
                "chunks": index.chunk_count(),
                "retrieval": index.retrieval_info(),
                "cases": suite.cases.len(),
                "recall_cases": recall_cases,
                "recall_hits": recall_hits,
                "recall_at_5": round_score(recall_at_5),
                "minimum_recall_at_5": suite.minimum_recall_at_5,
                "safety_assertions": safety_cases
            }))
            .unwrap()
        );
        assert!(
            recall_at_5 >= suite.minimum_recall_at_5,
            "Recall@5 {:.3} is below {:.3}",
            recall_at_5,
            suite.minimum_recall_at_5
        );
    }

    #[test]
    fn configured_vault_hybrid_recovers_a_semantic_rotation_query() {
        let Some(root) = env::var_os(KNOWLEDGE_ROOT_ENV) else {
            return;
        };
        if !env::var(KNOWLEDGE_RETRIEVAL_ENV)
            .is_ok_and(|mode| mode.eq_ignore_ascii_case("embedded"))
        {
            return;
        }
        let lexical = KnowledgeIndex::load(Path::new(&root)).unwrap();
        let hybrid = KnowledgeIndex::from_env().unwrap();
        let context = KnowledgeVersionContext::from_game_version(GameVersion::AnYingQianJi);
        let request = KnowledgeSearchQuery {
            query: "怎样减少战斗中的空转".to_string(),
            version_scope: KnowledgeVersionScope::CurrentOnly,
            category: None,
            top_k: 5,
        };
        let lexical_results = lexical.search(&context, request.clone()).unwrap();
        let hybrid_results = hybrid.search(&context, request).unwrap();
        let expected_title = "英雄及挑战阆风悬城_ 分山实战技巧";
        assert!(
            lexical_results
                .results
                .iter()
                .all(|result| !result.title.contains(expected_title)),
            "comparison query no longer distinguishes the two retrieval modes"
        );
        let recovered = hybrid_results
            .results
            .iter()
            .find(|result| result.title.contains(expected_title))
            .expect("hybrid retrieval should recover the semantic rotation result");
        assert!(recovered
            .dense_similarity
            .is_some_and(|score| score >= 0.55));
        println!(
            "HYBRID_GAIN query={} recovered={} dense_similarity={:?}",
            "怎样减少战斗中的空转", recovered.title, recovered.dense_similarity
        );
    }

    #[test]
    fn reciprocal_rank_fusion_rewards_agreement_without_requiring_both_channels() {
        let lexical_only = reciprocal_rank_fusion(Some(1), None);
        let dense_only = reciprocal_rank_fusion(None, Some(1));
        let agreed = reciprocal_rank_fusion(Some(1), Some(1));
        assert!(agreed > lexical_only);
        assert!(lexical_only > dense_only);
        assert!(dense_only > 0.0);
    }

    fn knowledge_eval_error_code(error: &KnowledgeIndexError) -> &'static str {
        match error {
            KnowledgeIndexError::VersionConflict { .. } => "version_conflict",
            KnowledgeIndexError::CrossVersionIntentRequired => "cross_version_intent_required",
            KnowledgeIndexError::InvalidQuery(_) => "invalid_query",
            KnowledgeIndexError::UnknownSeason(_) => "unknown_season",
            KnowledgeIndexError::UnknownCategory(_) => "unknown_category",
            _ => "unexpected_error",
        }
    }

    fn knowledge_eval_version_match(value: KnowledgeVersionMatch) -> &'static str {
        match value {
            KnowledgeVersionMatch::CurrentExact => "current_exact",
            KnowledgeVersionMatch::TestServerExact => "test_server_exact",
            KnowledgeVersionMatch::HistoricalExplicit => "historical_explicit",
            KnowledgeVersionMatch::CrossVersion => "cross_version",
            KnowledgeVersionMatch::ReferenceOnly => "reference_only",
        }
    }

    fn safe_knowledge_eval_url(value: &str) -> bool {
        value.starts_with("https://") || value.starts_with("http://")
    }
}
