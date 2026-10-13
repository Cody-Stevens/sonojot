//! The embedded lexicon supplies aliases and tiers; versions are parsed from
//! the utterance, never selected from the lexicon's informative `known` list.
use super::{numbers, tokens};
use serde::Deserialize;
use std::sync::OnceLock;

pub(super) const GPT_TIER_JOIN: &str = " ";
#[derive(Deserialize)]
struct Lexicon {
    families: Vec<Family>,
    products: Vec<Name>,
}
#[derive(Deserialize)]
struct Family {
    id: String,
    canonical: String,
    spoken: Vec<String>,
    misheard: Vec<String>,
    ambiguous: bool,
    version_join: String,
    version_format: String,
    tiers: Vec<Name>,
    #[serde(default)]
    tier_alone_anchor: bool,
    #[serde(skip)]
    aliases: Vec<Vec<String>>,
}
#[derive(Deserialize)]
struct Name {
    canonical: String,
    spoken: Vec<String>,
    #[serde(default)]
    misheard: Vec<String>,
    #[serde(default)]
    ambiguous: bool,
    join: Option<String>,
    #[serde(skip)]
    aliases: Vec<Vec<String>>,
}
fn lexicon() -> &'static Lexicon {
    static LEXICON: OnceLock<Lexicon> = OnceLock::new();
    LEXICON.get_or_init(|| {
        let mut lex: Lexicon =
            serde_json::from_str(include_str!("models.json")).expect("embedded model lexicon");
        for family in &mut lex.families {
            family.aliases = family
                .spoken
                .iter()
                .chain(&family.misheard)
                .chain(std::iter::once(&family.canonical))
                .map(|a| words(a))
                .collect();
            for tier in &mut family.tiers {
                tier.init();
            }
        }
        for product in &mut lex.products {
            product.init();
        }
        lex
    })
}
impl Name {
    fn init(&mut self) {
        self.aliases = self
            .spoken
            .iter()
            .chain(&self.misheard)
            .chain(std::iter::once(&self.canonical))
            .map(|a| words(a))
            .collect();
    }
}

