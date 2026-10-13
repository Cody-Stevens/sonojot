//! Experimental B4 spoken email/URL normalization. This module is opt-in; the
//! caller must exclude Format only and As heard and retain the raw transcript.
//!
//! Eligibility requires an adjacent email/navigation cue, horizontal whitespace
//! between spoken components, and a complete parse. Bare addresses, adjacent
//! unseparated name words, non-ASCII identifiers, mixed spoken/literal syntax,
//! arbitrary schemes, ports, percent escapes, and unlisted TLDs are unsupported.
//! Quotes/code are protected. Literal markers guard a candidate's
//! comma/semicolon segment or an adjacent literal-instruction segment;
//! uncertainty guards its clause. Negation never guards. Explicit
//! following/previous-text literal references cover a neighboring sentence.
//! No marker vetoes the whole source.
//!
//! Grammar: ASCII alphanumeric atoms; `digit zero` through `digit nine` (or one
//! numeric digit); `dot`, `hyphen`/`dash`, `underscore`, email `at`/`plus`; URL
//! `http[s] colon slash slash`, `slash`, `question mark`, `equals`, `ampersand`,
//! and `hash`/`hashtag`. Each ordinary atom is one source word. Explicit digit
//! expressions may follow an atom; ordinary adjacent words are never joined.
//! Query pairs require nonempty keys/values; fragments require a nonempty atom.
//! Domain labels are lowercased; all other identifier casing is retained.
//! Candidates are limited to 256 source words and 128 bytes per atom. Boundaries
//! must be external punctuation, a line/end boundary, or a listed prose connector
//! or continuation word.
//!
//! Edits use UTF-8 byte offsets into the complete, SHA-256-bound source. The
//! validator reconstructs eligibility and the complete grammar result from that
//! source; supplied replacements and accounting are never trusted. Cancellation
//! rolls back the entire transaction to the complete source with `cancelled=true`.
//! The caller must still check job/cancellation state at its publication boundary.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};

