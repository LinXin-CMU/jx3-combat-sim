use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

use super::knowledge::KnowledgeAudience;

pub const DOMAIN_TERM_SCHEMA_V1: &str = "agent-domain-term/v1";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum DomainTermKindV1 {
    DefinedTerm,
    RotationShorthand,
    NumericCode,
    Acronym,
    Colloquial,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DomainTermSourceV1 {
    pub document_id: String,
    pub title: String,
    pub season: String,
    pub category: String,
    pub heading: String,
    pub source_url: String,
    pub yuque_url: String,
    pub source_updated_at: String,
    pub document_hash: String,
    pub chunk_hash: String,
    pub excerpt: String,
    #[serde(default)]
    pub audience: KnowledgeAudience,
    #[serde(default)]
    pub fact_eligible: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DomainTermCardV1 {
    pub schema_version: String,
    pub term_id: String,
    pub surface: String,
    pub aliases: Vec<String>,
    pub kind: DomainTermKindV1,
    /// A source-authored definition when one exists; otherwise a compact usage
    /// context. The basis is explicit so downstream models cannot mistake an
    /// occurrence excerpt for a canonical definition.
    pub meaning: String,
    pub meaning_basis: String,
    pub season: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub audience: KnowledgeAudience,
    #[serde(default)]
    pub fact_eligible: bool,
    pub occurrence_count: usize,
    pub document_count: usize,
    pub confidence: String,
    pub sources: Vec<DomainTermSourceV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResolvedDomainTermV1 {
    pub matched_surface: String,
    pub resolution: String,
    pub cards: Vec<DomainTermCardV1>,
}

#[derive(Debug, Clone)]
pub struct DomainTermChunkContext<'a> {
    pub document_id: &'a str,
    pub title: &'a str,
    pub season: &'a str,
    pub category: &'a str,
    pub heading: &'a str,
    pub text: &'a str,
    pub source_url: &'a str,
    pub yuque_url: &'a str,
    pub source_updated_at: &'a str,
    pub document_hash: &'a str,
    pub chunk_hash: &'a str,
    pub audience: KnowledgeAudience,
    pub fact_eligible: bool,
}

#[derive(Debug, Clone)]
struct TermOccurrence {
    surface: String,
    kind: DomainTermKindV1,
    definition: Option<String>,
    explicit_definition: bool,
    occurrence_count: usize,
    source: DomainTermSourceV1,
}

#[derive(Debug, Clone, Default)]
pub struct DomainTerminologyIndexV1 {
    cards: Vec<DomainTermCardV1>,
    index_hash: String,
}

impl DomainTerminologyIndexV1 {
    pub fn build<'a>(chunks: impl IntoIterator<Item = DomainTermChunkContext<'a>>) -> Self {
        let mut occurrences = Vec::new();
        for chunk in chunks {
            if chunk.fact_eligible {
                occurrences.extend(extract_occurrences(&chunk));
            }
        }

        let mut groups = BTreeMap::<(String, String, KnowledgeAudience, String), Vec<TermOccurrence>>::new();
        for occurrence in occurrences {
            groups
                .entry((
                    occurrence.surface.clone(),
                    occurrence.source.season.clone(),
                    occurrence.source.audience,
                    occurrence.source.category.clone(),
                ))
                .or_default()
                .push(occurrence);
        }

        let mut cards = groups
            .into_iter()
            .filter_map(|((surface, season, audience, category), mut items)| {
                let explicit = items.iter().any(|item| item.explicit_definition);
                let documents = items
                    .iter()
                    .map(|item| item.source.document_id.as_str())
                    .collect::<BTreeSet<_>>();
                let document_count = documents.len();
                let mut chunk_occurrences = BTreeMap::new();
                for item in &items {
                    chunk_occurrences.entry((
                        item.source.document_id.as_str(),
                        item.source.chunk_hash.as_str(),
                    )).or_insert(item.occurrence_count);
                }
                let occurrence_count = chunk_occurrences.values().sum::<usize>();
                let strongest_kind = items
                    .iter()
                    .map(|item| item.kind)
                    .min()
                    .unwrap_or(DomainTermKindV1::Colloquial);
                let admitted = explicit
                    || document_count >= 2
                    || matches!(strongest_kind,
                        DomainTermKindV1::RotationShorthand
                            | DomainTermKindV1::NumericCode
                            | DomainTermKindV1::Acronym
                    ) && occurrence_count >= 2;
                if !admitted {
                    return None;
                }
                items.sort_by(|left, right| {
                    right
                        .explicit_definition
                        .cmp(&left.explicit_definition)
                        .then_with(|| left.source.title.cmp(&right.source.title))
                        .then_with(|| left.source.heading.cmp(&right.source.heading))
                });
                let explicit_meaning = items
                    .iter()
                    .find_map(|item| item.definition.clone());
                let meaning_basis = if explicit_meaning.is_some() {
                    "source_definition"
                } else {
                    "representative_usage"
                };
                let meaning = explicit_meaning
                    .unwrap_or_else(|| items[0].source.excerpt.clone());
                let mut seen_chunks = BTreeSet::new();
                let sources = items
                    .iter()
                    .filter(|item| seen_chunks.insert((
                        item.source.document_id.clone(), item.source.chunk_hash.clone(),
                    )))
                    .take(4)
                    .map(|item| item.source.clone())
                    .collect::<Vec<_>>();
                let mut aliases = numeric_aliases(&surface);
                if meaning_basis == "source_definition" {
                    aliases.extend(explicit_aliases(&meaning, &surface));
                }
                aliases.sort();
                aliases.dedup();
                let identity = format!("{season}\0{audience:?}\0{category}\0{surface}\0{meaning_basis}\0{meaning}");
                Some(DomainTermCardV1 {
                    schema_version: DOMAIN_TERM_SCHEMA_V1.to_string(),
                    term_id: sha256(identity.as_bytes()),
                    surface,
                    aliases,
                    kind: strongest_kind,
                    meaning,
                    meaning_basis: meaning_basis.to_string(),
                    season,
                    category,
                    audience,
                    fact_eligible: true,
                    occurrence_count,
                    document_count,
                    confidence: if explicit || document_count >= 3 {
                        "high"
                    } else {
                        "medium"
                    }
                    .to_string(),
                    sources,
                })
            })
            .collect::<Vec<_>>();
        cards.sort_by(|left, right| {
            left.surface
                .cmp(&right.surface)
                .then_with(|| left.season.cmp(&right.season))
        });
        let mut hasher = Sha256::new();
        for card in &cards {
            hasher.update(card.term_id.as_bytes());
            hasher.update([0]);
            for source in &card.sources {
                hasher.update(source.chunk_hash.as_bytes());
                hasher.update([0]);
            }
        }
        Self {
            cards,
            index_hash: format!("{:x}", hasher.finalize()),
        }
    }

    pub fn index_hash(&self) -> &str {
        &self.index_hash
    }

    pub fn card_count(&self) -> usize {
        self.cards.len()
    }

    pub fn resolve(&self, question: &str, current_season: &str) -> Vec<ResolvedDomainTermV1> {
        let audience = KnowledgeAudience::from_question(question, None);
        self.resolve_filtered(question, current_season, |card| {
            card.season == current_season && audience.accepts(card.audience)
        })
    }

    pub fn resolve_filtered(
        &self,
        question: &str,
        current_season: &str,
        accepts: impl Fn(&DomainTermCardV1) -> bool,
    ) -> Vec<ResolvedDomainTermV1> {
        let normalized = normalize(question);
        let mut matched = BTreeMap::<String, Vec<DomainTermCardV1>>::new();
        for card in &self.cards {
            if !card.fact_eligible || !accepts(card) {
                continue;
            }
            let surface = normalize(&card.surface);
            let alias_match = card
                .aliases
                .iter()
                .any(|alias| normalized.contains(&normalize(alias)));
            if normalized.contains(&surface) || alias_match {
                matched
                    .entry(card.surface.clone())
                    .or_default()
                    .push(card.clone());
            }
        }
        let mut resolved = matched
            .into_iter()
            .map(|(surface, mut cards)| {
                cards.sort_by_key(|card| usize::from(card.season != current_season));
                let current_count = cards
                    .iter()
                    .filter(|card| card.season == current_season)
                    .count();
                let resolution = match (current_count, cards.len()) {
                    (1, 1) => "current_scope",
                    (1, _) => "current_scope_with_historical_variants",
                    (0, 1) => "historical_only",
                    (count, _) if count > 1 => "current_scope_ambiguous",
                    _ => "historical_ambiguous",
                };
                let best = cards.first().expect("term group is non-empty");
                let score = resolution_score(best, current_season, question);
                (
                    score,
                    ResolvedDomainTermV1 {
                        matched_surface: surface,
                        resolution: resolution.to_string(),
                        cards: cards.into_iter().take(4).collect(),
                    },
                )
            })
            .collect::<Vec<_>>();
        resolved.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| right.1.matched_surface.chars().count().cmp(&left.1.matched_surface.chars().count()))
                .then_with(|| left.1.matched_surface.cmp(&right.1.matched_surface))
        });
        let strongest = resolved.first().map(|item| item.0).unwrap_or_default();
        resolved
            .into_iter()
            .filter(|(score, _)| *score + 18 >= strongest)
            .take(8)
            .map(|(_, item)| item)
            .collect()
    }
}