fn words(s: &str) -> Vec<String> {
    let s = s.replace(['-', '_'], " ");
    // Canonical spellings must parse back into the same components: Qwen3,
    // 27B, A3B, GLM-4.6V and Whisper's v3.
    let s = re!(r"([a-zA-Z])(\d)").replace_all(&s, "$1 $2");
    let s = re!(r"(\d)([a-zA-Z])").replace_all(&s, "$1 $2");
    tokens(&s)
}
fn alias_len(input: &[String], aliases: &[Vec<String>]) -> Option<usize> {
    aliases
        .iter()
        .filter_map(|a| (!a.is_empty() && input.starts_with(a)).then_some(a.len()))
        .max()
}
fn name_len(input: &[String], name: &Name) -> Option<usize> {
    alias_len(input, &name.aliases)
}
fn family_len(input: &[String], family: &Family) -> Option<usize> {
    alias_len(input, &family.aliases)
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Part {
    Version(String),
    Tier(usize),
    Size(String, String), // prefix A/E, exact number
}
#[derive(Clone, Debug, Eq, PartialEq)]
struct Parsed {
    family: usize,
    short: bool,
    parts: Vec<Part>,
    cue: String,
    effort: String,
}
fn version_number(t: &[String], paired: bool) -> Option<String> {
    if t.is_empty() || t.len() > 12 {
        return None;
    }
    if t.len() == 1 && re!(r"^\d{1,4}(?:\.\d{1,4})?$").is_match(&t[0]) {
        return Some(t[0].clone());
    }
    if t.iter().any(|w| {
        matches!(
            w.as_str(),
            "a" | "minus" | "negative" | "thousand" | "million" | "billion" | "trillion"
        )
    }) {
        return None;
    }
    if t.iter().any(|w| w == "point") {
        if t.iter().filter(|w| w.as_str() == "point").count() != 1 {
            return None;
        }
        let mut result = Vec::new();
        for (i, group) in t.split(|w| w == "point").enumerate() {
            if group.is_empty() {
                return None;
            }
            let r: Vec<_> = group.iter().map(String::as_str).collect();
            if i > 0
                && r.iter().all(|w| {
                    numbers::value(w).is_some_and(|n| n < 10)
                        || matches!(*w, "oh" | "o")
                        || re!(r"^\d$").is_match(w)
                })
            {
                let digits: String = r
                    .iter()
                    .map(|w| {
                        numbers::value(w).map(|n| n.to_string()).unwrap_or_else(|| {
                            if matches!(*w, "oh" | "o") {
                                "0".into()
                            } else {
                                w.to_string()
                            }
                        })
                    })
                    .collect();
                if digits.len() > 4 {
                    return None;
                }
                result.push(digits);
            } else {
                let n = numbers::integer(&r, false)?;
                if !(0..=9999).contains(&n) {
                    return None;
                }
                result.push(n.to_string());
            }
        }
        return Some(result.join("."));
    }
    let refs: Vec<_> = t.iter().map(String::as_str).collect();
    if let Some(n) = numbers::integer(&refs, false).filter(|n| (0..=9999).contains(n)) {
        return Some(n.to_string());
    }
    if paired && t.len() == 2 {
        let a = numbers::integer(&[&t[0]], false)?;
        let b = numbers::integer(&[&t[1]], false)?;
        if (0..=9).contains(&a) && (0..=9).contains(&b) {
            return Some(format!("{a}.{b}"));
        }
    }
    None
}
fn size_number(t: &[String]) -> Option<String> {
    version_number(t, false)
        .or_else(|| {
            if t.len() < 2 {
                return None;
            }
            let a = numbers::integer(&[&t[0]], false)?;
            let tail: Vec<_> = t[1..].iter().map(String::as_str).collect();
            let b = numbers::integer(&tail, false)?;
            ((1..=9).contains(&a) && (10..=99).contains(&b)).then(|| (a * 100 + b).to_string())
        })
        .filter(|s| s.matches('.').count() <= 1 && s.chars().any(|c| matches!(c, '1'..='9')))
}
fn accepts_sizes(f: &Family) -> bool {
    f.tiers.iter().any(|t| t.canonical.contains("{n}"))
        || matches!(f.id.as_str(), "gpt-oss" | "parakeet")
}
fn parse_parts(f: &Family, t: &[String], parts: &mut Vec<Part>) -> bool {
    if t.is_empty() {
        return true;
    }
    if parts.len() >= 8 {
        return false;
    }
    // Longest tier first (Flash-Lite before Flash, reasoning-plus before reasoning).
    let mut tiers: Vec<_> = f
        .tiers
        .iter()
        .enumerate()
        .filter(|(_, n)| !n.canonical.contains('{'))
        .filter_map(|(i, n)| name_len(t, n).map(|len| (len, i)))
        .collect();
    tiers.sort_by(|a, b| b.cmp(a));
    for (len, i) in tiers {
        if parts.contains(&Part::Tier(i)) {
            continue;
        }
        if f.id == "gpt"
            && t.first().is_some_and(|w| w == "extra")
            && t.get(1).is_some_and(|w| w == "high")
        {
            continue;
        }
        // Whisper's API id is a version, not a tier named "1".
        if f.id == "whisper" && f.tiers[i].canonical == "1" {
            continue;
        }
        if !matches!(f.id.as_str(), "qwen" | "lfm" | "whisper" | "glm")
            && parts.iter().any(|p| matches!(p, Part::Tier(_)))
        {
            continue;
        }
        if !parts.iter().any(|p| matches!(p, Part::Version(_)))
            && !matches!(
                f.id.as_str(),
                "claude"
                    | "grok"
                    | "muse"
                    | "mistral"
                    | "devstral"
                    | "magistral"
                    | "voxtral"
                    | "mercury"
                    | "granite"
                    | "whisper"
                    | "parakeet"
            )
            && t.len() > len
        {
            continue;
        }
        parts.push(Part::Tier(i));
        if parse_parts(f, &t[len..], parts) {
            return true;
        }
        parts.pop();
    }
    if accepts_sizes(f)
        && (parts.iter().any(|p| matches!(p, Part::Version(_)))
            || matches!(f.id.as_str(), "gpt-oss" | "parakeet"))
    {
        let (prefix, start) = if matches!(t.first().map(String::as_str), Some("a" | "e")) {
            (t[0].to_uppercase(), 1)
        } else {
            (String::new(), 0)
        };
        let allowed = prefix.is_empty()
            || f.tiers
                .iter()
                .any(|n| n.canonical.starts_with(&format!("{prefix}{{n}}")));
        if allowed
            && !parts
                .iter()
                .any(|p| matches!(p, Part::Size(pre, _) if pre == &prefix))
        {
            for end in (start + 1..t.len().min(start + 13)).rev() {
                if t[end] != "b" {
                    continue;
                }
                if let Some(n) = size_number(&t[start..end]) {
                    parts.push(Part::Size(prefix.clone(), n));
                    if parse_parts(f, &t[end + 1..], parts) {
                        return true;
                    }
                    parts.pop();
                }
            }
        }
    }
    if !parts.iter().any(|p| matches!(p, Part::Version(_)))
        && !f.version_format.is_empty()
        && f.id != "gpt-oss"
    {
        let mut start = 0;
        let mut prefix = "";
        match f.id.as_str() {
            "kimi" => {
                if !matches!(t[0].as_str(), "k" | "kay") {
                    return false;
                }
                start = 1;
                prefix = "K";
            }
            "deepseek" => {
                match t[0].as_str() {
                    "v" | "vee" => prefix = "V",
                    "r" | "are" => prefix = "R",
                    _ => return false,
                }
                start = 1;
            }
            "minimax" => {
                if !matches!(t[0].as_str(), "m" | "em") {
                    return false;
                }
                start = 1;
                prefix = "M";
            }
            "whisper" | "parakeet" => {
                if t[0] == "v" {
                    start = 1;
                    prefix = "v";
                } else if !(f.id == "whisper"
                    && t.len() == 1
                    && matches!(t[0].as_str(), "one" | "1"))
                {
                    return false;
                }
            }
            _ => {}
        }
        let paired = matches!(f.id.as_str(), "gpt" | "claude" | "gemini" | "grok" | "glm");
        for end in (start + 1..=t.len().min(start + 12)).rev() {
            if let Some(v) = version_number(&t[start..end], paired) {
                if v.contains('.') && !f.version_format.contains('.') {
                    continue;
                }
                let mut consumed = end;
                let n = if f.id == "gemma" && t.get(end).is_some_and(|w| w == "n") {
                    consumed += 1;
                    "n"
                } else {
                    ""
                };
                parts.push(Part::Version(format!("{prefix}{v}{n}")));
                if parse_parts(f, &t[consumed..], parts) {
                    return true;
                }
                parts.pop();
            }
        }
    }
    false
}

fn effort(t: &[String]) -> bool {
    matches!(
        t.join(" ").as_str(),
        "low"
            | "medium"
            | "high"
            | "x high"
            | "ex high"
            | "xhigh"
            | "extra high"
            | "max"
            | "maximum"
            | "ultra"
            | "minimal"
            | "none"
            | "no thinking"
    )
}
fn parse(input: &str) -> Option<Parsed> {
    let all = words(input);
    if all.is_empty() || all.len() > 36 {
        return None;
    }
    let lex = lexicon();
    // MODEL can include an explicit company/model cue, with no implicit words.
    let (all, company) = if all.starts_with(&["typesafe".into()]) {
        (&all[1..], "TypeSafe ")
    } else if all.starts_with(&["type".into(), "safe".into()]) {
        (&all[2..], "TypeSafe ")
    } else {
        (all.as_slice(), "")
    };
    for (index, f) in lex.families.iter().enumerate() {
        if !company.is_empty() && f.id != "jev" {
            continue;
        }
        let mut anchors = Vec::new();
        if let Some(n) = family_len(all, f) {
            anchors.push((n, false, None));
        }
        if f.tier_alone_anchor {
            for (i, tier) in f.tiers.iter().enumerate() {
                if let Some(n) = name_len(all, tier) {
                    anchors.push((n, true, Some(i)));
                }
            }
        }
        for (len, short, initial) in anchors {
            let rest = &all[len..];
            // Effort is retained verbatim, except the explicit spoken letter
            // form x high. It is never interpreted as a GPT tier.
            let mut endings = vec![(rest.len(), String::new(), String::new())];
            if rest.last().is_some_and(|w| w == "model") {
                let cue = input.split_whitespace().last()?;
                endings.push((rest.len() - 1, format!(" {cue}"), String::new()));
            }
            for e in 0..rest.len() {
                let cue = matches!(rest[e].as_str(), "on" | "at" | "with" | "using");
                let offset = usize::from(cue);
                if effort(&rest[e + offset..]) {
                    // Preserve source casing for effort words using the input's
                    // final whitespace tokens, which have no model separators.
                    let count = rest.len() - e;
                    let source: Vec<_> = input.split_whitespace().collect();
                    let tail = source[source.len().saturating_sub(count)..].join(" ");
                    let tail = re!(r"(?i)\bx\s+high\b")
                        .replace_all(&tail, "xhigh")
                        .into_owned();
                    endings.push((e, String::new(), format!(" {tail}")));
                }
            }
            for (end, cue, effort) in endings {
                let mut parts: Vec<_> = initial.map(Part::Tier).into_iter().collect();
                if !parse_parts(f, &rest[..end], &mut parts) {
                    continue;
                }
                let version = parts.iter().any(|p| matches!(p, Part::Version(_)));
                let tier = parts
                    .iter()
                    .any(|p| matches!(p, Part::Tier(_) | Part::Size(_, _)));
                if f.ambiguous && !version && !tier {
                    continue;
                }
                // A bare homophone is prose, even when the canonical family
                // is unambiguous. Effort and "model" do not supply a version.
                if parts.is_empty() && !f.spoken.iter().chain(std::iter::once(&f.canonical))
                    .any(|alias| words(alias) == all[..len]) {
                    continue;
                }
                if matches!(f.id.as_str(), "kimi" | "deepseek" | "minimax") && !version {
                    continue;
                }
                if f.id == "gpt-oss" && !parts.iter().any(|part| matches!(part, Part::Size(_, _))) {
                    continue;
                }
                if short && !version {
                    continue;
                }
                // A size cannot stand in for the family version (except OSS).
                if parts.iter().any(|p| matches!(p, Part::Size(_, _)))
                    && !version
                    && f.id != "gpt-oss"
                    && f.id != "parakeet"
                {
                    continue;
                }
                return Some(Parsed {
                    family: index,
                    short,
                    parts,
                    cue: format!("{company}{cue}"),
                    effort,
                });
            }
        }
    }
    None
}

fn display(p: &Parsed) -> String {
    let f = &lexicon().families[p.family];
    let mut out = if p.short {
        String::new()
    } else {
        f.canonical.clone()
    };
    let granular = f.id == "granite"
        && p.parts.iter().any(|p| {
            matches!(p, Part::Size(_, _))
                || matches!(p, Part::Tier(i) if f.tiers[*i].canonical.starts_with("H-"))
        });
    let parakeet_full = f.id == "parakeet"
        && p.parts
            .iter()
            .any(|p| matches!(p, Part::Size(_, _) | Part::Version(_)));
    for (i, part) in p.parts.iter().enumerate() {
        let (join, text) = match part {
            Part::Version(v) => {
                let join = if f.id == "whisper" && v.starts_with('v') {
                    "-"
                } else if granular || parakeet_full {
                    "-"
                } else if i > 0 {
                    " "
                } else {
                    f.version_join.as_str()
                };
                if f.id == "whisper" && i == 0 && !v.starts_with('v') {
                    out = "whisper".into();
                }
                (join, v.clone())
            }
            Part::Tier(i) => {
                let tier = &f.tiers[*i];
                let join = if f.id == "whisper" && out == "Whisper" {
                    " "
                } else if granular || parakeet_full {
                    "-"
                } else if f.id == "whisper" && out != "Whisper" {
                    "-"
                } else {
                    tier.join
                        .as_deref()
                        .unwrap_or(if f.id == "gpt" { GPT_TIER_JOIN } else { " " })
                };
                (join, tier.canonical.clone())
            }
            Part::Size(pre, n) => {
                let join = if matches!(f.id.as_str(), "qwen" | "lfm" | "gpt-oss")
                    || granular
                    || parakeet_full
                {
                    "-"
                } else {
                    " "
                };
                (
                    join,
                    format!("{pre}{n}{}", if f.id == "gpt-oss" { "b" } else { "B" }),
                )
            }
        };
        if !out.is_empty() {
            out.push_str(join);
        }
        out.push_str(&text);
    }
    if p.cue.starts_with("TypeSafe ") {
        out = format!("TypeSafe {out}");
    }
    if let Some(cue) = p.cue.split_whitespace().last().filter(|cue| cue.eq_ignore_ascii_case("model")) {
        out.push(' ');
        out.push_str(cue);
    }
    out.push_str(&p.effort);
    out
}

fn product(input: &str, context: &str) -> Option<String> {
    let w = words(input);
    for name in &lexicon().products {
        if name.ambiguous {
            continue;
        }
        // The lexicon explicitly excludes unqualified Google Cloud Code.
        if name.canonical == "Claude Code"
            && w.first().is_some_and(|s| s == "cloud")
            && !words(context)
                .iter()
                .any(|s| matches!(s.as_str(), "agent" | "cli" | "coding"))
        {
            continue;
        }
        if name_len(&w, name) == Some(w.len()) {
            return Some(name.canonical.clone());
        }
    }
    None
}
pub(super) fn render(input: &str, before: &str, after: &str) -> Option<String> {
    // Exact products win over family homophones, e.g. ChatGPT.
    product(input, &format!("{before} {after}")).or_else(|| parse(input).map(|p| display(&p)))
}
pub(super) fn guard(original: &str, candidate: &str, before: &str, after: &str) -> bool {
    let context = format!("{before} {after}");
    if let (Some(a), Some(b)) = (product(original, &context), product(candidate, &context)) {
        return a == b;
    }
    match (parse(original), parse(candidate)) {
        (Some(mut a), Some(mut b)) => {
            // Effort casing is intentionally untouched, but the comparison is
            // case-insensitive like the numeric guard.
            a.effort = a.effort.to_lowercase();
            b.effort = b.effort.to_lowercase();
            a == b
        }
        _ => false,
    }
}
pub(super) fn anchor(input: &str) -> bool {
    let w = words(input);
    lexicon().families.iter().any(|f| {
        family_len(&w, f).is_some()
            || f.tier_alone_anchor && f.tiers.iter().any(|t| name_len(&w, t).is_some())
    }) || lexicon().products.iter().any(|p| name_len(&w, p).is_some())
        || w.first().is_some_and(|s| s == "typesafe")
}
