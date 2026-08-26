use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::env;
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::GameVersion;

pub const KNOWLEDGE_INDEX_SCHEMA_V1: &str = "agent-knowledge-index/v1";
pub const KNOWLEDGE_SEARCH_SCHEMA_V1: &str = "agent-knowledge-search/v1";
pub const KNOWLEDGE_ROOT_ENV: &str = "JX3_KNOWLEDGE_ROOT";
const MANIFEST_FILE: &str = "_migration-manifest.json";
const MAX_DOCUMENTS: usize = 5_000;
const MAX_DOCUMENT_BYTES: u64 = 8 * 1024 * 1024;
const MAX_CORPUS_BYTES: u64 = 64 * 1024 * 1024;
const MAX_QUERY_CHARACTERS: usize = 200;
const MAX_TOP_K: usize = 5;
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum KnowledgeVersionScope {
    CurrentOnly,
    SpecificSeason { season: String },
    CrossVersion,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeSearchResponse {
    pub schema_version: String,
    pub corpus_hash: String,
    pub current_season: String,
    pub requested_scope: KnowledgeVersionScope,
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
}

#[derive(Debug, Clone)]
pub struct KnowledgeIndex {
    schema_version: &'static str,
    corpus_hash: String,
    chunks: Vec<KnowledgeChunk>,
    document_frequency: HashMap<String, usize>,
    average_document_length: f64,
    seasons: BTreeSet<String>,
    categories: BTreeSet<String>,
}

impl KnowledgeIndex {
    pub fn from_env() -> Result<Self, KnowledgeIndexError> {
        let root = env::var_os(KNOWLEDGE_ROOT_ENV).ok_or(KnowledgeIndexError::NotConfigured)?;
        Self::load(Path::new(&root))
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
            let version_warning = title_season_warning(&entry.title, &entry.season);
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
                });
            }
        }

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
            chunks,
            document_frequency,
            average_document_length,
            seasons,
            categories,
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

    pub fn search(
        &self,
        context: &KnowledgeVersionContext,
        query: KnowledgeSearchQuery,
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
        let test_server_release_hint = match query.version_scope {
            KnowledgeVersionScope::CurrentOnly
                if context.game_version == GameVersion::AnYingQianJiTest
                    && query_mentions_season(&query.query, "暗影千机（2026）") =>
            {
                Some("暗影千机")
            }
            _ => None,
        };
        let mut ranked = Vec::new();
        for chunk in &self.chunks {
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
            let mut score = self.bm25_score(chunk, &query_terms);
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
            if score <= 0.0 || !score.is_finite() {
                continue;
            }
            ranked.push((score, version_match, chunk));
        }
        ranked.sort_by(|left, right| {
            fact_eligible(right.2)
                .cmp(&fact_eligible(left.2))
                .then_with(|| right.0.partial_cmp(&left.0).unwrap_or(Ordering::Equal))
                .then_with(|| left.2.title.cmp(&right.2.title))
                .then_with(|| left.2.heading.cmp(&right.2.heading))
        });

        let mut seen_documents = HashSet::new();
        let results = ranked
            .into_iter()
            .filter(|(_, _, chunk)| seen_documents.insert(chunk.document_id.clone()))
            .take(query.top_k)
            .map(|(score, version_match, chunk)| KnowledgeSearchResult {
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
            })
            .collect();

        Ok(KnowledgeSearchResponse {
            schema_version: KNOWLEDGE_SEARCH_SCHEMA_V1.to_string(),
            corpus_hash: self.corpus_hash.clone(),
            current_season: context.current_season.to_string(),
            requested_scope: query.version_scope,
            results,
        })
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
    if !(1..=MAX_TOP_K).contains(&query.top_k) {
        return Err(KnowledgeIndexError::InvalidQuery(
            "top_k must be within 1..=5",
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

fn title_season_warning(title: &str, season: &str) -> Option<String> {
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
                    "山海源流（2025）/基础/旧版循环.md",
                    "旧版循环（2025）",
                    "山海源流（2025）",
                    "基础",
                    "external_mirror",
                    "full",
                    "# 旧版循环\n\n山海源流的盾飞循环使用旧版劫刀节奏。",
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
}