const EMAIL_ORIGIN: &str = "host.spoken_email.v1";
const URL_ORIGIN: &str = "host.spoken_url.v1";
const MAX_COMPONENT_WORDS: usize = 256;
const MAX_ATOM_BYTES: usize = 128;
const STRUCTURAL_WORDS: &[&str] = &[
    "at",
    "dot",
    "plus",
    "hyphen",
    "dash",
    "underscore",
    "digit",
    "slash",
    "colon",
    "question",
    "mark",
    "equals",
    "ampersand",
    "hash",
    "hashtag",
    "percent",
    "port",
    "http",
    "https",
    "space",
    "double",
    "number",
    "symbol",
    "period",
    "forward",
    "backslash",
];
/// A finite eligibility list, not a suffix-inference or public-suffix service.
const TLDS: &[&str] = &[
    "com", "org", "net", "edu", "gov", "mil", "int", "io", "dev", "app", "ai", "co", "uk", "us",
    "ca", "de", "fr", "au", "nz", "jp", "info", "biz", "me", "tv", "tech", "studio", "online",
    "site", "cloud", "xyz",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Component {
    pub start_byte: usize,
    pub end_byte: usize,
    pub source: String,
    pub replacement: String,
    /// Grammar role, including whitespace and explicit digit markers that emit
    /// no characters. Components partition the entire edit source span.
    pub role: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Edit {
    pub source_revision: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub source: String,
    pub replacement: String,
    pub origin: String,
    pub components: Vec<Component>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Normalization {
    pub text: String,
    pub edits: Vec<Edit>,
    pub source_revision: String,
    /// A cancelled result contains the complete original text and no edits.
    /// It is a fallback snapshot, not authorization to publish a cancelled job.
    pub cancelled: bool,
}

/// Lowercase hexadecimal SHA-256 of the exact source UTF-8 bytes.
pub fn source_revision(source: &str) -> String {
    format!("{:x}", Sha256::digest(source.as_bytes()))
}

/// Construct and independently validate deterministic eligible replacements.
pub fn normalize(source: &str, cancel: &AtomicBool) -> Result<Normalization, String> {
    normalize_checked(source, &|| cancel.load(Ordering::Acquire))
}

/// Recheck source identity, byte boundaries, order, nonoverlap, eligibility,
/// complete grammar, and component accounting before applying any edits.
/// A sorted subset of canonical edits is allowed; malformed edits fail atomically.
pub fn apply_edits(
    source: &str,
    edits: &[Edit],
    cancel: &AtomicBool,
) -> Result<Normalization, String> {
    let revision = source_revision(source);
    finish(
        source,
        &revision,
        apply_checked(source, &revision, edits, &|| cancel.load(Ordering::Acquire)),
    )
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VocabularyStatus {
    Confirmed,
    Rejected,
    #[default]
    #[serde(other)]
    Pending,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VocabularyProvenance {
    ExplicitUserConfirmation,
    Observed,
    ModelSuggested,
    #[default]
    #[serde(other)]
    Unverified,
}

/// An evaluation-only mapping. Both confirmed status and explicit user
/// confirmation provenance are required; neither is inferred from an observation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VocabularyEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub canonical: String,
    pub aliases: Vec<String>,
    pub scope: String,
    #[serde(default)]
    pub status: VocabularyStatus,
    #[serde(default)]
    pub provenance: VocabularyProvenance,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VocabularyScopeEvidence {
    /// Legacy shape: an active scope by itself is retrieval provenance and is
    /// never emitted as sufficient evidence by the current policy.
    ActiveScope { scope: String },
    /// Occurrence-attached descriptor evidence. `active_scope` records a
    /// matching active scope as provenance; it is never required or
    /// sufficient on its own.
    LocalContext {
        start_byte: usize,
        end_byte: usize,
        source: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        active_scope: Option<String>,
    },
}

/// A complete alias replacement, with offsets into the original raw source.
/// This is deliberately a separate type from a finite-grammar address edit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VocabularyEdit {
    pub source_revision: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub source: String,
    pub replacement: String,
    pub entry: VocabularyEntry,
    pub scope_evidence: VocabularyScopeEvidence,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VocabularyNormalization {
    pub text: String,
    /// Address edits only, accepted by the unchanged `apply_edits` validator.
    pub edits: Vec<Edit>,
    pub vocabulary_edits: Vec<VocabularyEdit>,
    pub source_revision: String,
    /// Cancellation rolls both stages back to the complete raw source.
    pub cancelled: bool,
}

/// Normalize addresses and confirmed scoped names as one atomic transaction.
/// Both edit lists use original-source coordinates, never intermediate text.
/// Scope inference requires evidence attached to each alias occurrence:
/// adjacent AI-model descriptors for `AI model discussion`. Other scopes
/// have no descriptor grammar yet and abstain. An active scope is recorded
/// as provenance alongside attached evidence and never authorizes an edit by
/// itself. Mappings accept complete prose names, including multiple words,
/// hyphens and underscores; addresses, dotted names and paths are unsupported.
/// The caller must exclude Format only/As heard and recheck cancellation before
/// publication. This API does not persist, confirm, or promote any vocabulary.
pub fn normalize_with_vocabulary(
    source: &str,
    entries: &[VocabularyEntry],
    active_scope: Option<&str>,
    cancel: &AtomicBool,
) -> Result<VocabularyNormalization, String> {
    normalize_with_vocabulary_checked(source, entries, active_scope, &|| {
        cancel.load(Ordering::Acquire)
    })
}

/// Revalidate both trace stages against raw source and the supplied confirmed
/// entries/scope. Apply a sorted independent subset from either list, merged by
/// original byte offset. Do not apply one list to the other stage's output.
/// Vocabulary edits cannot be passed to `apply_edits` as address grammar edits.
pub fn apply_vocabulary_edits(
    source: &str,
    edits: &[Edit],
    vocabulary_edits: &[VocabularyEdit],
    entries: &[VocabularyEntry],
    active_scope: Option<&str>,
    cancel: &AtomicBool,
) -> Result<VocabularyNormalization, String> {
    let revision = source_revision(source);
    finish_vocabulary(
        source,
        &revision,
        apply_vocabulary_checked(
            source,
            &revision,
            edits,
            vocabulary_edits,
            entries,
            active_scope,
            &|| cancel.load(Ordering::Acquire),
        ),
    )
}

#[derive(Debug)]
enum Failure {
    Cancelled,
    Invalid(String),
}

type Checked<T> = Result<T, Failure>;

fn check(cancel: &impl Fn() -> bool) -> Checked<()> {
    if cancel() {
        Err(Failure::Cancelled)
    } else {
        Ok(())
    }
}

fn finish(
    source: &str,
    revision: &str,
    result: Checked<Normalization>,
) -> Result<Normalization, String> {
    match result {
        Ok(result) => Ok(result),
        Err(Failure::Invalid(message)) => Err(message),
        Err(Failure::Cancelled) => Ok(Normalization {
            text: source.into(),
            edits: Vec::new(),
            source_revision: revision.into(),
            cancelled: true,
        }),
    }
}

fn normalize_checked(source: &str, cancel: &impl Fn() -> bool) -> Result<Normalization, String> {
    let revision = source_revision(source);
    let result = discover(source, &revision, cancel)
        .and_then(|edits| apply_checked(source, &revision, &edits, cancel));
    finish(source, &revision, result)
}

fn apply_checked(
    source: &str,
    revision: &str,
    edits: &[Edit],
    cancel: &impl Fn() -> bool,
) -> Checked<Normalization> {
    check(cancel)?;
    // Reparse the full immutable source, including cues and tails, rather than
    // validating only the supplied span or the syntax of its replacement.
    let canonical = discover(source, revision, cancel)?;
    let mut previous_end = 0;
    let mut canonical_index = 0;
    for edit in edits {
        check(cancel)?;
        if edit.source_revision != revision
            || edit.start_byte < previous_end
            || edit.start_byte >= edit.end_byte
            || source.get(edit.start_byte..edit.end_byte) != Some(edit.source.as_str())
        {
            return Err(Failure::Invalid(
                "Stale, overlapping, unsorted, or mismatched source edit.".into(),
            ));
        }
        while canonical_index < canonical.len()
            && canonical[canonical_index].start_byte < edit.start_byte
        {
            canonical_index += 1;
        }
        if canonical.get(canonical_index) != Some(edit) {
            return Err(Failure::Invalid(
                "Edit differs from the complete eligible source grammar/accounting.".into(),
            ));
        }
        validate_accounting(source, edit)?;
        previous_end = edit.end_byte;
        canonical_index += 1;
    }
    check(cancel)?;
    let mut text = String::with_capacity(source.len());
    let mut retained_start = 0;
    for edit in edits {
        check(cancel)?;
        text.push_str(&source[retained_start..edit.start_byte]);
        text.push_str(&edit.replacement);
        retained_start = edit.end_byte;
    }
    // This tail is always appended, including when there are no eligible edits.
    text.push_str(&source[retained_start..]);
    check(cancel)?;
    Ok(Normalization {
        text,
        edits: edits.to_vec(),
        source_revision: revision.into(),
        cancelled: false,
    })
}

fn validate_accounting(source: &str, edit: &Edit) -> Checked<()> {
    let mut end = edit.start_byte;
    let mut rendered = String::new();
    for component in &edit.components {
        if component.start_byte != end
            || component.end_byte <= end
            || component.end_byte > edit.end_byte
            || source.get(component.start_byte..component.end_byte)
                != Some(component.source.as_str())
        {
            return Err(Failure::Invalid(
                "Component accounting does not partition the exact source span.".into(),
            ));
        }
        end = component.end_byte;
        rendered.push_str(&component.replacement);
    }
    if end != edit.end_byte || rendered != edit.replacement {
        return Err(Failure::Invalid(
            "Component accounting does not reproduce the entire edit.".into(),
        ));
    }
    Ok(())
}

#[derive(Debug)]
struct Token<'a> {
    start: usize,
    end: usize,
    raw_start: usize,
    raw_end: usize,
    word: &'a str,
    protected: bool,
}

fn external_punctuation(c: char) -> bool {
    matches!(
        c,
        '.' | ',' | ';' | ':' | '!' | '?' | '(' | ')' | '[' | ']' | '{' | '}'
    )
}

fn horizontal(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b == b' ' || b == b'\t')
}

fn adjacent(source: &str, tokens: &[Token<'_>], left: usize, right: usize) -> bool {
    right == left + 1 && horizontal(&source[tokens[left].end..tokens[right].start])
}

/// Delimited quotes and code are opaque, including unmatched opening delimiters.
/// Apostrophes inside words are not treated as quote delimiters.
fn protected_ranges(source: &str, cancel: &impl Fn() -> bool) -> Checked<Vec<(usize, usize)>> {
    let mut ranges = Vec::new();
    let mut chars = source.char_indices().peekable();
    let mut previous = None;
    while let Some((start, c)) = chars.next() {
        check(cancel)?;
        let closer = match c {
            '"' => Some('"'),
            '“' => Some('”'),
            '‘' => Some('’'),
            '`' => Some('`'),
            '\'' if !previous.is_some_and(char::is_alphanumeric) => Some('\''),
            _ => None,
        };
        if let Some(closer) = closer {
            let mut count = 1;
            if c == '`' {
                while chars.peek().is_some_and(|(_, next)| *next == '`') {
                    chars.next();
                    count += 1;
                }
            }
            let mut end = source.len();
            let mut escaped = false;
            while let Some((index, next)) = chars.next() {
                check(cancel)?;
                if c != '`' && !escaped && next == '\\' {
                    escaped = true;
                    continue;
                }
                if next == closer && !escaped {
                    let mut closing_count = 1;
                    if c == '`' {
                        while chars.peek().is_some_and(|(_, next)| *next == '`') {
                            chars.next();
                            closing_count += 1;
                        }
                    }
                    if closing_count == count {
                        end = index + closer.len_utf8() * closing_count;
                        break;
                    }
                }
                escaped = false;
            }
            ranges.push((start, end));
            previous = Some(closer);
        } else {
            previous = Some(c);
        }
    }
    Ok(ranges)
}

fn literal_word(word: &str) -> bool {
    [
        "literal",
        "literally",
        "verbatim",
        "word",
        "words",
        "quote",
        "quoted",
        "code",
        "string",
        "snippet",
        "unchanged",
        "spell",
        "spelled",
        "spelling",
        "phrase",
        "exact",
        "exactly",
    ]
    .iter()
    .any(|guard| word.eq_ignore_ascii_case(guard))
}

/// Uncertainty about the candidate's structure. These still guard an address
/// candidate's clause; for vocabulary aliases only the directly attached forms
/// count (see `vocabulary_candidate_guarded`).
fn uncertainty_word(word: &str) -> bool {
    [
        "uncertain",
        "unsure",
        "maybe",
        "perhaps",
        "probably",
        "might",
        "guess",
        "guessing",
        "think",
        "or",
        "whether",
    ]
    .iter()
    .any(|guard| word.eq_ignore_ascii_case(guard))
}

/// Negation of the surrounding action is not uncertainty about the spelling
/// of a candidate, so these words never guard. They remain reserved atoms.
fn negation_word(word: &str) -> bool {
    ["not", "never", "don't", "don’t", "cannot"]
        .iter()
        .any(|guard| word.eq_ignore_ascii_case(guard))
}

fn guard_word(word: &str) -> bool {
    literal_word(word) || uncertainty_word(word) || negation_word(word)
}

fn tokenize<'a>(source: &'a str, cancel: &impl Fn() -> bool) -> Checked<Vec<Token<'a>>> {
    let ranges = protected_ranges(source, cancel)?;
    let mut range_index = 0;
    let mut tokens = Vec::new();
    let mut offset = 0;
    let mut line_start = 0;
    for raw in source.split_whitespace() {
        check(cancel)?;
        let raw_start = offset + source[offset..].find(raw).expect("split source token");
        let raw_end = raw_start + raw.len();
        if let Some(newline) = source[offset..raw_start].rfind('\n') {
            line_start = offset + newline + 1;
        }
        offset = raw_end;
        let trimmed_start = raw.trim_start_matches(external_punctuation);
        let word = trimmed_start.trim_end_matches(external_punctuation);
        let start = raw_start + raw.len() - trimmed_start.len();
        let end = start + word.len();
        while range_index < ranges.len() && ranges[range_index].1 <= raw_start {
            range_index += 1;
        }
        let quoted = ranges
            .get(range_index)
            .is_some_and(|(a, b)| *a < raw_end && *b > raw_start);
        let line_prefix = &source[line_start..raw_start];
        let code_line = line_prefix.starts_with("    ")
            || line_prefix.starts_with('\t')
            || line_prefix.trim_start().starts_with('>')
            || raw.starts_with('>');
        tokens.push(Token {
            start,
            end,
            raw_start,
            raw_end,
            word,
            protected: quoted || code_line,
        });
    }
    Ok(tokens)
}

fn sentence_boundary(source: &str, tokens: &[Token<'_>], after: usize) -> bool {
    let token = &tokens[after];
    source[token.end..token.raw_end].contains(['.', '!', '?', ';'])
        || after + 1 == tokens.len()
        || source[token.raw_end..tokens[after + 1].raw_start].contains(['\n', '\r'])
}

fn sentence_span(source: &str, tokens: &[Token<'_>], at: usize) -> (usize, usize) {
    let mut start = at;
    while start > 0 && !sentence_boundary(source, tokens, start - 1) {
        start -= 1;
    }
    let mut end = at + 1;
    while end < tokens.len() && !sentence_boundary(source, tokens, end - 1) {
        end += 1;
    }
    (start, end)
}

fn words_match(source: &str, tokens: &[Token<'_>], start: usize, words: &[&str]) -> bool {
    start + words.len() <= tokens.len()
        && words.iter().enumerate().all(|(offset, word)| {
            let index = start + offset;
            !tokens[index].protected
                && tokens[index].word.eq_ignore_ascii_case(word)
                && (offset == 0 || adjacent(source, tokens, index - 1, index))
        })
}

/// Only concrete independent action clauses end an uncertainty scope. In
/// particular, `or`, `but maybe ...`, and address continuations never do.
fn independent_clause(source: &str, tokens: &[Token<'_>], start: usize) -> bool {
    if !["but", "and", "then"]
        .iter()
        .any(|word| !tokens[start].protected && tokens[start].word.eq_ignore_ascii_case(word))
    {
        return false;
    }
    let mut next = start + 1;
    if words_match(source, tokens, next, &["then"]) {
        next += 1;
    }
    if words_match(source, tokens, next, &["please"]) {
        next += 1;
    }
    for prefix in [&["do", "not"][..], &["don't"], &["don’t"], &["never"]] {
        if words_match(source, tokens, next, prefix) {
            next += prefix.len();
            break;
        }
    }
    ["deploy", "compile", "build", "test", "call"]
        .iter()
        .any(|word| words_match(source, tokens, next, &[word]))
}

fn clause_span(source: &str, tokens: &[Token<'_>], first: usize, end: usize) -> (usize, usize) {
    let (mut left, mut right) = sentence_span(source, tokens, first);
    // A candidate cannot span an independent sentence or line.
    if end > right {
        return (left, end);
    }
    for index in left..first {
        if independent_clause(source, tokens, index) {
            left = index;
        }
    }
    for index in end..right {
        if independent_clause(source, tokens, index) {
            right = index;
            break;
        }
    }
    (left, right)
}

/// Verbs that make a literal word an instruction about text rather than an
/// ordinary noun ("update the code" is not a request to preserve spelling).
fn instruction_word(word: &str) -> bool {
    [
        "say", "write", "keep", "preserve", "repeat", "copy", "leave",
    ]
    .iter()
    .any(|verb| word.eq_ignore_ascii_case(verb))
}

fn literal_reference(tokens: &[Token<'_>], forward: bool) -> bool {
    let has = |words: &[&str]| {
        tokens.iter().any(|token| {
            !token.protected
                && words
                    .iter()
                    .any(|word| token.word.eq_ignore_ascii_case(word))
        })
    };
    has(&[
        "say", "write", "keep", "preserve", "repeat", "copy", "leave",
    ]) && tokens
        .iter()
        .any(|token| !token.protected && literal_word(token.word))
        && if forward {
            has(&["following", "next", "below"])
        } else {
            has(&[
                "those",
                "these",
                "that",
                "previous",
                "above",
                "it",
                "its",
                "them",
                "address",
                "addresses",
                "text",
            ])
        }
}

/// Segments split a sentence at commas and semicolons. A literal word in a
/// different segment refers to that segment's own text, not the candidate,
/// unless its segment is itself a literal instruction.
fn segment_boundary(source: &str, tokens: &[Token<'_>], after: usize) -> bool {
    sentence_boundary(source, tokens, after)
        || source[tokens[after].end..tokens[after + 1].start].contains([',', ';'])
}

fn segment_span(source: &str, tokens: &[Token<'_>], at: usize) -> (usize, usize) {
    let mut start = at;
    while start > 0 && !segment_boundary(source, tokens, start - 1) {
        start -= 1;
    }
    let mut end = at + 1;
    while end < tokens.len() && !segment_boundary(source, tokens, end - 1) {
        end += 1;
    }
    (start, end)
}

/// A protected range or a code-looking token (anything not a plain ASCII
/// alphabetic word, such as `user_id` or `status_code`).
fn literal_context_token(token: &Token<'_>) -> bool {
    token.protected || token.word.bytes().any(|b| !b.is_ascii_alphabetic())
}

/// A literal word adjacent to a protected or code-looking token belongs to
/// that literal; it is not an instruction about a neighboring candidate.
fn literal_consumed(source: &str, tokens: &[Token<'_>], at: usize) -> bool {
    (at > 0 && adjacent(source, tokens, at - 1, at) && literal_context_token(&tokens[at - 1]))
        || (at + 1 < tokens.len()
            && adjacent(source, tokens, at, at + 1)
            && literal_context_token(&tokens[at + 1]))
}

/// Whether the token range `start..end` names its own literal referent: an
/// unprotected content word directly attached to a spelling/naming word, as in
/// "keep naïve spelled that way". That instruction governs the named word, not
/// a candidate in a neighboring sentence or segment. Deictic and generic
/// referents ("that address", "those words", "it") still point across the
/// boundary and keep their protection.
fn local_literal_referent(source: &str, tokens: &[Token<'_>], start: usize, end: usize) -> bool {
    const NAMING_WORDS: &[&str] = &[
        "spelled",
        "spelt",
        "spelling",
        "written",
        "pronounced",
        "named",
        "called",
        "capitalized",
        "capitalised",
    ];
    const GENERIC_REFERENTS: &[&str] = &[
        "it",
        "its",
        "that",
        "this",
        "those",
        "these",
        "them",
        "the",
        "a",
        "an",
        "address",
        "addresses",
        "email",
        "emails",
        "word",
        "words",
        "phrase",
        "phrases",
        "string",
        "strings",
        "text",
        "name",
        "names",
        "spelling",
        "pronunciation",
        "sentence",
        "sentences",
        "line",
        "lines",
        "part",
        "parts",
        "one",
        "version",
        "versions",
        "everything",
        "all",
    ];
    // A named spelling request does not cancel a second, anaphoric request
    // in the same segment ("keep naive spelled that way and that address literal").
    if tokens[start..end].windows(2).any(|pair| {
        ["that", "this", "those", "these", "the", "previous"]
            .iter()
            .any(|word| pair[0].word.eq_ignore_ascii_case(word))
            && GENERIC_REFERENTS
                .iter()
                .any(|word| pair[1].word.eq_ignore_ascii_case(word))
    }) {
        return false;
    }
    (start..end.saturating_sub(1)).any(|index| {
        let referent = &tokens[index];
        let naming = &tokens[index + 1];
        !referent.protected
            && !naming.protected
            && adjacent(source, tokens, index, index + 1)
            && !referent.word.is_empty()
            && NAMING_WORDS
                .iter()
                .any(|word| naming.word.eq_ignore_ascii_case(word))
            && !GENERIC_REFERENTS
                .iter()
                .any(|word| referent.word.eq_ignore_ascii_case(word))
            && !instruction_word(referent.word)
            && !literal_word(referent.word)
    })
}

fn cross_sentence_literal_reference(source: &str, tokens: &[Token<'_>], at: usize) -> bool {
    let (sentence_start, sentence_end) = sentence_span(source, tokens, at);
    if sentence_start > 0 {
        let (previous, _) = sentence_span(source, tokens, sentence_start - 1);
        if literal_reference(&tokens[previous..sentence_start], true)
            && !local_literal_referent(source, tokens, previous, sentence_start)
        {
            return true;
        }
    }
    if sentence_end < tokens.len() {
        let (_, next) = sentence_span(source, tokens, sentence_end);
        if literal_reference(&tokens[sentence_end..next], false)
            && !local_literal_referent(source, tokens, sentence_end, next)
        {
            return true;
        }
    }
    false
}

fn candidate_guarded(source: &str, tokens: &[Token<'_>], first: usize, end: usize) -> bool {
    // Uncertainty about the address structure still guards the candidate's
    // clause; negating the surrounding action does not.
    let (left, right) = clause_span(source, tokens, first, end);
    if tokens[left..right]
        .iter()
        .any(|token| !token.protected && uncertainty_word(token.word))
    {
        return true;
    }
    // A literal word guards a candidate in its own segment, or in another
    // segment of the same sentence that is a literal instruction about text.
    // A literal word consumed by an adjacent protected/code literal is never
    // an instruction about this candidate.
    let (sentence_start, sentence_end) = sentence_span(source, tokens, first);
    let (segment_start, segment_end) = segment_span(source, tokens, first);
    for index in sentence_start..sentence_end {
        if tokens[index].protected
            || !literal_word(tokens[index].word)
            || literal_consumed(source, tokens, index)
        {
            continue;
        }
        if (segment_start..segment_end).contains(&index) {
            return true;
        }
        let (other_start, other_end) = segment_span(source, tokens, index);
        if tokens[other_start..other_end]
            .iter()
            .any(|token| !token.protected && instruction_word(token.word))
            && !local_literal_referent(source, tokens, other_start, other_end)
        {
            return true;
        }
    }
    // Preserve explicit cross-sentence literal instructions without allowing an
    // unrelated mention of code/string/exact elsewhere to veto every candidate.
    cross_sentence_literal_reference(source, tokens, first)
}

#[derive(Clone, Copy)]
enum Kind {
    Email,
    Url,
}

/// Cues establish local scope. They are retained outside the replacement.
fn cue(source: &str, tokens: &[Token<'_>], start: usize) -> Option<(Kind, usize)> {
    const EMAIL_CUES: &[&[&str]] = &[
        &["email", "address", "is"],
        &["email", "address"],
        &["email", "me", "at"],
        &["email", "us", "at"],
        &["email", "him", "at"],
        &["email", "her", "at"],
        &["email", "them", "at"],
        &["email", "is"],
        &["email", "to"],
        &["email"],
        &["contact"],
        &["send", "it", "to"],
    ];
    const URL_CUES: &[&[&str]] = &[
        &["go", "to"],
        &["visit"],
        &["open"],
        &["url", "is"],
        &["url"],
        &["website", "is"],
        &["website"],
        &["web", "address", "is"],
    ];
    for (kind, patterns) in [(Kind::Email, EMAIL_CUES), (Kind::Url, URL_CUES)] {
        for pattern in patterns {
            let next = start + pattern.len();
            if next >= tokens.len() {
                continue;
            }
            let matches = pattern.iter().enumerate().all(|(j, word)| {
                let index = start + j;
                !tokens[index].protected
                    && tokens[index].word.eq_ignore_ascii_case(word)
                    && (j == 0 || adjacent(source, tokens, index - 1, index))
            });
            if !matches {
                continue;
            }
            // `Contact` can also begin a noun phrase. These common compounds
            // and determiner-led uses provide no reliable addressing intent.
            if pattern == &["contact"]
                && ([
                    "lens",
                    "lenses",
                    "details",
                    "information",
                    "form",
                    "list",
                    "page",
                    "number",
                    "tracing",
                ]
                .iter()
                .any(|word| tokens[next].word.eq_ignore_ascii_case(word))
                    || (start > 0
                        && adjacent(source, tokens, start - 1, start)
                        && ["a", "the", "my", "our", "your", "his", "her", "their"]
                            .iter()
                            .any(|word| tokens[start - 1].word.eq_ignore_ascii_case(word))))
            {
                continue;
            }
            let gap = &source[tokens[next - 1].end..tokens[next].start];
            if horizontal(gap) || gap.strip_prefix(':').is_some_and(horizontal) {
                return Some((kind, next));
            }
        }
    }
    None
}

fn prose_boundary(word: &str) -> bool {
    [
        "and", "then", "please", "for", "so", "to", "when", "with", "before", "after", "because",
        "but",
    ]
    .iter()
    .any(|boundary| word.eq_ignore_ascii_case(boundary))
}

/// Words that close a finished candidate without becoming reserved atoms:
/// common dictation continuations after a complete address or URL ("... dot io
/// yet"). They are not added to `prose_boundary`, so they remain usable
/// components earlier in the grammar; only an already-complete candidate may
/// end before them. Unsupported words still reject the whole candidate rather
/// than leaving a dangling component.
const CONTINUATION_WORDS: &[&str] = &[
    "yet",
    "now",
    "again",
    "instead",
    "still",
    "first",
    "later",
    "today",
    "tomorrow",
    "tonight",
    "yesterday",
    "though",
    "anyway",
];

fn reserved(word: &str) -> bool {
    prose_boundary(word)
        || guard_word(word)
        || STRUCTURAL_WORDS
            .iter()
            .any(|keyword| word.eq_ignore_ascii_case(keyword))
}

struct Parser<'a, 't> {
    source: &'a str,
    tokens: &'t [Token<'a>],
    first: usize,
    next: usize,
    components: Vec<Component>,
    rendered: String,
}

impl<'a, 't> Parser<'a, 't> {
    fn peek(&self) -> Option<&'a str> {
        let token = self.tokens.get(self.next)?;
        if token.protected
            || token.word.is_empty()
            || self.next - self.first >= MAX_COMPONENT_WORDS
            || token.word.len() > MAX_ATOM_BYTES
            || !token.word.bytes().all(|b| b.is_ascii_alphanumeric())
            || (self.next > self.first
                && !adjacent(self.source, self.tokens, self.next - 1, self.next))
        {
            None
        } else {
            Some(token.word)
        }
    }

    fn is(&self, word: &str) -> bool {
        self.peek()
            .is_some_and(|next| next.eq_ignore_ascii_case(word))
    }

    fn take(&mut self, role: &str, replacement: &str) -> Option<()> {
        self.peek()?;
        let token = &self.tokens[self.next];
        if self.next > self.first {
            let start = self.tokens[self.next - 1].end;
            self.components.push(Component {
                start_byte: start,
                end_byte: token.start,
                source: self.source[start..token.start].into(),
                replacement: String::new(),
                role: "whitespace".into(),
            });
        }
        self.components.push(Component {
            start_byte: token.start,
            end_byte: token.end,
            source: token.word.into(),
            replacement: replacement.into(),
            role: role.into(),
        });
        self.rendered.push_str(replacement);
        self.next += 1;
        Some(())
    }

    fn symbol(&mut self, spoken: &str, replacement: &str, role: &str) -> Option<()> {
        if !self.is(spoken) {
            return None;
        }
        self.take(role, replacement)
    }

    fn digit(&mut self, role: &str) -> Option<()> {
        self.symbol("digit", "", "digit_marker")?;
        let word = self.peek()?;
        let digit = [
            "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
        ]
        .iter()
        .position(|name| word.eq_ignore_ascii_case(name))
        .map(|index| char::from(b'0' + index as u8))
        .or_else(|| {
            (word.len() == 1 && word.as_bytes()[0].is_ascii_digit())
                .then(|| word.as_bytes()[0] as char)
        })?;
        self.take(role, &digit.to_string())
    }

    fn atom(&mut self, role: &str, lowercase: bool) -> Option<()> {
        if self.is("digit") {
            self.digit(role)?;
        } else {
            let word = self.peek()?;
            if reserved(word) {
                return None;
            }
            let rendered = if lowercase {
                word.to_ascii_lowercase()
            } else {
                word.to_owned()
            };
            self.take(role, &rendered)?;
        }
        while self.is("digit") {
            self.digit(role)?;
        }
        Some(())
    }

    fn separated_atom(
        &mut self,
        role: &str,
        lowercase: bool,
        separators: &[(&str, &str)],
    ) -> Option<()> {
        self.atom(role, lowercase)?;
        while let Some((_, symbol)) = separators.iter().find(|(spoken, _)| self.is(spoken)) {
            self.take(role, symbol)?;
            self.atom(role, lowercase)?;
        }
        Some(())
    }

    fn domain(&mut self) -> Option<()> {
        let domain_start = self.rendered.len();
        let mut labels = 0;
        let last_label;
        loop {
            let label_start = self.rendered.len();
            self.separated_atom("domain_label", true, &[("hyphen", "-"), ("dash", "-")])?;
            if self.rendered.len() - label_start > 63 {
                return None;
            }
            labels += 1;
            if !self.is("dot") {
                last_label = label_start;
                break;
            }
            self.take("domain_separator", ".")?;
        }
        if labels < 2
            || self.rendered.len() - domain_start > 253
            || !TLDS.contains(&&self.rendered[last_label..])
        {
            return None;
        }
        Some(())
    }

    fn email(&mut self) -> Option<()> {
        let separators = &[
            ("dot", "."),
            ("hyphen", "-"),
            ("dash", "-"),
            ("underscore", "_"),
        ];
        self.separated_atom("local_part", false, separators)?;
        if self.is("plus") {
            self.take("plus_separator", "+")?;
            self.separated_atom("plus_tag", false, separators)?;
        }
        if self.rendered.len() > 64 {
            return None;
        }
        self.symbol("at", "@", "email_separator")?;
        self.domain()
    }

    fn url_atom(&mut self, role: &str) -> Option<()> {
        self.separated_atom(
            role,
            false,
            &[
                ("dot", "."),
                ("hyphen", "-"),
                ("dash", "-"),
                ("underscore", "_"),
                ("plus", "+"),
            ],
        )
    }

    fn url(&mut self) -> Option<()> {
        if self.is("http") || self.is("https") {
            let scheme = self.peek()?.to_ascii_lowercase();
            self.take("url_scheme", &scheme)?;
            self.symbol("colon", ":", "scheme_separator")?;
            self.symbol("slash", "/", "scheme_separator")?;
            self.symbol("slash", "/", "scheme_separator")?;
        }
        self.domain()?;
        while self.is("slash") {
            self.take("path_separator", "/")?;
            // Empty segments and an explicit trailing slash are valid and kept.
            if self
                .peek()
                .is_some_and(|word| !reserved(word) || word.eq_ignore_ascii_case("digit"))
            {
                self.url_atom("path")?;
            }
        }
        if self.is("question") {
            self.take("query_separator", "?")?;
            self.symbol("mark", "", "query_marker")?;
            loop {
                self.url_atom("query_key")?;
                self.symbol("equals", "=", "query_separator")?;
                self.url_atom("query_value")?;
                if !self.is("ampersand") {
                    break;
                }
                self.take("query_separator", "&")?;
            }
        }
        if self.is("hash") || self.is("hashtag") {
            self.take("fragment_separator", "#")?;
            self.url_atom("fragment")?;
        }
        Some(())
    }

    fn complete_boundary(&self) -> bool {
        let last = &self.tokens[self.next - 1];
        // A punctuated/line-separated continuation is unsupported, not permission
        // to normalize just its valid prefix and leave a dangling component.
        if self.tokens[self.next..]
            .iter()
            .find(|token| !token.word.is_empty())
            .is_some_and(|token| {
                STRUCTURAL_WORDS
                    .iter()
                    .any(|word| token.word.eq_ignore_ascii_case(word))
            })
        {
            return false;
        }
        // Only known external punctuation or a line boundary terminates a span.
        // An unknown adjacent word/literal/operator rejects the whole candidate.
        if last.end < last.raw_end {
            return true;
        }
        let Some(next) = self.tokens.get(self.next) else {
            return true;
        };
        if self.source[last.raw_end..next.raw_start].contains(['\n', '\r']) {
            return true;
        }
        if next.raw_start < next.start {
            return true;
        }
        prose_boundary(next.word)
            || CONTINUATION_WORDS
                .iter()
                .any(|word| next.word.eq_ignore_ascii_case(word))
    }
}

fn discover(source: &str, revision: &str, cancel: &impl Fn() -> bool) -> Checked<Vec<Edit>> {
    check(cancel)?;
    let tokens = tokenize(source, cancel)?;
    let mut edits = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        check(cancel)?;
        let Some((kind, first)) = cue(source, &tokens, index) else {
            index += 1;
            continue;
        };
        let mut parser = Parser {
            source,
            tokens: &tokens,
            first,
            next: first,
            components: Vec::new(),
            rendered: String::new(),
        };
        let parsed = match kind {
            Kind::Email => parser.email(),
            Kind::Url => parser.url(),
        };
        if parsed.is_none()
            || !parser.complete_boundary()
            || candidate_guarded(source, &tokens, index, parser.next)
        {
            index += 1;
            continue;
        }
        check(cancel)?;
        let start = tokens[first].start;
        let end = tokens[parser.next - 1].end;
        let edit = Edit {
            source_revision: revision.into(),
            start_byte: start,
            end_byte: end,
            source: source[start..end].into(),
            replacement: parser.rendered,
            origin: match kind {
                Kind::Email => EMAIL_ORIGIN,
                Kind::Url => URL_ORIGIN,
            }
            .into(),
            components: parser.components,
        };
        validate_accounting(source, &edit)?;
        index = parser.next;
        edits.push(edit);
    }
    check(cancel)?;
    Ok(edits)
}

/// Byte spans of cued email candidates the grammar started but could not
/// complete: a local part, `at`, and at least one domain-side atom consumed,
/// with no supported domain reached ("Email sam at comet.", "Email lee at
/// maple."). These spans are never normalized, and downstream formatters use
/// them to preserve the run byte-exact against speculative case or lexical
/// edits. Only email candidates carry the `at` separator that makes an
/// incomplete run address-like; an unparsed cue without `at` is ordinary prose.
pub fn incomplete_candidate_spans(
    source: &str,
    cancel: &impl Fn() -> bool,
) -> Result<Vec<(usize, usize)>, String> {
    match preserved_spans_checked(source, cancel, false) {
        Ok(spans) => Ok(spans),
        Err(Failure::Cancelled) => Err("Scan cancelled.".into()),
        Err(Failure::Invalid(message)) => Err(message),
    }
}

/// Spans downstream formatting must preserve: incomplete email attempts and
/// candidates whose literal/uncertainty guard prevented normalization. Guarded
/// spans include the cue so a request to keep the preceding text also preserves
/// its initial case. This API authorizes no new normalization.
pub fn preserved_candidate_spans(
    source: &str,
    cancel: &impl Fn() -> bool,
) -> Result<Vec<(usize, usize)>, String> {
    match preserved_spans_checked(source, cancel, true) {
        Ok(spans) => Ok(spans),
        Err(Failure::Cancelled) => Err("Scan cancelled.".into()),
        Err(Failure::Invalid(message)) => Err(message),
    }
}

fn preserved_spans_checked(
    source: &str,
    cancel: &impl Fn() -> bool,
    include_guarded: bool,
) -> Checked<Vec<(usize, usize)>> {
    let tokens = tokenize(source, cancel)?;
    let mut spans = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        check(cancel)?;
        let Some((kind, first)) = cue(source, &tokens, index) else {
            index += 1;
            continue;
        };
        let mut parser = Parser {
            source,
            tokens: &tokens,
            first,
            next: first,
            components: Vec::new(),
            rendered: String::new(),
        };
        // A failed parse that still consumed the `at` separator is an
        // incomplete address-like run; a parse that never reached `at` is not.
        let parsed = match kind {
            Kind::Email => parser.email(),
            Kind::Url => parser.url(),
        };
        if include_guarded
            && parsed.is_some()
            && (!parser.complete_boundary()
                || candidate_guarded(source, &tokens, index, parser.next))
        {
            spans.push((tokens[index].start, tokens[parser.next - 1].end));
            index = parser.next.max(index + 1);
            continue;
        }
        if matches!(kind, Kind::Email) && parsed.is_none() && parser.rendered.contains('@') {
            spans.push((tokens[first].start, tokens[parser.next - 1].end));
            index = parser.next.max(index + 1);
            continue;
        }
        index += 1;
    }
    check(cancel)?;
    Ok(spans)
}

fn finish_vocabulary(
    source: &str,
    revision: &str,
    result: Checked<VocabularyNormalization>,
) -> Result<VocabularyNormalization, String> {
    match result {
        Ok(result) => Ok(result),
        Err(Failure::Invalid(message)) => Err(message),
        Err(Failure::Cancelled) => Ok(VocabularyNormalization {
            text: source.into(),
            edits: Vec::new(),
            vocabulary_edits: Vec::new(),
            source_revision: revision.into(),
            cancelled: true,
        }),
    }
}

fn normalize_with_vocabulary_checked(
    source: &str,
    entries: &[VocabularyEntry],
    active_scope: Option<&str>,
    cancel: &impl Fn() -> bool,
) -> Result<VocabularyNormalization, String> {
    let revision = source_revision(source);
    let result = (|| {
        let edits = discover(source, &revision, cancel)?;
        let vocabulary_edits =
            discover_vocabulary(source, &revision, entries, active_scope, cancel)?;
        apply_vocabulary_checked(
            source,
            &revision,
            &edits,
            &vocabulary_edits,
            entries,
            active_scope,
            cancel,
        )
    })();
    finish_vocabulary(source, &revision, result)
}

/// This first typed path supports complete prose names (including multiword
/// names, hyphens and underscores). Dotted names, addresses, URLs and paths are
/// outside its grammar, so vocabulary cannot supply a missing email suffix.
fn vocabulary_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_ATOM_BYTES
        && name.split(' ').all(|word| {
            !word.is_empty()
                && word.chars().next().is_some_and(char::is_alphanumeric)
                && word.chars().last().is_some_and(char::is_alphanumeric)
                && word
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        })
}

fn same_scope(left: &str, right: &str) -> bool {
    !left.trim().is_empty() && left.trim().eq_ignore_ascii_case(right.trim())
}

/// Descriptor-before-alias evidence is usable only at the end of a sentence.
/// Even auxiliaries can introduce a person-relative clause ("the model Quinn
/// has trained is smaller"). Disable this optional continuation route rather
/// than infer the referent from a verb list. Descriptor-after-alias remains.
fn descriptor_prefix_closed(source: &str, tokens: &[Token<'_>], end: usize) -> bool {
    end >= tokens.len() || sentence_boundary(source, tokens, end - 1)
}

/// A plausible person predicate attached to the alias stays a negative signal
/// even when a descriptor is present on the other side.
fn person_predicate(source: &str, tokens: &[Token<'_>], end: usize) -> bool {
    [
        "emailed", "called", "said", "arrived", "asked", "replied", "joined",
    ]
    .iter()
    .any(|verb| {
        end < tokens.len()
            && adjacent(source, tokens, end - 1, end)
            && words_match(source, tokens, end, &[verb])
    })
}

fn vocabulary_scope(
    source: &str,
    tokens: &[Token<'_>],
    first: usize,
    end: usize,
    scope: &str,
    active_scope: Option<&str>,
) -> Option<VocabularyScopeEvidence> {
    // Only `AI model discussion` has a descriptor grammar. Other scopes have
    // no occurrence-attached evidence yet, so they always abstain. Add new
    // scope grammars explicitly.
    if !same_scope(scope, "AI model discussion") {
        return None;
    }
    if person_predicate(source, tokens, end) {
        return None;
    }
    // An active scope is retrieval provenance only: it is recorded alongside
    // attached evidence when it matches, and by itself it never authorizes an
    // edit. A topic does not settle the referent of an ambiguous name.
    let active = active_scope
        .filter(|active| same_scope(scope, active))
        .map(str::to_owned);
    // Evidence must be attached to this exact alias: a descriptor immediately
    // before or after it, with strict adjacency (no determiner between). A
    // descriptor before the alias also requires the sentence to close the noun
    // phrase (`descriptor_prefix_closed`); an ordinary following word leaves a
    // relative clause whose referent is a person or ambiguous.
    const AFTER: &[&[&str]] = &[
        &["model", "for", "automatic", "speech", "recognition"],
        &["model", "for", "natural", "language", "processing"],
        &["model", "for", "speech", "recognition"],
        &["model", "for", "speech", "synthesis"],
        &["model", "for", "text", "generation"],
        &["large", "language", "model"],
        &["language", "model"],
        &["AI", "model"],
        &["checkpoint"],
        &["model"],
        &["models"],
        &["LLM"],
    ];
    const BEFORE: &[&[&str]] = &[
        &["large", "language", "model"],
        &["language", "model"],
        &["AI", "model"],
        &["model"],
        &["LLM"],
    ];
    let evidence = |start: usize, end: usize| VocabularyScopeEvidence::LocalContext {
        start_byte: tokens[start].start,
        end_byte: tokens[end - 1].end,
        source: source[tokens[start].start..tokens[end - 1].end].into(),
        active_scope: active.clone(),
    };
    for pattern in AFTER {
        if end < tokens.len()
            && adjacent(source, tokens, end - 1, end)
            && words_match(source, tokens, end, pattern)
        {
            return Some(evidence(first, end + pattern.len()));
        }
    }
    for pattern in BEFORE {
        if first >= pattern.len()
            && adjacent(source, tokens, first - 1, first)
            && words_match(source, tokens, first - pattern.len(), pattern)
            && descriptor_prefix_closed(source, tokens, end)
        {
            return Some(evidence(first - pattern.len(), end));
        }
    }
    None
}

/// A token that could itself be a name: unprotected and within the prose-name
/// shape vocabulary mappings already accept.
fn name_like(token: &Token<'_>) -> bool {
    !token.protected && vocabulary_name(token.word)
}

/// Spelling/naming words: uncertainty or instruction about the identifier.
const VOCABULARY_NAMING_WORDS: &[&str] = &[
    "spelled",
    "spelt",
    "spelling",
    "named",
    "called",
    "pronounced",
];
const VOCABULARY_NAMING_PHRASES: &[&[&str]] = &[
    &["name", "is"],
    &["the", "name", "might", "be"],
    &["written", "as"],
];
/// Literal words for vocabulary candidates, narrower than the address list:
/// only words that speak about the identifier text itself.
const VOCABULARY_LITERAL_WORDS: &[&str] = &[
    "literally",
    "verbatim",
    "exactly",
    "quote",
    "word",
    "words",
    "phrase",
];
const VOCABULARY_HEDGE_WORDS: &[&str] = &[
    "maybe",
    "perhaps",
    "probably",
    "might",
    "may",
    "guess",
    "unsure",
    "uncertain",
    "whether",
];
const VOCABULARY_HEDGE_PHRASES: &[&[&str]] =
    &[&["not", "sure"], &["if", "it's"], &["if", "it", "is"]];

fn be_form(word: &str) -> bool {
    ["be", "is", "are", "was", "were", "am", "it's"]
        .iter()
        .any(|form| word.eq_ignore_ascii_case(form))
}

fn hedge_glue(word: &str) -> bool {
    be_form(word)
        || ["it", "the"]
            .iter()
            .any(|glue| word.eq_ignore_ascii_case(glue))
}

/// Whether the token run `from..to` (the gap between a hedge span and the
/// alias span) keeps them attached: at most two glue tokens, all horizontally
/// adjacent to their neighbors including the hedge and alias edge tokens, and
/// a be-form present whenever the gap is nonempty. A gap of only `the`
/// ("Maybe the Quinn model is fine") is an assessment hedge, not identifier
/// uncertainty.
fn hedge_bridge(source: &str, tokens: &[Token<'_>], from: usize, to: usize) -> bool {
    if to < from || to - from > 2 {
        return false;
    }
    for i in from..to {
        if tokens[i].protected || !hedge_glue(tokens[i].word) {
            return false;
        }
    }
    if to > from && !(from..to).any(|i| be_form(tokens[i].word)) {
        return false;
    }
    // `from` is one past the hedge's last token (or the alias edge); check the
    // pairs (from-1, from) ..= (to-1, to).
    (from..=to).all(|i| adjacent(source, tokens, i - 1, i))
}

fn vocabulary_hedge_attached(
    source: &str,
    tokens: &[Token<'_>],
    first: usize,
    end: usize,
    hedge_start: usize,
    hedge_end: usize,
) -> bool {
    hedge_bridge(source, tokens, hedge_end, first) || hedge_bridge(source, tokens, end, hedge_start)
}

/// Guards for a vocabulary candidate are directed at the identifier itself:
/// an alternation with another name-like token, a spelling/naming word or an
/// unconsumed literal word in the same clause, or a hedge directly attached
/// to the alias. Clause-level guard words and negation never apply.
fn vocabulary_candidate_guarded(
    source: &str,
    tokens: &[Token<'_>],
    first: usize,
    end: usize,
) -> bool {
    // The alias is one side of an alternation with a name-like token
    // ("Quinn or Qwen", "Qwen or Quinn", "either Quinn or ...").
    if first > 0
        && adjacent(source, tokens, first - 1, first)
        && tokens[first - 1].word.eq_ignore_ascii_case("either")
    {
        return true;
    }
    if end + 1 < tokens.len()
        && adjacent(source, tokens, end - 1, end)
        && adjacent(source, tokens, end, end + 1)
        && tokens[end].word.eq_ignore_ascii_case("or")
        && name_like(&tokens[end + 1])
    {
        return true;
    }
    if first >= 2
        && adjacent(source, tokens, first - 2, first - 1)
        && adjacent(source, tokens, first - 1, first)
        && tokens[first - 1].word.eq_ignore_ascii_case("or")
        && name_like(&tokens[first - 2])
    {
        return true;
    }
    let (left, right) = clause_span(source, tokens, first, end);
    for index in left..right {
        if tokens[index].protected {
            continue;
        }
        let word = tokens[index].word;
        if VOCABULARY_NAMING_WORDS
            .iter()
            .any(|naming| word.eq_ignore_ascii_case(naming))
            || VOCABULARY_LITERAL_WORDS
                .iter()
                .any(|literal| word.eq_ignore_ascii_case(literal))
                && !literal_consumed(source, tokens, index)
        {
            return true;
        }
        if VOCABULARY_HEDGE_WORDS
            .iter()
            .any(|hedge| word.eq_ignore_ascii_case(hedge))
            && vocabulary_hedge_attached(source, tokens, first, end, index, index + 1)
        {
            return true;
        }
        for phrase in VOCABULARY_NAMING_PHRASES {
            if index + phrase.len() <= right && words_match(source, tokens, index, phrase) {
                return true;
            }
        }
        for phrase in VOCABULARY_HEDGE_PHRASES {
            if index + phrase.len() <= right
                && words_match(source, tokens, index, phrase)
                && vocabulary_hedge_attached(
                    source,
                    tokens,
                    first,
                    end,
                    index,
                    index + phrase.len(),
                )
            {
                return true;
            }
        }
    }
    // Explicit literal instructions about neighboring sentences still cover
    // this alias occurrence.
    cross_sentence_literal_reference(source, tokens, first)
}

fn discover_vocabulary(
    source: &str,
    revision: &str,
    entries: &[VocabularyEntry],
    active_scope: Option<&str>,
    cancel: &impl Fn() -> bool,
) -> Checked<Vec<VocabularyEdit>> {
    check(cancel)?;
    let tokens = tokenize(source, cancel)?;
    // Reserve even incomplete cued identifiers. A vocabulary alias must never
    // silently repair an account spelling or complete an address for the parser.
    let mut address_owned = vec![false; tokens.len()];
    for index in 0..tokens.len() {
        check(cancel)?;
        if let Some((_, first)) = cue(source, &tokens, index) {
            let (_, end) = clause_span(source, &tokens, first, first + 1);
            address_owned[index..end].fill(true);
        }
    }
    let mut candidates = Vec::new();
    for entry in entries {
        check(cancel)?;
        if entry.status != VocabularyStatus::Confirmed
            || entry.provenance != VocabularyProvenance::ExplicitUserConfirmation
            || entry.scope.trim().is_empty()
            || !vocabulary_name(&entry.canonical)
        {
            continue;
        }
        for alias in &entry.aliases {
            check(cancel)?;
            if !vocabulary_name(alias) {
                continue;
            }
            let words: Vec<_> = alias.split(' ').collect();
            for first in 0..tokens.len() {
                check(cancel)?;
                let end = first + words.len();
                if !words_match(source, &tokens, first, &words)
                    || address_owned[first..end].iter().any(|owned| *owned)
                    || vocabulary_candidate_guarded(source, &tokens, first, end)
                {
                    continue;
                }
                // Bare spoken identifiers also remain outside vocabulary scope.
                if (first > 0
                    && adjacent(source, &tokens, first - 1, first)
                    && STRUCTURAL_WORDS
                        .iter()
                        .any(|word| tokens[first - 1].word.eq_ignore_ascii_case(word)))
                    || (end < tokens.len()
                        && adjacent(source, &tokens, end - 1, end)
                        && STRUCTURAL_WORDS
                            .iter()
                            .any(|word| tokens[end].word.eq_ignore_ascii_case(word)))
                {
                    continue;
                }
                let Some(scope_evidence) =
                    vocabulary_scope(source, &tokens, first, end, &entry.scope, active_scope)
                else {
                    continue;
                };
                let start_byte = tokens[first].start;
                let end_byte = tokens[end - 1].end;
                candidates.push(VocabularyEdit {
                    source_revision: revision.into(),
                    start_byte,
                    end_byte,
                    source: source[start_byte..end_byte].into(),
                    replacement: entry.canonical.clone(),
                    entry: entry.clone(),
                    scope_evidence,
                });
            }
        }
    }
    candidates.sort_by_key(|edit| (edit.start_byte, edit.end_byte));
    // Consider every matching mapping before removing identity mappings. An
    // explicit keep mapping is also evidence of ambiguity with a different one.
    let mut ambiguous = vec![false; candidates.len()];
    for left in 0..candidates.len() {
        check(cancel)?;
        for right in left + 1..candidates.len() {
            check(cancel)?;
            let a = &candidates[left];
            let b = &candidates[right];
            if b.start_byte >= a.end_byte {
                break;
            }
            if a.start_byte != b.start_byte
                || a.end_byte != b.end_byte
                || a.replacement != b.replacement
            {
                ambiguous[left] = true;
                ambiguous[right] = true;
            }
        }
    }
    let mut edits: Vec<VocabularyEdit> = Vec::new();
    for (index, edit) in candidates.into_iter().enumerate() {
        check(cancel)?;
        if !ambiguous[index]
            && edit.source != edit.replacement
            && !edits.last().is_some_and(|previous| {
                previous.start_byte == edit.start_byte && previous.end_byte == edit.end_byte
            })
        {
            edits.push(edit);
        }
    }
    Ok(edits)
}

fn apply_vocabulary_checked(
    source: &str,
    revision: &str,
    edits: &[Edit],
    vocabulary_edits: &[VocabularyEdit],
    entries: &[VocabularyEntry],
    active_scope: Option<&str>,
    cancel: &impl Fn() -> bool,
) -> Checked<VocabularyNormalization> {
    // Keep the address validator's finite grammar and component accounting intact.
    apply_checked(source, revision, edits, cancel)?;
    let canonical = discover_vocabulary(source, revision, entries, active_scope, cancel)?;
    let mut previous_end = 0;
    let mut canonical_index = 0;
    for edit in vocabulary_edits {
        check(cancel)?;
        if edit.source_revision != revision
            || edit.start_byte < previous_end
            || edit.start_byte >= edit.end_byte
            || source.get(edit.start_byte..edit.end_byte) != Some(edit.source.as_str())
        {
            return Err(Failure::Invalid(
                "Stale, overlapping, unsorted, or mismatched vocabulary edit.".into(),
            ));
        }
        while canonical_index < canonical.len()
            && canonical[canonical_index].start_byte < edit.start_byte
        {
            canonical_index += 1;
        }
        if canonical.get(canonical_index) != Some(edit) {
            return Err(Failure::Invalid("Vocabulary edit lacks its complete confirmed mapping and local/active scope evidence.".into()));
        }
        previous_end = edit.end_byte;
        canonical_index += 1;
    }
    let mut replacements: Vec<_> = edits
        .iter()
        .map(|edit| (edit.start_byte, edit.end_byte, edit.replacement.as_str()))
        .chain(
            vocabulary_edits
                .iter()
                .map(|edit| (edit.start_byte, edit.end_byte, edit.replacement.as_str())),
        )
        .collect();
    replacements.sort_by_key(|edit| edit.0);
    let mut text = String::with_capacity(source.len());
    let mut retained_start = 0;
    for (start, end, replacement) in replacements {
        check(cancel)?;
        if start < retained_start {
            return Err(Failure::Invalid(
                "Address and vocabulary edits overlap in the raw source.".into(),
            ));
        }
        text.push_str(&source[retained_start..start]);
        text.push_str(replacement);
        retained_start = end;
    }
    text.push_str(&source[retained_start..]);
    check(cancel)?;
    Ok(VocabularyNormalization {
        text,
        edits: edits.to_vec(),
        vocabulary_edits: vocabulary_edits.to_vec(),
        source_revision: revision.into(),
        cancelled: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn run(source: &str) -> Normalization {
        normalize(source, &AtomicBool::new(false)).unwrap()
    }

    fn unchanged(source: &str) {
        let result = run(source);
        assert_eq!(result.text, source);
        assert!(result.edits.is_empty(), "{source}");
        assert!(!result.cancelled);
    }

    #[test]
    fn supplied_positive_emails_and_url() {
        for (source, expected) in [
            (
                "Email me at alex at example dot com.",
                "Email me at alex@example.com.",
            ),
            (
                "Email alex dot smith plus work at example dot co dot uk.",
                "Email alex.smith+work@example.co.uk.",
            ),
            (
                "Email me at alex plus digit seven at example dot com.",
                "Email me at alex+7@example.com.",
            ),
            (
                "Go to example dot com slash docs slash setup.",
                "Go to example.com/docs/setup.",
            ),
        ] {
            let result = run(source);
            assert_eq!(result.text, expected, "{source}");
            assert_eq!(result.edits.len(), 1);
            assert_eq!(result, run(source), "deterministic repeated run");
            validate_accounting(source, &result.edits[0]).unwrap();
        }
    }

    #[test]
    fn supplied_negative_structures_and_literal() {
        for source in [
            "I met Alex at Example. Dot com companies were discussed.",
            "Say the words alex at example dot com literally.",
            "Use Alex at Harbor Vail Studio.",
            "the Alex at Harbor Vale dot example dot com. The portal rejects my passphrase, and when I enter Alex at Harbor Vail Studio, it reports that no membership matches this address.",
            "Open https://example.com/API/v1?Key=AbC#Top.",
            "Use release slash one dot five, not release slash fifteen.",
        ] { unchanged(source); }
    }

    #[test]
    fn preserves_digits_tags_labels_case_and_outer_punctuation() {
        for (source, expected) in [
            ("Email: Alex underscore Smith hyphen 2 plus Work dot digit zero digit seven at Mail dot Example dash Lab dot co dot uk!",
             "Email: Alex_Smith-2+Work.07@mail.example-lab.co.uk!"),
            ("Open https colon slash slash WWW dot Example dot com slash API slash v1 question mark Key equals AbC ampersand ID equals digit zero digit seven hash Top.",
             "Open https://www.example.com/API/v1?Key=AbC&ID=07#Top."),
            ("Visit http colon slash slash example dot org slash Docs dot HTML hashtag Top_2.",
             "Visit http colon slash slash example dot org slash Docs dot HTML hashtag Top_2."),
            ("Open example dot com slash Docs dot HTML hash Top underscore 2; keep this.",
             "Open example.com/Docs.HTML#Top_2; keep this."),
            ("Visit example dot net slash.", "Visit example.net/."),
            ("Email alex at example dot com, and keep all surrounding prose.", "Email alex@example.com, and keep all surrounding prose."),
        ] { assert_eq!(run(source).text, expected, "{source}"); }
    }

    #[test]
    fn unicode_coordinates_and_full_tail() {
        let source = "José 🙂: Email alex at example dot com. 尾部 naïve remains.\nGo to example dot com slash API.\n最後\n";
        let result = run(source);
        assert_eq!(
            result.text,
            "José 🙂: Email alex@example.com. 尾部 naïve remains.\nGo to example.com/API.\n最後\n"
        );
        assert_eq!(result.edits.len(), 2);
        assert_eq!(result.edits[0].start_byte, source.find("alex").unwrap());
        assert_eq!(result.source_revision, source_revision(source));
        for edit in &result.edits {
            assert_eq!(
                source.get(edit.start_byte..edit.end_byte),
                Some(edit.source.as_str())
            );
            validate_accounting(source, edit).unwrap();
        }
        assert!(result.edits[0].end_byte <= result.edits[1].start_byte);
    }

    #[test]
    fn existing_literals_and_unrelated_numeric_prose_are_exact() {
        for source in [
            "Email Alex.Smith+Work@example.co.uk.",
            "Open example.com/API/v1?Key=AbC#Top.",
            "Email alex at example.com.",
            "Open example dot com/API.",
            "Use System.Text.Json rather than Newtonsoft.Json.",
            "Keep version 1.10, not 1.1.",
            "Use minus five, not five.",
            "The ID is zero zero seven.",
            "Schedule it for three four twenty six.",
            "Store it in C:\\Temp\\NewBuild, not C:\\Temp\\OldBuild.",
            "Ask José about naïve Bayes.",
            "  I um... I THINK so\n",
            "",
        ] {
            unchanged(source);
        }
    }

    #[test]
    fn quoted_code_literal_language_and_uncertainty_are_protected() {
        for source in [
            "Say the words Email alex at example dot com literally.",
            "Write the exact phrase: Email alex at example dot com.",
            "Keep the following words literally.\nEmail alex at example dot com.",
            "Email alex at example dot com. Preserve those words verbatim.",
            "Keep Email alex at example dot com unchanged.",
            "Email alex at example dot com maybe.",
            "Email alex at example dot com or alex at another dot com.",
            "I think Email alex at example dot com.",
            "He said \"Please Email alex at example dot com.\"",
            "He said ‘Please Email alex at example dot com.’",
            "He said 'Please Email alex at example dot com.'",
            "Run `Please Email alex at example dot com.`",
            "```text\nEmail alex at example dot com.\n```",
            "\"unfinished\nEmail alex at example dot com.",
            "> Please Email alex at example dot com.",
            "    Email alex at example dot com.",
        ] {
            unchanged(source);
        }
        assert_eq!(
            run("`literal`\nEmail alex at example dot com.").text,
            "`literal`\nEmail alex@example.com."
        );
        // Negating the instruction does not guard the confirmed span itself.
        assert_eq!(
            run("Do not normalize Email alex at example dot com.").text,
            "Do not normalize Email alex@example.com."
        );
    }

    #[test]
    fn unsupported_or_ambiguous_components_abstain_as_a_whole() {
        for source in [
            "alex at example dot com.",
            "example dot com slash docs.",
            "Email Alex at Harbor Vale dot example dot com.",
            "Email Alex at Harbor Vail Studio.",
            "Email alex at example.",
            "Email alex at example dot unknownsuffix.",
            "Email alex at example dot com dot.",
            "Email alex at example dot com plus tag.",
            "Email alex plus at example dot com.",
            "Email alex plus work plus backup at example dot com.",
            "Email alex plus digit eleven at example dot com.",
            "Email alex smith at example dot com.",
            "Email alex at example dot com extra dot uk.",
            "Email alex at example dot com underscore uk.",
            "Email alex at example dot com, plus work.",
            "Email alex at example dot com. Dot uk.",
            "Email alex at example. Dot com.",
            "Email alex at example\ndot com.",
            "Email álëx at example dot com.",
            "Email alex at éxample dot com.",
            "Open example dot com slash Docs question mark Key equals.",
            "Open example dot com slash Docs question mark Key equals AbC ampersand.",
            "Open example dot com slash Docs question mark Key equals AbC hash.",
            "Open example dot com slash Docs percent twenty.",
            "Open example dot com port 8080.",
            "Open ftp colon slash slash example dot com.",
            "Open example dot com, slash API.",
            "Open example dot com slash Docs. Question mark Key equals AbC.",
            "Open example dot com slash Docs slash digit eleven.",
            "Open example dot com slash Docs unknown slash Tail.",
        ] {
            unchanged(source);
        }
    }

    #[test]
    fn validator_rejects_stale_source_mismatch_and_invalid_coordinates() {
        let source = "🙂 Email alex plus work at example dot com. Full tail.";
        let original = run(source).edits;
        let cancel = AtomicBool::new(false);
        assert!(apply_edits(&format!("{source} changed"), &original, &cancel).is_err());
        for mutation in 0..5 {
            let mut edits = original.clone();
            match mutation {
                0 => edits[0].source_revision = "stale".into(),
                1 => edits[0].source = "different".into(),
                2 => edits[0].start_byte = 1,
                3 => edits[0].end_byte = source.len() + 1,
                _ => edits[0].origin = "model".into(),
            }
            assert!(apply_edits(source, &edits, &cancel).is_err());
        }
    }

    #[test]
    fn validator_rejects_component_loss_even_with_forged_consistent_accounting() {
        let cancel = AtomicBool::new(false);
        for source in [
            "Email alex plus digit seven at example dot co dot uk.",
            "Open example dot com slash API slash v1 question mark Key equals AbC ampersand ID equals 7 hash Top.",
        ] {
            let original = run(source).edits;
            assert_eq!(original.len(), 1);
            for index in 0..original[0].components.len() {
                if original[0].components[index].replacement.is_empty() { continue; }
                let mut edits = original.clone();
                edits[0].components[index].replacement.clear();
                edits[0].replacement = edits[0].components.iter().map(|part| part.replacement.as_str()).collect();
                assert!(apply_edits(source, &edits, &cancel).is_err(), "lost component {index}: {source}");
            }
            let mut edits = original.clone();
            edits[0].components.pop();
            assert!(apply_edits(source, &edits, &cancel).is_err());
        }
    }

    #[test]
    fn validator_rejects_shortened_url_span_and_forged_eligibility() {
        let source = "Open example dot com slash API question mark Key equals AbC hash Top.";
        let mut edits = run(source).edits;
        let shortened_end = source.find(" slash").unwrap();
        edits[0].end_byte = shortened_end;
        edits[0].source = source[edits[0].start_byte..shortened_end].into();
        edits[0].replacement = "example.com".into();
        edits[0]
            .components
            .retain(|part| part.end_byte <= shortened_end);
        assert!(apply_edits(source, &edits, &AtomicBool::new(false)).is_err());
        let source = "Maybe Email alex at example dot com.";
        let mut forged = run("Email alex at example dot com.").edits;
        for edit in &mut forged {
            edit.source_revision = source_revision(source);
            edit.start_byte += 6;
            edit.end_byte += 6;
            for part in &mut edit.components {
                part.start_byte += 6;
                part.end_byte += 6;
            }
        }
        assert!(apply_edits(source, &forged, &AtomicBool::new(false)).is_err());
    }

    #[test]
    fn validator_rejects_forged_case_and_accounting_roles() {
        let source = "Open example dot com slash API question mark Key equals AbC hash Top.";
        let original = run(source).edits;
        let cancel = AtomicBool::new(false);
        let mut edits = original.clone();
        for part in &mut edits[0].components {
            part.replacement.make_ascii_lowercase();
        }
        edits[0].replacement.make_ascii_lowercase();
        assert!(apply_edits(source, &edits, &cancel).is_err());
        let mut edits = original;
        edits[0].components[0].role = "untrusted_alias".into();
        assert!(apply_edits(source, &edits, &cancel).is_err());
    }

    #[test]
    fn multiple_edits_are_idempotent_and_preserve_prose_connectors() {
        let source = "Please Email alex at example dot com and open example dot org slash Docs, then retain the tail 🙂.";
        let result = run(source);
        assert_eq!(result.edits.len(), 2);
        assert_eq!(
            result.text,
            "Please Email alex@example.com and open example.org/Docs, then retain the tail 🙂."
        );
        unchanged(&result.text);
    }

    #[test]
    fn grammar_limits_abstain_without_partial_edits() {
        unchanged(&format!("Email {} at example dot com.", "a".repeat(65)));
        unchanged(&format!("Email alex at {} dot com.", "a".repeat(64)));
        unchanged(&format!(
            "Open example dot com slash {}.",
            "A".repeat(MAX_ATOM_BYTES + 1)
        ));
        unchanged(&format!(
            "Open example dot com{} Tail.",
            " slash A".repeat(MAX_COMPONENT_WORDS)
        ));
        unchanged("Email alex\u{a0}at example dot com.");
    }

    #[test]
    fn application_requires_order_nonoverlap_and_allows_independent_subset() {
        let source = "Email alex at example dot com. Go to example dot org slash Docs. Tail.";
        let original = run(source).edits;
        assert_eq!(original.len(), 2);
        let cancel = AtomicBool::new(false);
        assert_eq!(
            apply_edits(source, &original[1..], &cancel).unwrap().text,
            "Email alex at example dot com. Go to example.org/Docs. Tail."
        );
        assert!(apply_edits(source, &[original[0].clone(), original[0].clone()], &cancel).is_err());
        assert!(apply_edits(source, &[original[1].clone(), original[0].clone()], &cancel).is_err());
    }

    #[test]
    fn cancellation_before_work_returns_complete_original() {
        let source = "Email alex at example dot com.\nKeep this entire tail 🙂\n";
        let result = normalize(source, &AtomicBool::new(true)).unwrap();
        assert!(result.cancelled);
        assert_eq!(result.text, source);
        assert!(result.edits.is_empty());
        let result = apply_edits(source, &run(source).edits, &AtomicBool::new(true)).unwrap();
        assert!(result.cancelled);
        assert_eq!(result.text, source);
        assert!(result.edits.is_empty());
    }

    #[test]
    fn cancellation_at_every_checkpoint_never_loses_source_or_tail() {
        let source = "Email alex plus work at example dot com.\nOpen example dot com slash API hash Top.\nFull tail 🙂\n";
        let total = Cell::new(0);
        let expected = normalize_checked(source, &|| {
            total.set(total.get() + 1);
            false
        })
        .unwrap();
        assert_eq!(expected.edits.len(), 2);
        for stop_at in 1..=total.get() {
            let calls = Cell::new(0);
            let result = normalize_checked(source, &|| {
                calls.set(calls.get() + 1);
                calls.get() >= stop_at
            })
            .unwrap();
            assert!(result.cancelled, "checkpoint {stop_at}");
            assert_eq!(result.text, source, "checkpoint {stop_at}");
            assert!(result.edits.is_empty());
            assert_eq!(result.source_revision, expected.source_revision);
        }
    }

    fn confirmed_entry(canonical: &str, aliases: &[&str], scope: &str) -> VocabularyEntry {
        VocabularyEntry {
            id: None,
            canonical: canonical.into(),
            aliases: aliases.iter().map(|alias| (*alias).into()).collect(),
            scope: scope.into(),
            status: VocabularyStatus::Confirmed,
            provenance: VocabularyProvenance::ExplicitUserConfirmation,
        }
    }

    fn qwen_entry() -> VocabularyEntry {
        confirmed_entry("Qwen", &["Quinn"], "AI model discussion")
    }

    fn with_vocabulary(
        source: &str,
        entries: &[VocabularyEntry],
        scope: Option<&str>,
    ) -> VocabularyNormalization {
        normalize_with_vocabulary(source, entries, scope, &AtomicBool::new(false)).unwrap()
    }

    #[test]
    fn round_two_structured_regressions_r205_r211_r216_r217() {
        let fixtures =
            include_str!("../../evaluation/round2-review-pack/regression_additions.jsonl");
        let mut checked = 0;
        for line in fixtures.lines() {
            let row: serde_json::Value = serde_json::from_str(line).unwrap();
            if ![
                "R205", "R206", "R207", "R208", "R209", "R210", "R211", "R216", "R217",
            ]
            .contains(&row["id"].as_str().unwrap())
            {
                continue;
            }
            let source = row["input_text"].as_str().unwrap();
            let entries: Vec<VocabularyEntry> = row
                .get("confirmed_vocabulary")
                .map(|value| serde_json::from_value(value.clone()).unwrap())
                .unwrap_or_default();
            let result = with_vocabulary(source, &entries, None);
            assert_eq!(
                result.text,
                row["required_assertions"]["exact_text"].as_str().unwrap(),
                "{}",
                row["id"]
            );
            assert!(!result.cancelled);
            assert_eq!(result.source_revision, source_revision(source));
            checked += 1;
        }
        assert_eq!(checked, 9);
    }

    #[test]
    fn literal_and_uncertainty_scope_is_candidate_local() {
        for prefix in [
            "Update the code.",
            "The error string is empty.",
            "The exact count is ten.",
            "Maybe deploy later.",
            "Do not deploy yet.",
            "The spelling is correct.",
        ] {
            let source = format!("{prefix} Email alex at example dot com.");
            assert_eq!(
                run(&source).text,
                format!("{prefix} Email alex@example.com.")
            );
        }
        assert_eq!(
            run("Email alex at example dot com, but do not deploy yet. Maybe Contact bob at example dot org.").text,
            "Email alex@example.com, but do not deploy yet. Maybe Contact bob at example dot org."
        );
        assert_eq!(
            run("Email alex at example dot com and do not compile yet.").text,
            "Email alex@example.com and do not compile yet."
        );
        // Negating the action is not uncertainty about the address spelling.
        assert_eq!(
            run("Do not Contact alex at example dot com.").text,
            "Do not Contact alex@example.com."
        );
        // A literal word in another comma-separated segment does not veto the
        // candidate unless that segment is itself a literal instruction.
        assert_eq!(
            run("Update the code, then email sam at comet dot io.").text,
            "Update the code, then email sam@comet.io."
        );
        assert_eq!(
            run("Keep `user_id` unchanged, then email sam at comet dot io.").text,
            "Keep `user_id` unchanged, then email sam@comet.io."
        );
        for source in [
            "Contact alex at example dot com, but maybe alex at example dot net.",
            "Contact alex at example dot com, but keep these words verbatim.",
            "Send it to alex at example dot com, or contact bob at example dot org.",
            "Write the following exact string. Contact alex at example dot com.",
            "Contact alex at example dot com. Keep that exact phrase.",
        ] {
            unchanged(source);
        }
    }

    #[test]
    fn contact_and_send_cues_keep_near_misses_and_incomplete_addresses() {
        for source in [
            "Contact Nora at Beacon. Dot net was an example.",
            "Contact Nora at Beacon dot net offices tomorrow.",
            "Contact lens at example dot com.",
            "The contact string is alex at example dot com.",
            "Send it to Iris at Beacon. Dot net companies were discussed.",
            "Send it to Iris at Beacon tomorrow.",
            "Send it to iris at beacon dot.",
            "Send it to iris at beacon dot net dot.",
            "Send it to iris at beacon dot net. Dot uk.",
            "Send it to iris at beacon dot unknownsuffix.",
            "Send this to iris at beacon dot net.",
            "Send it\nto iris at beacon dot net.",
            "Send it to iris at beacon dot net or iris at another dot net.",
            "Contact alex at example dot com literally.",
            "`Contact alex at example dot com.`",
            "\"Send it to iris at beacon dot net.\"",
        ] {
            unchanged(source);
        }
    }

    #[test]
    fn vocabulary_requires_both_confirmation_fields_and_round_trips_serde() {
        let source = "We benchmarked the Quinn model for speech recognition.";
        let entry = qwen_entry();
        let json = serde_json::to_string(&entry).unwrap();
        assert_eq!(
            serde_json::from_str::<VocabularyEntry>(&json).unwrap(),
            entry
        );
        for status in [VocabularyStatus::Pending, VocabularyStatus::Rejected] {
            let mut entry = qwen_entry();
            entry.status = status;
            assert_eq!(with_vocabulary(source, &[entry], None).text, source);
        }
        for provenance in [
            VocabularyProvenance::Observed,
            VocabularyProvenance::ModelSuggested,
            VocabularyProvenance::Unverified,
        ] {
            let mut entry = qwen_entry();
            entry.provenance = provenance;
            assert_eq!(with_vocabulary(source, &[entry], None).text, source);
        }
        for json in [
            r#"{"canonical":"Qwen","aliases":["Quinn"],"scope":"AI model discussion"}"#,
            r#"{"canonical":"Qwen","aliases":["Quinn"],"scope":"AI model discussion","status":"suggested","provenance":"explicit_user_confirmation"}"#,
            r#"{"canonical":"Qwen","aliases":["Quinn"],"scope":"AI model discussion","status":"confirmed","provenance":"imported"}"#,
        ] {
            let entry: VocabularyEntry = serde_json::from_str(json).unwrap();
            let result = with_vocabulary(source, &[entry], Some("AI model discussion"));
            assert_eq!(result.text, source);
            assert!(result.vocabulary_edits.is_empty());
        }
    }

    #[test]
    fn vocabulary_scope_requires_attached_evidence_or_explicit_active_scope() {
        let entries = [qwen_entry()];
        // No descriptor attached to the alias occurrence: abstain even when
        // the active scope matches. A topic is retrieval context, not
        // referent evidence.
        for source in [
            "Quinn emailed me after lunch.",
            "We discussed an AI model. Quinn emailed me after lunch.",
            "The AI model was fast and Quinn thanked us.",
            "Quinn read the AI model report.",
            "The model for speech recognition was tested by Quinn.",
            "We benchmarked Quinn.",
            "Quinn is presenting the AI model results.",
            "We benchmarked Quinn. Model for speech recognition is the topic.",
            "We benchmarked Quinn\nmodel for speech recognition.",
        ] {
            for scope in [None, Some("AI model discussion")] {
                assert_eq!(
                    with_vocabulary(source, &entries, scope).text,
                    source,
                    "{source} (scope {scope:?})"
                );
            }
        }
        for (source, expected) in [
            (
                "We benchmarked the Quinn model.",
                "We benchmarked the Qwen model.",
            ),
            (
                "We benchmarked the Quinn models.",
                "We benchmarked the Qwen models.",
            ),
            (
                "We benchmarked the Quinn checkpoint.",
                "We benchmarked the Qwen checkpoint.",
            ),
            (
                "We benchmarked the Quinn AI model.",
                "We benchmarked the Qwen AI model.",
            ),
            (
                "We benchmarked the Quinn language model.",
                "We benchmarked the Qwen language model.",
            ),
            (
                "We benchmarked the Quinn LLM.",
                "We benchmarked the Qwen LLM.",
            ),
            (
                "We benchmarked the AI model Quinn.",
                "We benchmarked the AI model Qwen.",
            ),
            (
                "We benchmarked the model Quinn.",
                "We benchmarked the model Qwen.",
            ),
            (
                "We benchmarked the LLM Quinn.",
                "We benchmarked the LLM Qwen.",
            ),
        ] {
            let result = with_vocabulary(source, &entries, None);
            assert_eq!(result.text, expected);
            assert!(matches!(
                result.vocabulary_edits[0].scope_evidence,
                VocabularyScopeEvidence::LocalContext { .. }
            ));
        }
        // A matching active scope is recorded alongside attached evidence.
        let result = with_vocabulary(
            "We benchmarked the Quinn model.",
            &entries,
            Some("AI model discussion"),
        );
        assert_eq!(result.text, "We benchmarked the Qwen model.");
        assert!(matches!(
            &result.vocabulary_edits[0].scope_evidence,
            VocabularyScopeEvidence::LocalContext {
                active_scope: Some(scope),
                ..
            } if same_scope(scope, "AI model discussion")
        ));
        assert_eq!(
            with_vocabulary("We benchmarked Quinn.", &entries, Some("person names")).text,
            "We benchmarked Quinn."
        );
        assert_eq!(
            with_vocabulary(
                "Quinn emailed me after lunch.",
                &entries,
                Some("AI model discussion")
            )
            .text,
            "Quinn emailed me after lunch."
        );
    }

    #[test]
    fn vocabulary_preserves_literal_uncertain_and_identifier_contexts() {
        let entries = [qwen_entry()];
        for source in [
            "Say the words Quinn model for speech recognition literally.",
            "Keep the following words literally.\nQuinn model for speech recognition.",
            "Quinn model for speech recognition. Preserve those words verbatim.",
            "The name might be Quinn or Qwen.",
            "Benchmark the model Quinn or Qwen.",
            "Benchmark either Quinn or Qwen.",
            "\"Quinn model for speech recognition\"",
            "`Quinn` model for speech recognition.",
            "Quinn `model for speech recognition`.",
            "    Quinn model for speech recognition.",
            "> Quinn model for speech recognition.",
            "Quinnish model for speech recognition.",
            "Quinn_model for speech recognition.",
            "Quinn@example.com",
            "https://example.com/Quinn",
            "Email Quinn at example.",
            "Contact Quinn at example dot unknownsuffix.",
            "Quinn at example dot com",
        ] {
            let result = with_vocabulary(source, &entries, None);
            assert_eq!(result.text, source, "{source}");
            assert!(result.vocabulary_edits.is_empty());
        }
        // A hedge or alternation about the assessment, not the identifier,
        // does not guard the confirmed alias span.
        for (source, expected) in [
            (
                "Maybe the Quinn model for speech recognition.",
                "Maybe the Qwen model for speech recognition.",
            ),
            (
                "Quinn model for speech recognition or another model.",
                "Qwen model for speech recognition or another model.",
            ),
            (
                "Do not use the Quinn model for speech recognition.",
                "Do not use the Qwen model for speech recognition.",
            ),
            (
                "I think the Quinn language model is too slow.",
                "I think the Qwen language model is too slow.",
            ),
        ] {
            assert_eq!(
                with_vocabulary(source, &entries, None).text,
                expected,
                "{source}"
            );
        }
        for source in [
            "Email Quinn at example.",
            "Contact Quinn at example dot unknownsuffix.",
            "Quinn at example dot com",
        ] {
            assert_eq!(
                with_vocabulary(source, &entries, Some("AI model discussion")).text,
                source
            );
        }
        let source = "Contact Quinn at example dot com.";
        let result = with_vocabulary(source, &entries, Some("AI model discussion"));
        assert_eq!(result.text, "Contact Quinn@example.com.");
        assert!(result.vocabulary_edits.is_empty());
    }

    #[test]
    fn vocabulary_alternation_guards_are_candidate_directed() {
        for source in ["Quinn or Qwen", "either Quinn or"] {
            let tokens = tokenize(source, &|| false).unwrap();
            let first = tokens
                .iter()
                .position(|token| token.word.eq_ignore_ascii_case("Quinn"))
                .unwrap();
            assert!(vocabulary_candidate_guarded(
                source,
                &tokens,
                first,
                first + 1
            ));
        }

        let source = "Quinn model or another model";
        let tokens = tokenize(source, &|| false).unwrap();
        assert!(!vocabulary_candidate_guarded(source, &tokens, 0, 1));
        assert_eq!(
            with_vocabulary(source, &[qwen_entry()], None).text,
            "Qwen model or another model"
        );
    }

    #[test]
    fn vocabulary_applies_only_complete_multiword_mappings() {
        let entries = [confirmed_entry(
            "Qwen Two",
            &["quinn two", "cue too"],
            "AI model discussion",
        )];
        let source =
            "Benchmark the quinn two model and the cue too model. Keep the quinn two phrase.";
        // The literal request belongs to the second sentence only.
        let result = with_vocabulary(source, &entries, Some("AI model discussion"));
        assert_eq!(
            result.text,
            "Benchmark the Qwen Two model and the Qwen Two model. Keep the quinn two phrase."
        );
        assert_eq!(result.vocabulary_edits.len(), 2);
        for source in [
            "Benchmark the quinn model.",
            "Benchmark the quinn. Two models.",
            "Benchmark the quinn\ntwo model.",
            "Benchmark the quinn twoo model.",
        ] {
            assert_eq!(
                with_vocabulary(source, &entries, Some("AI model discussion")).text,
                source
            );
        }
        // A complete multiword alias without an attached descriptor abstains;
        // the active scope is never sufficient on its own.
        assert_eq!(
            with_vocabulary("Use quinn two.", &entries, Some("AI model discussion")).text,
            "Use quinn two."
        );
        let result = with_vocabulary("Benchmark the quinn\ttwo  model.", &entries, None);
        assert_eq!(result.text, "Benchmark the Qwen Two  model.");
        assert_eq!(result.vocabulary_edits[0].source, "quinn\ttwo");
    }

    #[test]
    fn vocabulary_conflicting_and_overlapping_mappings_abstain() {
        let source = "We benchmarked the Quinn model for speech recognition.";
        for entries in [
            vec![
                qwen_entry(),
                confirmed_entry("Other", &["Quinn"], "AI model discussion"),
            ],
            vec![
                qwen_entry(),
                confirmed_entry("Quinn", &["Quinn"], "AI model discussion"),
            ],
        ] {
            assert_eq!(with_vocabulary(source, &entries, None).text, source);
        }
        let entries = [
            confirmed_entry("Editor", &["visual studio"], "editors"),
            confirmed_entry("IDE", &["studio coat"], "editors"),
        ];
        assert_eq!(
            with_vocabulary("Use visual studio coat.", &entries, Some("editors")).text,
            "Use visual studio coat."
        );
        let entries = [qwen_entry(), qwen_entry()];
        assert_eq!(
            with_vocabulary(source, &entries, None)
                .vocabulary_edits
                .len(),
            1
        );
    }

    #[test]
    fn vocabulary_never_supplies_an_email_suffix_or_address_mapping() {
        for canonical in [
            "alex@example.com",
            "example.com",
            "https://example.com",
            "../example",
            "",
            " ",
            "Qwen.",
        ] {
            let entries = [confirmed_entry(canonical, &["example"], "accounts")];
            for source in [
                "example",
                "Contact alex at example.",
                "Email alex at example dot com.",
            ] {
                let result = with_vocabulary(source, &entries, Some("accounts"));
                assert_eq!(result.text, run(source).text);
                assert!(result.vocabulary_edits.is_empty());
            }
        }
    }

    #[test]
    fn vocabulary_and_address_edits_compose_against_original_unicode_source() {
        let entries = [confirmed_entry(
            "Longer Model Name",
            &["Quinn"],
            "AI model discussion",
        )];
        let source =
            "🙂 Quinn language model. Contact alex at example dot com. Quinn AI model. 尾部\n";
        let result = with_vocabulary(source, &entries, None);
        assert_eq!(result.text, "🙂 Longer Model Name language model. Contact alex@example.com. Longer Model Name AI model. 尾部\n");
        assert_eq!(result.edits.len(), 1);
        assert_eq!(result.vocabulary_edits.len(), 2);
        for edit in &result.vocabulary_edits {
            assert_eq!(&source[edit.start_byte..edit.end_byte], edit.source);
            assert_eq!(edit.source_revision, source_revision(source));
        }
        let cancel = AtomicBool::new(false);
        assert_eq!(
            apply_vocabulary_edits(
                source,
                &result.edits,
                &result.vocabulary_edits,
                &entries,
                None,
                &cancel
            )
            .unwrap(),
            result
        );
        assert_eq!(
            apply_vocabulary_edits(
                source,
                &result.edits,
                &result.vocabulary_edits[1..],
                &entries,
                None,
                &cancel
            )
            .unwrap()
            .text,
            "🙂 Quinn language model. Contact alex@example.com. Longer Model Name AI model. 尾部\n"
        );
        assert_eq!(
            apply_edits(source, &result.edits, &cancel).unwrap().text,
            "🙂 Quinn language model. Contact alex@example.com. Quinn AI model. 尾部\n"
        );
        assert!(with_vocabulary(&result.text, &entries, None)
            .vocabulary_edits
            .is_empty());
    }

    #[test]
    fn vocabulary_validator_rejects_forged_stale_or_unconfirmed_edits() {
        let entries = [qwen_entry()];
        let source = "🙂 Quinn language model. Quinn AI model. Tail.";
        let result = with_vocabulary(source, &entries, None);
        let cancel = AtomicBool::new(false);
        for mutation in 0..8 {
            let mut edits = result.vocabulary_edits.clone();
            match mutation {
                0 => edits[0].source_revision = "stale".into(),
                1 => edits[0].source = "Wrong".into(),
                2 => edits[0].replacement = "Wrong".into(),
                3 => edits[0].start_byte = 1,
                4 => edits[0].end_byte = source.len() + 1,
                5 => edits[0].entry.provenance = VocabularyProvenance::Observed,
                6 => {
                    edits[0].scope_evidence = VocabularyScopeEvidence::ActiveScope {
                        scope: "AI model discussion".into(),
                    }
                }
                _ => edits.swap(0, 1),
            }
            assert!(
                apply_vocabulary_edits(source, &[], &edits, &entries, None, &cancel).is_err(),
                "mutation {mutation}"
            );
        }
        assert!(apply_vocabulary_edits(
            &format!("{source} changed"),
            &[],
            &result.vocabulary_edits,
            &entries,
            None,
            &cancel
        )
        .is_err());
        let mut pending = entries.clone();
        pending[0].status = VocabularyStatus::Pending;
        assert!(apply_vocabulary_edits(
            source,
            &[],
            &result.vocabulary_edits,
            &pending,
            None,
            &cancel
        )
        .is_err());
        assert!(apply_vocabulary_edits(
            source,
            &[],
            &[
                result.vocabulary_edits[0].clone(),
                result.vocabulary_edits[0].clone()
            ],
            &entries,
            None,
            &cancel
        )
        .is_err());
        // A caller cannot smuggle a generic alias through address validation.
        let edit = &result.vocabulary_edits[0];
        let forged = Edit {
            source_revision: edit.source_revision.clone(),
            start_byte: edit.start_byte,
            end_byte: edit.end_byte,
            source: edit.source.clone(),
            replacement: edit.replacement.clone(),
            origin: EMAIL_ORIGIN.into(),
            components: vec![Component {
                start_byte: edit.start_byte,
                end_byte: edit.end_byte,
                source: edit.source.clone(),
                replacement: edit.replacement.clone(),
                role: "local_part".into(),
            }],
        };
        assert!(apply_edits(source, &[forged], &cancel).is_err());
    }

    #[test]
    fn vocabulary_active_scope_must_still_be_present_at_application() {
        let entries = [qwen_entry()];
        // The matching active scope is recorded inside the attached evidence,
        // so replaying without it fails the canonical re-derivation.
        let source = "Benchmark the Quinn model.";
        let result = with_vocabulary(source, &entries, Some("AI model discussion"));
        assert_eq!(result.vocabulary_edits.len(), 1);
        assert!(apply_vocabulary_edits(
            source,
            &[],
            &result.vocabulary_edits,
            &entries,
            None,
            &AtomicBool::new(false)
        )
        .is_err());

        let mut forged = result.vocabulary_edits.clone();
        let VocabularyScopeEvidence::LocalContext { active_scope, .. } =
            &mut forged[0].scope_evidence
        else {
            panic!("Expected local-context evidence");
        };
        *active_scope = Some("different scope".into());
        assert!(apply_vocabulary_edits(
            source,
            &[],
            &forged,
            &entries,
            Some("AI model discussion"),
            &AtomicBool::new(false)
        )
        .is_err());
    }

    #[test]
    fn cross_stage_cancellation_at_every_checkpoint_rolls_back_both_stages() {
        let source =
            "🙂 Quinn language model. Contact alex at example dot com. Quinn AI model. Full tail\n";
        let entries = [qwen_entry()];
        let total = Cell::new(0);
        let expected = normalize_with_vocabulary_checked(source, &entries, None, &|| {
            total.set(total.get() + 1);
            false
        })
        .unwrap();
        assert_eq!(expected.edits.len(), 1);
        assert_eq!(expected.vocabulary_edits.len(), 2);
        for stop_at in 1..=total.get() {
            let calls = Cell::new(0);
            let result = normalize_with_vocabulary_checked(source, &entries, None, &|| {
                calls.set(calls.get() + 1);
                calls.get() >= stop_at
            })
            .unwrap();
            assert!(result.cancelled, "checkpoint {stop_at}");
            assert_eq!(result.text, source, "checkpoint {stop_at}");
            assert!(result.edits.is_empty());
            assert!(result.vocabulary_edits.is_empty());
            assert_eq!(result.source_revision, expected.source_revision);
        }
        let cancelled = apply_vocabulary_edits(
            source,
            &expected.edits,
            &expected.vocabulary_edits,
            &entries,
            None,
            &AtomicBool::new(true),
        )
        .unwrap();
        assert!(cancelled.cancelled);
        assert_eq!(cancelled.text, source);
        assert!(cancelled.edits.is_empty());
        assert!(cancelled.vocabulary_edits.is_empty());
    }

    #[test]
    fn round_three_vocabulary_and_guard_regressions_v301_v313() {
        let entries = [qwen_entry()];
        let ai_scope = Some("AI model discussion");

        // V301: a plain `model` descriptor attached after the alias is
        // occurrence evidence, with or without a matching active scope.
        for scope in [None, ai_scope] {
            let result = with_vocabulary(
                "We tested the Quinn model on technical dictation.",
                &entries,
                scope,
            );
            assert_eq!(
                result.text,
                "We tested the Qwen model on technical dictation."
            );
            assert_eq!(result.vocabulary_edits.len(), 1);
        }

        // V302/V303: a matching active scope is retrieval context only; it
        // never settles the referent of an ambiguous name.
        for scope in [None, ai_scope] {
            let result = with_vocabulary("Quinn will send the report tomorrow.", &entries, scope);
            assert_eq!(result.text, "Quinn will send the report tomorrow.");
            assert!(result.vocabulary_edits.is_empty());
        }
        for source in ["Quinn has emailed the report.", "Quinn emailed the report."] {
            let result = with_vocabulary(source, &entries, ai_scope);
            assert_eq!(result.text, source);
            assert!(result.vocabulary_edits.is_empty());
        }

        // V304: an AI-model noun phrase not attached to the alias is not
        // evidence for this occurrence.
        let result = with_vocabulary("Ask Quinn to review the AI model.", &entries, ai_scope);
        assert_eq!(result.text, "Ask Quinn to review the AI model.");
        assert!(result.vocabulary_edits.is_empty());

        // V305/V306: negation and assessment hedges keep their exact text and
        // do not block the confirmed identifier span.
        assert_eq!(
            with_vocabulary(
                "Do not use the Quinn model for speech recognition.",
                &entries,
                None
            )
            .text,
            "Do not use the Qwen model for speech recognition."
        );
        assert_eq!(
            with_vocabulary(
                "I think the Quinn language model is too slow.",
                &entries,
                ai_scope
            )
            .text,
            "I think the Qwen language model is too slow."
        );

        // V307: uncertainty about the identifier itself still guards.
        let result = with_vocabulary("The name might be Quinn or Qwen.", &entries, ai_scope);
        assert_eq!(result.text, "The name might be Quinn or Qwen.");
        assert!(result.vocabulary_edits.is_empty());

        // V308-V310: literal words veto only their own segment or the literal
        // they are attached to.
        assert_eq!(
            run("Update the code, then email sam at comet dot io.").text,
            "Update the code, then email sam@comet.io."
        );
        unchanged("Say the words email sam at comet dot io literally.");
        assert_eq!(
            run("Keep `user_id` unchanged, then email sam at comet dot io.").text,
            "Keep `user_id` unchanged, then email sam@comet.io."
        );

        // V311: never invent a domain suffix.
        unchanged("Email sam at comet.");

        // V312: a protected quote is opaque; the text stage never strips
        // protocol markers.
        unchanged("Say \"Language English<asr_text>\" literally.");

        // V313: UTF-8 offsets, original hash, and the untouched tail.
        let source = "Café ready. Email sam at comet dot io. Keep naïve exactly.";
        let result = run(source);
        assert_eq!(
            result.text,
            "Café ready. Email sam@comet.io. Keep naïve exactly."
        );
        assert_eq!(result.edits.len(), 1);
        let edit = &result.edits[0];
        assert_eq!(edit.start_byte, source.find("sam at").unwrap());
        assert_eq!(
            &source[edit.start_byte..edit.end_byte],
            "sam at comet dot io"
        );
        assert_eq!(result.source_revision, source_revision(source));
        assert!(result.text.ends_with("Keep naïve exactly."));
        validate_accounting(source, edit).unwrap();

        // Continued descriptor-before-alias phrases conservatively abstain in v4;
        // descriptor-after-alias and alias-final evidence remain available.
        assert_eq!(
            with_vocabulary("the model Quinn is slow", &entries, None).text,
            "the model Quinn is slow"
        );
        let result = with_vocabulary("Benchmark the model Quinn or Qwen.", &entries, ai_scope);
        assert_eq!(result.text, "Benchmark the model Quinn or Qwen.");
        assert!(result.vocabulary_edits.is_empty());
    }

    #[test]
    fn round_four_relative_clause_person_is_never_a_model_alias() {
        let entries = [qwen_entry()];
        // R410/R411: a following ordinary word can make the alias the subject
        // of a reduced relative clause; the referent is a person or ambiguous.
        // Only a sentence end closes this optional prefix route.
        for scope in [None, Some("AI model discussion")] {
            for source in [
                "The model Quinn trained is smaller.",
                "The model Quinn reviewed failed.",
                "The model Quinn built outperforms it.",
                "The model Quinn, trained by the team, is smaller.",
                "The model Quinn uses grouped attention.",
                "The model Quinn and Qwen are paired.",
                "The model Quinn has trained is smaller.",
                "The model Quinn is training failed.",
                "The model Quinn will review failed.",
            ] {
                let result = with_vocabulary(source, &entries, scope);
                assert_eq!(result.text, source, "{source} (scope {scope:?})");
                assert!(result.vocabulary_edits.is_empty(), "{source}");
            }
        }
        // Continued prefix phrases abstain; alias-final evidence still repairs.
        for (source, expected) in [
            ("The model Quinn is smaller.", "The model Quinn is smaller."),
            ("The model Quinn was faster.", "The model Quinn was faster."),
            (
                "The model Quinn has shipped.",
                "The model Quinn has shipped.",
            ),
            ("the model Quinn is slow", "the model Quinn is slow"),
            (
                "We benchmarked the model Quinn.",
                "We benchmarked the model Qwen.",
            ),
            (
                "The model Quinn for dictation works.",
                "The model Quinn for dictation works.",
            ),
        ] {
            for scope in [None, Some("AI model discussion")] {
                assert_eq!(
                    with_vocabulary(source, &entries, scope).text,
                    expected,
                    "{source} (scope {scope:?})"
                );
            }
        }
        // The descriptor-suffix route and its controls are unchanged.
        assert_eq!(
            with_vocabulary("Use the Quinn model for dictation.", &entries, None).text,
            "Use the Qwen model for dictation."
        );
        assert_eq!(
            with_vocabulary(
                "Quinn will send the report tomorrow.",
                &entries,
                Some("AI model discussion")
            )
            .text,
            "Quinn will send the report tomorrow."
        );
        assert_eq!(
            with_vocabulary("Do not use the Quinn model for this task.", &entries, None).text,
            "Do not use the Qwen model for this task."
        );
        assert_eq!(
            with_vocabulary(
                "I think the Quinn language model is suitable.",
                &entries,
                Some("AI model discussion")
            )
            .text,
            "I think the Qwen language model is suitable."
        );
    }

    #[test]
    fn round_four_continuation_words_close_complete_candidates() {
        // T301/R413: a supported continuation closes a finished address without
        // consuming prose or losing the negation.
        for (source, expected) in [
            (
                "Don't send it to ops at northwind dot io yet.",
                "Don't send it to ops@northwind.io yet.",
            ),
            (
                "Send it to ops at northwind dot io now.",
                "Send it to ops@northwind.io now.",
            ),
            (
                "Send it to ops at northwind dot io again.",
                "Send it to ops@northwind.io again.",
            ),
            (
                "Email ops at northwind dot io today, then wait.",
                "Email ops@northwind.io today, then wait.",
            ),
        ] {
            assert_eq!(run(source).text, expected, "{source}");
        }
        // Incomplete or unsupported continuations still reject the whole
        // candidate; no suffix is invented and no prose is consumed.
        for source in [
            "Don't send it to ops at northwind yet.",
            "Don't send it to ops at northwind dot yet.",
            "Send it to ops at northwind dot io forevermore.",
            "Send it to ops at northwind dot io somehow.",
            "Email sam at comet yet.",
        ] {
            unchanged(source);
        }
    }

    #[test]
    fn round_four_literal_guard_follows_its_own_referent() {
        // T318/R414: "Keep naïve spelled that way" names its own referent, so
        // it does not protect the earlier address candidate.
        assert_eq!(
            run("Café orders: email cafe at bistro dot fr. Keep naïve spelled that way.").text,
            "Café orders: email cafe@bistro.fr. Keep naïve spelled that way."
        );
        assert_eq!(
            run("email cafe at bistro dot fr, keep naïve spelled that way.").text,
            "email cafe@bistro.fr, keep naïve spelled that way."
        );
        assert_eq!(
            run("Keep naïve spelled that way. Email cafe at bistro dot fr.").text,
            "Keep naïve spelled that way. Email cafe@bistro.fr."
        );
        // R415: genuine anaphoric references to the address keep protecting it.
        for source in [
            "Email cafe at bistro dot fr. Keep that address exactly as spoken.",
            "Email cafe at bistro dot fr. Keep that address spelled exactly as spoken.",
            "Email cafe at bistro dot fr. Keep the addresses spelled as spoken.",
            "Email cafe at bistro dot fr. Keep it exactly as spoken.",
            "Email cafe at bistro dot fr. Keep that email exactly as spoken.",
            "Email cafe at bistro dot fr. Keep naive spelled that way and that address literal.",
            "Email cafe at bistro dot fr. Preserve those words verbatim.",
            "email cafe at bistro dot fr, but keep that address exactly as spoken.",
        ] {
            unchanged(source);
        }
    }

    #[test]
    fn round_four_incomplete_candidate_spans_are_reported_not_normalized() {
        // V311/T303/R416: cued address-like runs the grammar cannot complete
        // are reported for byte-exact protection and never normalized.
        for (source, span) in [
            ("Email sam at comet.", "sam at comet"),
            ("Email lee at maple.", "lee at maple"),
            ("Email sam at comet dot.", "sam at comet dot"),
            (
                "Send it to iris at beacon dot net dot.",
                "iris at beacon dot net dot",
            ),
            (
                "Contact alex at example dot unknownsuffix.",
                "alex at example dot unknownsuffix",
            ),
        ] {
            let spans = incomplete_candidate_spans(source, &|| false).unwrap();
            assert_eq!(spans.len(), 1, "{source}");
            assert_eq!(&source[spans[0].0..spans[0].1], span, "{source}");
            unchanged(source);
        }
        // A guarded incomplete run is still reported: protection is stricter
        // preservation, never normalization.
        let guarded = "Say the words Email sam at comet literally.";
        let spans = incomplete_candidate_spans(guarded, &|| false).unwrap();
        assert_eq!(spans.len(), 1);
        assert_eq!(&guarded[spans[0].0..spans[0].1], "sam at comet");
        unchanged(guarded);
        // Complete candidates, uncued runs and non-address prose report none.
        for source in [
            "Email sam at comet dot io.",
            "Meet sam at noon.",
            "Email the report.",
        ] {
            assert!(
                incomplete_candidate_spans(source, &|| false)
                    .unwrap()
                    .is_empty(),
                "{source}"
            );
        }
    }
}