fn resolution_score(card: &DomainTermCardV1, current_season: &str, question: &str) -> usize {
    let scope = if card.season == current_season { 40 } else { 0 };
    let kind = match card.kind {
        DomainTermKindV1::DefinedTerm => 36,
        DomainTermKindV1::RotationShorthand => 32,
        DomainTermKindV1::NumericCode | DomainTermKindV1::Acronym => 28,
        DomainTermKindV1::Colloquial => 12,
    };
    let exact = usize::from(normalize(question) == normalize(&card.surface)) * 24;
    scope + kind + exact + card.surface.chars().count().min(16)
}

fn extract_occurrences(context: &DomainTermChunkContext<'_>) -> Vec<TermOccurrence> {
    let mut candidates = Vec::<(String, DomainTermKindV1, Option<String>, bool)>::new();
    for (open, close) in [('「', '」'), ('“', '”'), ('《', '》'), ('【', '】')] {
        for surface in delimited_spans(context.text, open, close) {
            let kind = classify_surface(&surface);
            candidates.push((surface, kind, None, false));
        }
    }
    for sentence in context.text.split(['。', '！', '？', '\n']) {
        candidates.extend(explicit_definition_candidates(sentence));
        for token in ascii_runs(sentence) {
            if is_numeric_code(&token) {
                candidates.push((token, DomainTermKindV1::NumericCode, None, false));
            } else if is_acronym(&token) {
                candidates.push((token, DomainTermKindV1::Acronym, None, false));
            }
        }
        for token in sentence.split(|character: char| {
            character.is_whitespace()
                || matches!(character, '，' | ',' | '；' | ';' | '：' | ':' | '（' | '）' | '(' | ')' | '/' | '|')
        }) {
            let token = trim_term(token);
            if is_rotation_shorthand(token) {
                candidates.push((
                    token.to_string(),
                    DomainTermKindV1::RotationShorthand,
                    None,
                    false,
                ));
            } else if is_numeric_code(token) {
                candidates.push((
                    token.to_string(),
                    DomainTermKindV1::NumericCode,
                    None,
                    false,
                ));
            } else if is_acronym(token) {
                candidates.push((
                    token.to_string(),
                    DomainTermKindV1::Acronym,
                    None,
                    false,
                ));
            }
        }
    }
    let mut unique = BTreeSet::new();
    candidates
        .into_iter()
        .filter(|(surface, _, _, _)| valid_surface(surface))
        .filter(|(surface, kind, definition, explicit)| {
            unique.insert((surface.clone(), *kind, definition.clone(), *explicit))
        })
        .map(|(surface, kind, definition, explicit_definition)| TermOccurrence {
            occurrence_count: context.text.matches(&surface).count().max(1),
            source: DomainTermSourceV1 {
                document_id: context.document_id.to_string(),
                title: context.title.to_string(),
                season: context.season.to_string(),
                category: context.category.to_string(),
                heading: context.heading.to_string(),
                source_url: context.source_url.to_string(),
                yuque_url: context.yuque_url.to_string(),
                source_updated_at: context.source_updated_at.to_string(),
                document_hash: context.document_hash.to_string(),
                chunk_hash: context.chunk_hash.to_string(),
                excerpt: excerpt_around(context.text, &surface),
                audience: context.audience,
                fact_eligible: context.fact_eligible,
            },
            surface,
            kind,
            definition,
            explicit_definition,
        })
        .collect()
}

