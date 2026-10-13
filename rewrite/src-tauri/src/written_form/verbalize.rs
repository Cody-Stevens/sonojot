use super::numbers::{self, AMP_NAMES, ORDINALS, SCALES, SMALL, TENS};

const LIMIT: usize = 512;
pub(super) fn integer_speech(n: i128) -> String {
    if n < 0 {
        return format!("minus {}", integer_speech(n.saturating_abs()));
    }
    if n < 20 {
        return SMALL[n as usize].into();
    }
    if n < 100 {
        return format!(
            "{}{}",
            TENS[n as usize / 10],
            if n % 10 > 0 {
                format!(" {}", SMALL[n as usize % 10])
            } else {
                String::new()
            }
        );
    }
    if n < 1000 {
        return format!(
            "{} hundred{}",
            SMALL[n as usize / 100],
            if n % 100 > 0 {
                format!(" {}", integer_speech(n % 100))
            } else {
                String::new()
            }
        );
    }
    for (word, scale) in SCALES.iter().rev() {
        if n >= *scale {
            return format!(
                "{} {word}{}",
                integer_speech(n / scale),
                if n % scale > 0 {
                    format!(" {}", integer_speech(n % scale))
                } else {
                    String::new()
                }
            );
        }
    }
    unreachable!()
}
fn ordinal_speech(n: i128) -> String {
    let s = integer_speech(n);
    let (prefix, last) = s.rsplit_once(' ').map_or(("", s.as_str()), |(a, b)| (a, b));
    let ord = ORDINALS
        .iter()
        .find(|o| numbers::ordinal_word(o) == Some(last))
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{last}th"));
    if prefix.is_empty() {
        ord
    } else {
        format!("{prefix} {ord}")
    }
}
fn dedup(v: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    v.into_iter().filter(|s| seen.insert(s.clone())).collect()
}
pub(super) fn numeric_speech(value: &str) -> Vec<String> {
    let value = value.replace(',', "");
    let negative = value.starts_with('-');
    let unsigned = value.trim_start_matches('-');
    let (whole, fraction) = unsigned
        .split_once('.')
        .map_or((unsigned, None), |(w, f)| (w, Some(f)));
    let Ok(n) = whole.parse::<i128>() else {
        return vec![];
    };
    if n < 0 || fraction.is_some_and(|f| f.is_empty() || !f.bytes().all(|b| b.is_ascii_digit())) {
        return vec![];
    }
    let spoken = integer_speech(n);
    let mut options = vec![spoken.clone()];
    if let Some(f) = fraction {
        let tail = f
            .bytes()
            .map(|c| SMALL[(c - b'0') as usize])
            .collect::<Vec<_>>()
            .join(" ");
        options = vec![format!("{spoken} point {tail}")];
        if n == 0 {
            options.push(format!("point {tail}"));
        }
        if f.len() == 2 && !f.starts_with('0') {
            let rest = integer_speech(f.parse().unwrap_or(0));
            options.push(format!("{spoken} point {rest}"));
            if n == 0 {
                options.push(format!("point {rest}"));
            }
        }
        if tail.contains("zero") {
            options.extend(options.clone().iter().map(|s| s.replace("zero", "oh")));
            options.extend(options.clone().iter().map(|s| s.replace("zero", "o")));
        }
    } else {
        // The reference also verbalizes 12,500 as "twelve point five
        // thousand". Keep this exact: coefficient/scale avoids float rounding.
        if (1000..1_000_000).contains(&n) && n % 1000 != 0 {
            let coefficient = numbers::Number {
                coefficient: n,
                places: 3,
            }
            .trimmed_digits();
            options.extend(
                numeric_speech(&coefficient)
                    .into_iter()
                    .map(|s| format!("{s} thousand")),
            );
        }
        if (1000..=2999).contains(&n) {
            let (century, rem) = (integer_speech(n / 100), n % 100);
            if rem > 0 {
                options.push(format!(
                    "{century} {}{}",
                    if rem < 10 { "oh " } else { "" },
                    integer_speech(rem)
                ));
                if rem < 10 {
                    options.push(format!("{century} o {}", integer_speech(rem)));
                }
            } else {
                options.push(format!("{century} hundred"));
            }
        }
    }
    options.extend(options.clone().iter().map(|o| article(o)));
    options.extend(options.clone().iter().map(|o| {
        re!(r"hundred (\w)")
            .replace_all(o, "hundred and $1")
            .into_owned()
    }));
    if negative {
        options = options
            .into_iter()
            .flat_map(|s| [format!("minus {s}"), format!("negative {s}")])
            .collect();
    }
    options.push(value);
    dedup(options)
}
fn one(s: &str) -> Vec<String> {
    vec![s.into()]
}
fn combine(parts: &[Vec<String>]) -> Vec<String> {
    let mut output = vec![String::new()];
    for part in parts {
        output = output
            .iter()
            .flat_map(|a| {
                part.iter().map(move |b| {
                    if a.is_empty() {
                        b.clone()
                    } else if b.is_empty() {
                        a.clone()
                    } else {
                        format!("{a} {b}")
                    }
                })
            })
            .take(LIMIT)
            .collect();
    }
    dedup(output)
}
fn article(s: &str) -> String {
    re!(r"^one (hundred\b|thousand\b|million\b|billion\b|trillion\b)")
        .replace(s, "a $1")
        .into_owned()
}