fn ascii_runs(value: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            current.push(character);
        } else if !current.is_empty() {
            result.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        result.push(current);
    }
    result
}

fn explicit_definition_candidates(
    sentence: &str,
) -> Vec<(String, DomainTermKindV1, Option<String>, bool)> {
    let mut result = Vec::new();
    for marker in ["也就是", "指的是"] {
        let Some((left, right)) = sentence.split_once(marker) else {
            continue;
        };
        let surface = explicit_subject(left);
        let definition = leading_definition(right);
        if valid_surface(&surface) && definition.chars().count() >= 2 {
            result.push((
                surface,
                DomainTermKindV1::DefinedTerm,
                Some(definition),
                true,
            ));
        }
    }
    if let Some((left, right)) = sentence.split_once('指') {
        let surface = direct_definition_subject(left);
        let definition = leading_definition(right.trim_start_matches("的是"));
        if valid_surface(&surface) && definition.chars().count() >= 2 {
            result.push((
                surface,
                DomainTermKindV1::DefinedTerm,
                Some(definition),
                true,
            ));
        }
    }
    for marker in ["简称", "俗称", "统称为", "称为", "叫做"] {
        let Some((definition, right)) = sentence.split_once(marker) else {
            continue;
        };
        let surface = leading_term(right);
        let definition = trailing_definition(definition);
        if valid_surface(&surface) && definition.chars().count() >= 2 {
            result.push((
                surface,
                DomainTermKindV1::DefinedTerm,
                Some(definition),
                true,
            ));
        }
    }
    result
}

fn delimited_spans(text: &str, open: char, close: char) -> Vec<String> {
    let mut result = Vec::new();
    let mut remainder = text;
    while let Some(start) = remainder.find(open) {
        let inside = &remainder[start + open.len_utf8()..];
        let Some(end) = inside.find(close) else {
            break;
        };
        let surface = trim_term(&inside[..end]);
        if valid_surface(surface) {
            result.push(surface.to_string());
        }
        remainder = &inside[end + close.len_utf8()..];
    }
    result
}

fn classify_surface(surface: &str) -> DomainTermKindV1 {
    if is_rotation_shorthand(surface) {
        DomainTermKindV1::RotationShorthand
    } else if is_numeric_code(surface) {
        DomainTermKindV1::NumericCode
    } else if is_acronym(surface) {
        DomainTermKindV1::Acronym
    } else {
        DomainTermKindV1::Colloquial
    }
}

fn is_rotation_shorthand(value: &str) -> bool {
    let count = value.chars().count();
    if !(3..=18).contains(&count) || !value.chars().all(is_cjk) {
        return false;
    }
    let distinct = value.chars().collect::<BTreeSet<_>>().len();
    distinct <= 8
        && value
            .chars()
            .collect::<Vec<_>>()
            .windows(2)
            .any(|pair| pair[0] == pair[1])
}