/// Complete spoken alternatives, bounded to 512 per Cartesian expansion.
/// Unknown strings have only an identity alternative; parsing is still required.
pub fn verbalize(written: &str) -> Vec<String> {
    if written.len() > 2048 {
        return vec![];
    }
    let mut phrases = verbalize_inner(written.trim());
    phrases.extend(phrases.clone().iter().map(|s| article(s)));
    dedup(phrases)
}
fn verbalize_inner(v: &str) -> Vec<String> {
    if v.is_empty() {
        return vec![];
    }
    if AMP_NAMES.contains(&v) {
        return vec![v
            .chars()
            .map(|c| {
                if c == '&' {
                    "and".into()
                } else {
                    c.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(" ")];
    }
    if let Some(c) =
        re!(r"^([$€])(-?(?:\d{1,3}(?:,\d{3})+|\d+)(?:\.\d+)?)(?: (million|billion|trillion))?$")
            .captures(v)
    {
        let currency = if &c[1] == "$" {
            vec!["dollar".into(), "dollars".into()]
        } else {
            vec!["euro".into(), "euros".into()]
        };
        let mut result = combine(&[
            numeric_speech(&c[2]),
            one(c.get(3).map_or("", |m| m.as_str())),
            currency.clone(),
        ]);
        if c.get(3).is_none() {
            if let Some((whole, frac)) = c[2].split_once('.') {
                if frac.len() <= 2 {
                    let cents = format!("{frac:0<2}")
                        .parse::<i128>()
                        .unwrap_or(0)
                        .to_string();
                    result.extend(combine(&[
                        numeric_speech(whole),
                        currency,
                        vec!["and".into(), "".into()],
                        numeric_speech(&cents),
                        vec!["cent".into(), "cents".into()],
                    ]));
                }
            }
        }
        return dedup(result);
    }
    if let Some(c) =
        re!(r"^(-?(?:\d{1,3}(?:,\d{3})+|\d+)(?:\.\d+)?)(?: (million|billion|trillion))?%$")
            .captures(v)
    {
        return combine(&[
            numeric_speech(&c[1]),
            one(c.get(2).map_or("", |m| m.as_str())),
            one("percent"),
        ]);
    }
    if let Some(c) = re!(r"^(-?(?:\d{1,3}(?:,\d{3})+|\d+)(?:\.\d+)?)([KMx])$").captures(v) {
        return combine(&[numeric_speech(&c[1]), one(&c[2])]);
    }
    if let Some(c) = re!(r"^(January|February|March|April|May|June|July|August|September|October|November|December) (\d{1,2})(?:, (\d{4}))?$").captures(v) {
        let day = c[2].parse().unwrap_or(0);
        return combine(&[one(&c[1]), vec![ordinal_speech(day), integer_speech(day), c[2].into()], c.get(3).map_or_else(|| one(""), |y| numeric_speech(y.as_str()))]);
    }
    if let Some(c) = re!(r"^(the )?(\d{1,2})(?:st|nd|rd|th) of (\w+)(?:, (\d{4}))?$").captures(v) {
        if numbers::month(&c[3]).is_some() {
            return combine(&[
                one(c.get(1).map_or("", |m| m.as_str().trim())),
                one(&ordinal_speech(c[2].parse().unwrap_or(0))),
                one("of"),
                one(&c[3]),
                c.get(4)
                    .map_or_else(|| one(""), |y| numeric_speech(y.as_str())),
            ]);
        }
    }
    if let Some(c) = re!(r"^(at )?(\d{1,2})(?::(\d{2}))?(?: (AM|PM))?$").captures(v) {
        if c.get(3).is_some() || c.get(4).is_some() {
            let mut minutes = one("");
            if let Some(m) = c.get(3) {
                let n = m.as_str().parse().unwrap_or(0);
                minutes = one(&integer_speech(n));
                if n < 10 {
                    minutes.extend([
                        format!("oh {}", integer_speech(n)),
                        format!("o {}", integer_speech(n)),
                    ]);
                }
                minutes.push(n.to_string());
            }
            return combine(&[
                one(c.get(1).map_or("", |m| m.as_str().trim())),
                numeric_speech(&c[2]),
                minutes,
                one(c.get(4).map_or("", |m| m.as_str())),
            ]);
        }
    }
    if let Some(c) = re!(r"^([Vv]|version |Python |\.NET )(\d+(?:\.\d+)*)$").captures(v) {
        let Some(parts) = c[2]
            .split('.')
            .map(|p| p.parse::<i128>().ok().map(integer_speech))
            .collect::<Option<Vec<_>>>()
        else {
            return one(v);
        };
        let mut speech = vec![parts.join(" point ")];
        let mut choices = Vec::new();
        for p in c[2].split('.') {
            if !choices.is_empty() {
                choices.push(one("point"));
            }
            choices.push(vec![
                integer_speech(p.parse().unwrap_or(0)),
                p.bytes()
                    .map(|b| SMALL[(b - b'0') as usize])
                    .collect::<Vec<_>>()
                    .join(" "),
                p.to_owned(),
            ]);
        }
        speech.extend(combine(&choices));
        if matches!(c[1].trim(), "Python" | ".NET") && parts.len() > 1 {
            speech.push(parts.join(" "));
        }
        speech.push(c[2].into());
        return combine(&[one(c[1].trim()), speech]);
    }
    if let Some(c) = re!(r"^(\d+)(st|nd|rd|th)$").captures(v) {
        if let Ok(n) = c[1].parse::<i128>() {
            if &c[2] == numbers::suffix(n) {
                return vec![ordinal_speech(n), v.into()];
            }
        }
    }
    if let Some((a, b)) = v.split_once(" to ") {
        let endpoint = re!(
            r"^([$€]?)(-?(?:\d{1,3}(?:,\d{3})+|\d+)(?:\.\d+)?)(?: (million|billion|trillion))?(%?)$"
        );
        if let (Some(ac), Some(bc)) = (endpoint.captures(a), endpoint.captures(b)) {
            if ac[1] == bc[1] {
                // "$2 to $3 million" has a shared million. Do not offer
                // "two dollars to three million dollars" as its inverse.
                let mut result = if !ac[1].is_empty() && ac.get(3).is_none() && bc.get(3).is_some()
                {
                    vec![]
                } else {
                    combine(&[verbalize_inner(a), one("to"), verbalize_inner(b)])
                };
                let suffix = match &bc[1] {
                    "$" => vec!["dollar".into(), "dollars".into()],
                    "€" => vec!["euro".into(), "euros".into()],
                    _ if &bc[4] == "%" => one("percent"),
                    _ => one(""),
                };
                if ac[4].is_empty() && (bc[4].is_empty() || bc[1].is_empty()) {
                    let amount = |c: &regex::Captures<'_>| {
                        format!(
                            "{}{}",
                            &c[2],
                            c.get(3)
                                .map_or(String::new(), |s| format!(" {}", s.as_str()))
                        )
                    };
                    result.extend(combine(&[
                        verbalize_inner(&amount(&ac)),
                        one("to"),
                        verbalize_inner(&amount(&bc)),
                        suffix.clone(),
                    ]));
                    if ac.get(3).is_none() && bc.get(3).is_none() {
                        if let (Some(mut av), Some(mut bv)) = (
                            numbers::number(&[&ac[2]], false),
                            numbers::number(&[&bc[2]], false),
                        ) {
                            let thousands = |n: numbers::Number| {
                                (1000..1_000_000).contains(&(n.coefficient / 10i128.pow(n.places)))
                            };
                            if thousands(av) && thousands(bv) && av.places <= 33 && bv.places <= 33
                            {
                                av.places += 3;
                                bv.places += 3;
                                result.extend(combine(&[
                                    numeric_speech(&av.trimmed_digits()),
                                    one("to"),
                                    numeric_speech(&bv.trimmed_digits()),
                                    one("thousand"),
                                    suffix,
                                ]));
                            }
                        }
                    }
                }
                return dedup(result);
            }
        }
    }
    if let Some(c) = re!(r"^(-?(?:\d{1,3}(?:,\d{3})+|\d+)(?:\.\d+)?)(?: (million|billion|trillion))?(?: ([A-Za-z]+))?$").captures(v) {
        return combine(&[numeric_speech(&c[1]), one(c.get(2).map_or("", |m| m.as_str())), one(c.get(3).map_or("", |m| m.as_str()))]);
    }
    one(v)
}