fn is_numeric_code(value: &str) -> bool {
    (3..=6).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn is_acronym(value: &str) -> bool {
    (2..=8).contains(&value.len())
        && value.bytes().all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        && value.bytes().any(|byte| byte.is_ascii_uppercase())
}

fn valid_surface(value: &str) -> bool {
    let count = value.chars().count();
    (2..=20).contains(&count)
        && !value.chars().any(char::is_control)
        && !value.contains("http")
        && !value.contains('=')
        && !value.contains('.')
        && value.chars().any(|character| is_cjk(character) || character.is_ascii_alphanumeric())
}

fn is_cjk(character: char) -> bool {
    matches!(character as u32, 0x3400..=0x9fff)
}

fn trailing_phrase(value: &str) -> String {
    let value = value
        .rsplit(['，', ',', '；', ';', '：', ':', '（', '(', ' '])
        .next()
        .unwrap_or(value);
    let chars = trim_term(value).chars().collect::<Vec<_>>();
    chars[chars.len().saturating_sub(16)..].iter().collect()
}

fn explicit_subject(value: &str) -> String {
    let surface = trailing_phrase(value);
    let normalized = trim_term(&surface)
        .trim_start_matches("所谓")
        .trim_start_matches("这里的")
        .trim_start_matches("其中的")
        .trim();
    let count = normalized.chars().count();
    if !(2..=12).contains(&count)
        || ["这个", "这种", "这些", "它们", "其中", "也", "都"]
            .contains(&normalized)
    {
        String::new()
    } else {
        normalized.to_string()
    }
}

fn direct_definition_subject(value: &str) -> String {
    let normalized = trim_term(value);
    if normalized.chars().count() > 12
        || normalized
            .chars()
            .any(|character| matches!(character, '，' | ',' | '；' | ';' | '：' | ':' | '（' | '('))
    {
        String::new()
    } else {
        normalized.to_string()
    }
}

fn leading_term(value: &str) -> String {
    trim_term(value)
        .split(['，', ',', '；', ';', '：', ':', '（', '(', ' '])
        .next()
        .unwrap_or_default()
        .chars()
        .take(16)
        .collect()
}

fn leading_definition(value: &str) -> String {
    trim_term(value)
        .split(['，', ',', '；', ';'])
        .next()
        .unwrap_or_default()
        .chars()
        .take(80)
        .collect()
}

fn trailing_definition(value: &str) -> String {
    let chars = trim_term(value).chars().collect::<Vec<_>>();
    chars[chars.len().saturating_sub(80)..].iter().collect()
}

fn trim_term(value: &str) -> &str {
    value.trim_matches(|character: char| {
        character.is_whitespace()
            || matches!(
                character,
                '`' | '*' | '_' | '-' | '—' | '·' | '。' | '！' | '？' | '"' | '\'' | '「' | '」' | '“' | '”' | '《' | '》' | '【' | '】'
            )
    })
}

fn excerpt_around(text: &str, surface: &str) -> String {
    let Some(byte_index) = text.find(surface) else {
        return text.chars().take(180).collect();
    };
    let prefix_chars = text[..byte_index].chars().count();
    let chars = text.chars().collect::<Vec<_>>();
    let start = prefix_chars.saturating_sub(60);
    let end = (prefix_chars + surface.chars().count() + 120).min(chars.len());
    chars[start..end].iter().collect::<String>().trim().to_string()
}

fn numeric_aliases(surface: &str) -> Vec<String> {
    let pairs = [('一', '1'), ('二', '2'), ('三', '3'), ('四', '4'), ('五', '5'), ('六', '6'), ('七', '7'), ('八', '8'), ('九', '9')];
    let mut arabic = surface.to_string();
    for (chinese, digit) in pairs {
        arabic = arabic.replace(chinese, &digit.to_string());
    }
    let mut aliases = Vec::new();
    if arabic != surface {
        aliases.push(arabic);
    }
    aliases
}

fn explicit_aliases(meaning: &str, canonical: &str) -> Vec<String> {
    let Some((_, listed)) = meaning.rsplit_once("常说的") else {
        return Vec::new();
    };
    let listed = listed
        .split(['，', ',', '；', ';', '。'])
        .next()
        .unwrap_or(listed)
        .trim_end_matches("操作");
    listed
        .split(['/', '、', '和'])
        .map(trim_term)
        .filter(|alias| valid_surface(alias) && *alias != canonical)
        .map(ToString::to_string)
        .collect()
}

fn normalize(value: &str) -> String {
    value
        .to_lowercase()
        .chars()
        .filter(|character| !character.is_whitespace() && !matches!(character, '_' | '-' | '·'))
        .collect()
}

fn sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::knowledge::{KnowledgeClientScope, KnowledgeMountScope};

    fn chunk<'a>(text: &'a str, document_id: &'a str) -> DomainTermChunkContext<'a> {
        DomainTermChunkContext {
            document_id,
            title: "测试攻略",
            season: "当前赛季",
            category: "白皮书",
            heading: "循环",
            text,
            source_url: "https://example.com/source",
            yuque_url: "https://example.com/yuque",
            source_updated_at: "2026-09-05",
            document_hash: document_id,
            chunk_hash: document_id,
            audience: KnowledgeAudience {
                client: KnowledgeClientScope::Flagship,
                mount: None,
            },
            fact_eligible: true,
        }
    }

    #[test]
    fn extracts_definition_sequence_numeric_and_acronym_without_a_seed_list() {
        let index = DomainTerminologyIndexV1::build([
            chunk("白刀指未触发联动的绝刀。常用循环写作「斩绝绝」，加速采用14156，CW另行处理。", "a"),
            chunk("「斩绝绝」可拆开理解，14156档位下会变化，CW需要留窗口。", "b"),
        ]);

        let white = index.resolve("白刀是什么意思", "当前赛季");
        let rotation = index.resolve("斩绝绝怎么处理", "当前赛季");
        let haste = index.resolve("14156有什么区别", "当前赛季");
        let acronym = index.resolve("CW什么时候开", "当前赛季");

        assert!(white.iter().any(|item| item.matched_surface == "白刀"));
        assert!(rotation.iter().any(|item| item.matched_surface == "斩绝绝"));
        assert!(haste.iter().any(|item| item.matched_surface == "14156"));
        assert!(acronym.iter().any(|item| item.matched_surface == "CW"));
    }

    #[test]
    fn preserves_version_ambiguity_instead_of_globally_merging_meanings() {
        let old = DomainTermChunkContext { season: "旧赛季", ..chunk("白刀指旧机制下的一类绝刀。", "old") };
        let current = DomainTermChunkContext { season: "当前赛季", ..chunk("白刀指当前机制下未触发联动的绝刀。", "current") };
        let index = DomainTerminologyIndexV1::build([old, current]);

        let resolved = index.resolve_filtered("白刀", "当前赛季", |_| true);

        assert_eq!(resolved[0].cards[0].season, "当前赛季");
        assert_eq!(resolved[0].resolution, "current_scope_with_historical_variants");
    }

    #[test]
    fn scoped_term_definitions_keep_client_and_mount_separate() {
        let fenshan = KnowledgeAudience {
            client: KnowledgeClientScope::Flagship,
            mount: Some(KnowledgeMountScope::Fenshanjin),
        };
        let wujie = KnowledgeAudience { client: KnowledgeClientScope::Wujie, ..fenshan };
        let tiegu = KnowledgeAudience { mount: Some(KnowledgeMountScope::Tieguyi), ..fenshan };
        let index = DomainTerminologyIndexV1::build([
            DomainTermChunkContext { audience: fenshan, ..chunk("回风指旗舰分山的资源窗口。", "a") },
            DomainTermChunkContext { audience: wujie, ..chunk("回风指无界的技能组合。", "b") },
            DomainTermChunkContext { audience: tiegu, ..chunk("回风指铁骨的防御窗口。", "c") },
        ]);
        for (audience, expected) in [(fenshan, "旗舰分山"), (wujie, "无界"), (tiegu, "铁骨")] {
            let resolved = index.resolve_filtered("回风", "当前赛季", |card| audience.accepts(card.audience));
            assert_eq!(resolved.len(), 1);
            assert_eq!(resolved[0].cards.len(), 1);
            let card = &resolved[0].cards[0];
            assert!(card.meaning.contains(expected));
            assert_eq!(card.document_count, 1);
            assert_eq!(card.sources[0].audience, audience);
        }
        let all = index.resolve_filtered("回风", "当前赛季", |_| true);
        assert_eq!(all[0].resolution, "current_scope_ambiguous");
        assert_eq!(all[0].cards.len(), 3);
        assert!(all[0].cards.iter().all(|card| card.document_count == 1));
    }

    #[test]
    fn reference_only_and_unqualified_chunks_cannot_define_terms() {
        let index = DomainTerminologyIndexV1::build([
            DomainTermChunkContext { fact_eligible: false, ..chunk("伪术指仅用于来源身份的描述。", "a") },
            DomainTermChunkContext { fact_eligible: false, ..chunk("伪术指过期机制的描述。", "b") },
            DomainTermChunkContext { fact_eligible: false, ..chunk("伪术指错误内容。", "c") },
        ]);
        assert_eq!(index.card_count(), 0);
        assert!(index.resolve("伪术", "当前赛季").is_empty());
    }

    #[test]
    fn repeated_technical_tokens_need_no_cross_scope_document_to_be_recognized() {
        let index = DomainTerminologyIndexV1::build([
            chunk("12345是一个加速档位。再次讨论12345时需要检查循环。", "one"),
        ]);
        let resolved = index.resolve("12345怎么用", "当前赛季");
        let card = &resolved[0].cards[0];
        assert_eq!(card.surface, "12345");
        assert_eq!(card.document_count, 1);
        assert_eq!(card.occurrence_count, 2);
        assert_eq!(card.meaning_basis, "representative_usage");
        assert_eq!(card.confidence, "medium");
    }
}
