use super::{tokens, Class};

pub(super) const SMALL: [&str; 20] = [
    "zero",
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
];
pub(super) const TENS: [&str; 10] = [
    "zero", "ten", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
];
pub(super) const SCALES: [(&str, i128); 4] = [
    ("thousand", 1000),
    ("million", 1_000_000),
    ("billion", 1_000_000_000),
    ("trillion", 1_000_000_000_000),
];
pub(super) const ORDINALS: [&str; 32] = [
    "zeroth",
    "first",
    "second",
    "third",
    "fourth",
    "fifth",
    "sixth",
    "seventh",
    "eighth",
    "ninth",
    "tenth",
    "eleventh",
    "twelfth",
    "thirteenth",
    "fourteenth",
    "fifteenth",
    "sixteenth",
    "seventeenth",
    "eighteenth",
    "nineteenth",
    "twentieth",
    "thirtieth",
    "fortieth",
    "fiftieth",
    "sixtieth",
    "seventieth",
    "eightieth",
    "ninetieth",
    "hundredth",
    "thousandth",
    "millionth",
    "billionth",
];
pub(super) const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
pub(super) const AMP_NAMES: [&str; 18] = [
    "R&D", "Q&A", "P&L", "M&A", "B&B", "AT&T", "S&P", "H&M", "T&C", "R&R", "PB&J", "A&W", "D&D",
    "B&W", "G&A", "L&D", "O&M", "F&B",
];
const UNITS: &str = "millisecond milliseconds microsecond microseconds nanosecond nanoseconds gig gigs gigabyte gigabytes megabyte megabytes kilobyte kilobytes byte bytes bit bits terabyte terabytes hertz kilohertz megahertz gigahertz volt volts watt watts kilowatt kilowatts amp amps ampere amperes pixel pixels meter meters metre metres kilometer kilometers kilometre kilometres centimeter centimeters centimetre centimetres millimeter millimeters millimetre millimetres inch inches foot feet mile miles gram grams kilogram kilograms milligram milligrams liter liters litre litres pound pounds ounce ounces degree degrees gb mb kb tb mhz ghz hz db dpi fps ms mbps gbps";
const DURATIONS: &str =
    "second seconds minute minutes hour hours day days week weeks month months year years";
pub(super) fn unit(t: &str) -> bool {
    UNITS.split(' ').any(|s| s == t)
}
pub(super) fn duration(t: &str) -> bool {
    DURATIONS.split(' ').any(|s| s == t)
}
pub(super) fn month(t: &str) -> Option<usize> {
    MONTHS.iter().position(|m| m.eq_ignore_ascii_case(t))
}
pub(super) fn scale(t: &str) -> Option<i128> {
    SCALES.iter().find(|s| s.0 == t).map(|s| s.1)
}
pub(super) fn value(t: &str) -> Option<i128> {
    SMALL
        .iter()
        .position(|w| *w == t)
        .map(|n| n as i128)
        .or_else(|| TENS.iter().position(|w| *w == t).map(|n| n as i128 * 10))
}
pub(super) fn ordinal_word(t: &str) -> Option<&'static str> {
    let i = ORDINALS.iter().position(|w| *w == t)?;
    Some(if i < 20 {
        SMALL[i]
    } else if i < 28 {
        TENS[i - 18]
    } else {
        ["hundred", "thousand", "million", "billion"][i - 28]
    })
}
pub(super) fn numeric_word(t: &str) -> bool {
    value(t).is_some()
        || scale(t).is_some()
        || ordinal_word(t).is_some()
        || matches!(
            t,
            "hundred" | "point" | "oh" | "o" | "and" | "a" | "minus" | "negative"
        )
        || re!(r"^\d[\d,.]*(?:st|nd|rd|th)?$").is_match(t)
}

fn small_int(t: &[&str]) -> Option<i128> {
    if t.len() >= 3 && re!(r"^[1-9]00$").is_match(t[0]) && t[1] == "and" {
        let tail = small_int(&t[2..])?;
        return (0 < tail && tail < 100).then(|| t[0].parse::<i128>().unwrap() + tail);
    }
    if t.contains(&"hundred") {
        if t.len() < 2 || t[1] != "hundred" || t.iter().filter(|w| **w == "hundred").count() != 1 {
            return None;
        }
        let lead = small_int(&t[..1])?;
        if !(1..=9).contains(&lead) {
            return None;
        }
        let mut rest = &t[2..];
        if rest.first() == Some(&"and") {
            rest = &rest[1..];
            if rest.is_empty() {
                return None;
            }
        }
        let tail = if rest.is_empty() { 0 } else { small_int(rest)? };
        return (tail < 100).then_some(lead * 100 + tail);
    }
    match t {
        [a] if re!(r"^\d{1,3}$").is_match(a) => a.parse().ok(),
        [a] => value(a),
        [a, b] if TENS[2..].contains(a) && SMALL[1..10].contains(b) => Some(value(a)? + value(b)?),
        _ => None,
    }
}
pub(super) fn grouped_digits(t: &[&str]) -> Vec<String> {
    let mut result = Vec::new();
    let mut i = 0;
    while i < t.len() {
        let mut end = i;
        while end < t.len() && re!(r"^[0-9]+$").is_match(t[end]) {
            end += 1;
        }
        if end - i >= 2
            && re!(r"^[1-9][0-9]{0,2}$").is_match(t[i])
            && t[i + 1..end].iter().all(|s| s.len() == 3)
        {
            result.push(t[i..end].join(""));
        } else {
            result.extend(t[i..end.max(i + 1)].iter().map(|s| s.to_string()));
        }
        i = end.max(i + 1);
    }
    result
}
pub(super) fn integer(t: &[&str], article: bool) -> Option<i128> {
    if t.is_empty() {
        return None;
    }
    let mut words = t.to_vec();
    if article
        && words.len() > 1
        && words[0] == "a"
        && (words[1] == "hundred" || scale(words[1]).is_some())
    {
        words[0] = "one";
    }
    let t = words.as_slice();
    if t.len() == 1 && re!(r"^(?:\d+|\d{1,3}(?:,\d{3})+)$").is_match(t[0]) {
        return t[0].replace(',', "").parse().ok();
    }
    let (mut total, mut start, mut last) = (0i128, 0, 1_000_000_000_000_000i128);
    for (i, word) in t.iter().enumerate() {
        if let Some(s) = scale(word) {
            if s >= last {
                return None;
            }
            let part = small_int(&t[start..i]).or_else(|| {
                (i == start + 1)
                    .then(|| t[start].parse::<i128>().ok())
                    .flatten()
            })?;
            if part <= 0 || part >= 1000 {
                return None;
            }
            total = total.checked_add(part.checked_mul(s)?)?;
            last = s;
            start = i + 1;
        }
    }
    let mut rest = &t[start..];
    if start > 0 && rest.first() == Some(&"and") {
        rest = &rest[1..];
        if rest.is_empty() {
            return None;
        }
    }
    let tail = if rest.is_empty() && start > 0 {
        0
    } else {
        small_int(rest)?
    };
    total.checked_add(tail)
}

/// Decimal coefficient and scale: no binary floating point or rounding.
#[derive(Clone, Copy, Debug)]
pub(super) struct Number {
    pub coefficient: i128,
    pub places: u32,
}
impl Number {
    pub fn whole(self) -> Option<i128> {
        let p = 10i128.checked_pow(self.places)?;
        (self.coefficient % p == 0).then_some(self.coefficient / p)
    }
    pub fn below_ten(self) -> bool {
        self.coefficient.unsigned_abs() / 10u128.pow(self.places) < 10
    }
    pub fn digits(self, commas: bool) -> String {
        let mut s = self.coefficient.unsigned_abs().to_string();
        if self.places > 0 {
            let width = self.places as usize + 1;
            if s.len() < width {
                s = format!("{}{}", "0".repeat(width - s.len()), s);
            }
            s.insert(s.len() - self.places as usize, '.');
        }
        let split = s.find('.').unwrap_or(s.len());
        if commas {
            for i in (1..split).rev() {
                if (split - i) % 3 == 0 {
                    s.insert(i, ',');
                }
            }
        }
        if self.coefficient < 0 {
            s.insert(0, '-');
        }
        s
    }
    pub fn trimmed_digits(self) -> String {
        let s = self.digits(true);
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_owned()
        } else {
            s
        }
    }
}
pub(super) fn number(t: &[&str], article: bool) -> Option<Number> {
    let grouped = grouped_digits(t);
    let refs: Vec<_> = grouped.iter().map(String::as_str).collect();
    let mut t = refs.as_slice();
    let mut sign = 1i128;
    if matches!(t.first(), Some(&"minus" | &"negative")) {
        sign = -1;
        t = &t[1..];
    }
    if t.is_empty() {
        return None;
    }
    if t.len() == 1 && re!(r"^-?(?:\d+|\d{1,3}(?:,\d{3})+)(?:\.\d+)?$").is_match(t[0]) {
        let s = t[0].replace(',', "");
        let places = s.split_once('.').map_or(0, |(_, f)| f.len() as u32);
        if places > 36 {
            return None;
        }
        return Some(Number {
            coefficient: s.replace('.', "").parse::<i128>().ok()?.checked_mul(sign)?,
            places,
        });
    }
    if t.last().is_some_and(|w| scale(w).is_some())
        && (t.contains(&"point") || t.len() == 2 && re!(r"^\d+(?:\.\d+)?$").is_match(t[0]))
    {
        if let Some(mut n) = number(&t[..t.len() - 1], article) {
            n.coefficient = n
                .coefficient
                .checked_mul(scale(t[t.len() - 1])?)?
                .checked_mul(sign)?;
            return Some(n);
        }
    }
    if let Some(i) = t.iter().position(|w| *w == "point") {
        if t.iter().filter(|w| **w == "point").count() != 1 {
            return None;
        }
        let whole = if i == 0 {
            0
        } else {
            integer(&t[..i], article)?
        };
        let tail = &t[i + 1..];
        if tail.is_empty() {
            return None;
        }
        let frac = if tail.iter().all(|w| {
            SMALL[..10].contains(w) || matches!(*w, "oh" | "o") || re!(r"^\d$").is_match(w)
        }) {
            tail.iter()
                .map(|w| {
                    value(w).map(|v| v.to_string()).unwrap_or_else(|| {
                        if matches!(*w, "oh" | "o") {
                            "0".into()
                        } else {
                            w.to_string()
                        }
                    })
                })
                .collect::<String>()
        } else {
            let v = integer(tail, false)?;
            if !(10..=99).contains(&v) {
                return None;
            }
            v.to_string()
        };
        if frac.len() > 36 {
            return None;
        }
        return Some(Number {
            coefficient: format!("{whole}{frac}")
                .parse::<i128>()
                .ok()?
                .checked_mul(sign)?,
            places: frac.len() as u32,
        });
    }
    Some(Number {
        coefficient: integer(t, article)?.checked_mul(sign)?,
        places: 0,
    })
}
pub(super) fn format_number(t: &[&str], force: bool, article: bool) -> Option<String> {
    let value = number(t, article)?;
    if !force && !t.contains(&"point") && !t.iter().any(|w| w.contains('.')) && value.below_ten() {
        return None;
    }
    if matches!(t.last(), Some(&"million" | &"billion" | &"trillion")) {
        let prefix = &t[..t.len() - 1];
        let prefix = if article && prefix == ["a"] {
            &["one"][..]
        } else {
            prefix
        };
        if let Some(n) = number(prefix, article) {
            return Some(format!("{} {}", n.digits(true), t[t.len() - 1]));
        }
    }
    Some(if t.last().is_some_and(|w| scale(w).is_some()) {
        value.trimmed_digits()
    } else {
        value.digits(true)
    })
}
pub(super) fn suffix(n: i128) -> &'static str {
    if (10..=20).contains(&(n % 100)) {
        "th"
    } else {
        match n % 10 {
            1 => "st",
            2 => "nd",
            3 => "rd",
            _ => "th",
        }
    }
}
pub(super) fn ordinal(t: &[&str]) -> Option<i128> {
    if t.len() == 1 {
        if let Some(c) = re!(r"^(\d+)(st|nd|rd|th)$").captures(t[0]) {
            let v = c[1].parse().ok()?;
            return (suffix(v) == &c[2]).then_some(v);
        }
    }
    let mut words = t.to_vec();
    *words.last_mut()? = ordinal_word(t.last()?)?;
    integer(&words, false)
}
pub(super) fn year(t: &[&str]) -> Option<i128> {
    if let Some(v) = integer(t, false).filter(|v| (1000..=2999).contains(v)) {
        return Some(v);
    }
    if t.last() == Some(&"hundred") {
        if let Some(c) = integer(&t[..t.len() - 1], false).filter(|c| (10..=29).contains(c)) {
            return Some(c * 100);
        }
    }
    for i in 1..t.len() {
        let mut tail = &t[i..];
        let zero = matches!(tail.first(), Some(&"oh" | &"o"));
        if zero {
            tail = &tail[1..];
        }
        if let (Some(a), Some(b)) = (integer(&t[..i], false), integer(tail, false)) {
            if (10..=29).contains(&a) && ((10..=99).contains(&b) || zero && (0..=9).contains(&b)) {
                return Some(a * 100 + b);
            }
        }
    }
    None
}
fn range(t: &[&str], force: bool) -> Option<(String, String)> {
    if t.iter().filter(|w| **w == "to").count() != 1 {
        return None;
    }
    let i = t.iter().position(|w| *w == "to")?;
    let (left, right) = (&t[..i], &t[i + 1..]);
    if left.is_empty()
        || right.is_empty()
        || t.iter()
            .any(|w| matches!(*w, "a" | "minus" | "negative") || w.starts_with('-'))
    {
        return None;
    }
    let (a, b) = (number(left, false)?, number(right, false)?);
    // A spoken hundred on the left could be an explicit magnitude or part
    // of a shared coefficient. Do not choose between those readings.
    if left.contains(&"hundred")
        && !left.iter().any(|w| scale(w).is_some())
        && right.iter().any(|w| scale(w).is_some())
    {
        return None;
    }
    let left_scales = left
        .iter()
        .filter(|w| scale(w).is_some() || **w == "hundred")
        .count();
    let right_scales = right
        .iter()
        .filter(|w| scale(w).is_some() || **w == "hundred")
        .count();
    if right_scales > 0 && left_scales == 0 {
        let magnitude = scale(right[right.len() - 1])?;
        let coefficient = number(&right[..right.len() - 1], false)?;
        let bounded = |n: Number| n.coefficient > 0 && n.coefficient / 10i128.pow(n.places) < 1000;
        if right_scales != 1 || !bounded(a) || !bounded(coefficient) {
            return None;
        }
        if a.coefficient
            .checked_mul(10i128.checked_pow(coefficient.places)?)?
            > coefficient
                .coefficient
                .checked_mul(10i128.checked_pow(a.places)?)?
        {
            return None;
        }
        if magnitude == 1000 {
            let a = Number {
                coefficient: a.coefficient.checked_mul(1000)?,
                places: a.places,
            };
            return Some((a.trimmed_digits(), b.trimmed_digits()));
        }
    } else if left_scales > 0 && right_scales == 0 {
        return None;
    } else if !force
        && !t.contains(&"point")
        && !t.iter().any(|w| re!(r"^\d+\.\d+$").is_match(w))
        && a.below_ten()
        && b.below_ten()
    {
        return None;
    }
    Some((
        format_number(left, true, false)?,
        format_number(right, true, false)?,
    ))
}

fn currency(w: &str) -> Option<&'static str> {
    match w {
        "dollar" | "dollars" => Some("$"),
        "euro" | "euros" => Some("€"),
        _ => None,
    }
}
fn money(t: &[&str]) -> Option<String> {
    if t.contains(&"to") {
        if t.iter().filter(|w| **w == "to").count() != 1 {
            return None;
        }
        let symbol = currency(t.last()?)?;
        let i = t.iter().position(|w| *w == "to")?;
        if i > 0 && currency(t[i - 1]).is_some() {
            if currency(t[i - 1]) != Some(symbol) {
                return None;
            }
            return Some(format!("{} to {}", money(&t[..i])?, money(&t[i + 1..])?));
        }
        let (a, b) = range(&t[..t.len() - 1], true)?;
        return Some(format!("{symbol}{a} to {symbol}{b}"));
    }
    let last = *t.last()?;
    if matches!(last, "bucks" | "buck" | "pounds" | "pound") {
        return Some(format!(
            "{} {last}",
            format_number(&t[..t.len() - 1], false, false)?
        ));
    }
    let indices: Vec<_> = t
        .iter()
        .enumerate()
        .filter(|(_, w)| matches!(**w, "dollar" | "dollars" | "euro" | "euros"))
        .map(|(i, _)| i)
        .collect();
    if let [i] = indices.as_slice() {
        let i = *i;
        let mut amount = format_number(&t[..i], true, true)?;
        let mut rest = &t[i + 1..];
        if !rest.is_empty() {
            if rest.first() == Some(&"and") {
                rest = &rest[1..];
            }
            if !matches!(rest.last(), Some(&"cent" | &"cents")) {
                return None;
            }
            let cents = integer(&rest[..rest.len() - 1], false)?;
            if !(0..100).contains(&cents) {
                return None;
            }
            let dollars = number(&t[..i], true)?.whole()?;
            amount = format!(
                "{}.{cents:02}",
                Number {
                    coefficient: dollars,
                    places: 0
                }
                .digits(true)
            );
        }
        return Some(format!(
            "{}{amount}",
            if t[i].starts_with("dollar") {
                "$"
            } else {
                "€"
            }
        ));
    }
    if matches!(last, "cent" | "cents") {
        return Some(format!(
            "{} {last}",
            format_number(&t[..t.len() - 1], true, false)?
        ));
    }
    None
}
fn day_valid(day: i128, m: usize, y: Option<i128>) -> bool {
    let y = y.unwrap_or(2000);
    let days = match m {
        1 => {
            if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
                29
            } else {
                28
            }
        }
        3 | 5 | 8 | 10 => 30,
        _ => 31,
    };
    (1..=days).contains(&day)
}
fn date(t: &[&str]) -> Option<String> {
    let indices: Vec<_> = t
        .iter()
        .enumerate()
        .filter_map(|(i, w)| month(w).map(|m| (i, m)))
        .collect();
    let [(mi, m)] = indices.as_slice() else {
        return None;
    };
    let (mi, m) = (*mi, *m);
    if mi == 0 {
        for split in 1..t.len() {
            let Some(day) = ordinal(&t[1..=split]).or_else(|| integer(&t[1..=split], false)) else {
                continue;
            };
            let yt = &t[split + 1..];
            let y = year(yt);
            if !yt.is_empty() && y.is_none() || !day_valid(day, m, y) {
                continue;
            }
            return Some(format!(
                "{} {day}{}",
                MONTHS[m],
                y.map_or(String::new(), |v| format!(", {v}"))
            ));
        }
    } else if mi >= 2 && t[mi - 1] == "of" {
        let start = usize::from(t[0] == "the");
        let day = ordinal(&t[start..mi - 1])?;
        let yt = &t[mi + 1..];
        let y = year(yt);
        if !yt.is_empty() && y.is_none() || !day_valid(day, m, y) {
            return None;
        }
        return Some(format!(
            "{}{day}{} of {}{}",
            if start == 1 { "the " } else { "" },
            suffix(day),
            MONTHS[m],
            y.map_or(String::new(), |v| format!(", {v}"))
        ));
    }
    None
}
fn time(mut t: &[&str], before: &str) -> Option<String> {
    if t.iter().any(|w| w.contains("clock")) {
        return None;
    }
    let prefix = if t.first() == Some(&"at") {
        t = &t[1..];
        "at "
    } else {
        ""
    };
    let period = if matches!(t.last(), Some(&"am" | &"pm")) {
        let p = t[t.len() - 1].to_uppercase();
        t = &t[..t.len() - 1];
        Some(p)
    } else {
        None
    };
    if period.is_none()
        && prefix.is_empty()
        && tokens(before).last().map(String::as_str) != Some("at")
    {
        return None;
    }
    let limit = if period.is_some() { 12 } else { 23 };
    let hour = integer(t, false);
    if let Some(h) = hour {
        return if period.is_some() && (1..=limit).contains(&h) {
            Some(format!("{prefix}{h} {}", period?))
        } else {
            None
        };
    }
    for i in 1..t.len() {
        let mut mt = &t[i..];
        if matches!(mt.first(), Some(&"oh" | &"o")) {
            mt = &mt[1..];
        }
        if let (Some(h), Some(m)) = (integer(&t[..i], false), integer(mt, false)) {
            if (i128::from(period.is_some())..=limit).contains(&h) && (0..60).contains(&m) {
                return Some(format!(
                    "{prefix}{h}:{m:02}{}",
                    period.map_or(String::new(), |p| format!(" {p}"))
                ));
            }
        }
    }
    None
}
fn version(t: &[&str]) -> Option<String> {
    if t.len() < 2 || !matches!(t[0], "version" | "v" | "python" | ".net") {
        return None;
    }
    let values = &t[1..];
    let component = |g: &[&str]| -> Option<String> {
        if g.len() == 1 && re!(r"^[0-9]+$").is_match(g[0]) {
            return Some(g[0].to_owned());
        }
        if !g.is_empty()
            && g.iter()
                .all(|w| SMALL[..10].contains(w) || matches!(*w, "oh" | "o"))
        {
            return Some(
                g.iter()
                    .map(|w| value(w).unwrap_or(0).to_string())
                    .collect(),
            );
        }
        integer(g, false).map(|n| n.to_string())
    };
    let parts: Vec<String> = if values.contains(&"point") {
        values
            .split(|w| *w == "point")
            .map(component)
            .collect::<Option<_>>()?
    } else if values.len() == 1 && re!(r"^\d+(?:\.\d+)*$").is_match(values[0]) {
        values[0].split('.').map(str::to_owned).collect()
    } else if let Some(v) = integer(values, false) {
        vec![v.to_string()]
    } else if matches!(t[0], "python" | ".net") && values.len() == 2 {
        values
            .iter()
            .map(|w| component(&[w]))
            .collect::<Option<_>>()?
    } else {
        return None;
    };
    if parts
        .iter()
        .any(|p| p.len() > 4 || p.parse::<u32>().is_err())
    {
        return None;
    }
    let v = parts.join(".");
    Some(format!(
        "{}{v}",
        match t[0] {
            "v" => "v",
            "python" => "Python ",
            ".net" => ".NET ",
            _ => "version ",
        }
    ))
}

pub(super) fn candidate(
    t: &[&str],
    class: Class,
    before: &str,
    after: &str,
    original: &str,
) -> Option<String> {
    let last = *t.last()?;
    match class {
        Class::Model => None,
        Class::Amp => {
            if t.iter().filter(|w| **w == "and").count() != 1
                || t.iter().any(|w| {
                    *w != "and" && !(w.len() == 1 && w.as_bytes()[0].is_ascii_alphabetic())
                })
            {
                return None;
            }
            let name: String = t
                .iter()
                .map(|w| {
                    if *w == "and" {
                        "&".into()
                    } else {
                        w.to_uppercase()
                    }
                })
                .collect();
            AMP_NAMES.contains(&name.as_str()).then_some(name)
        }
        Class::Cardinal => {
            if matches!(
                tokens(before).last().map(String::as_str),
                Some("number" | "step")
            ) || t[0] == "minus"
            {
                return None;
            }
            if t.contains(&"to") {
                let (a, b) = range(t, false)?;
                return Some(format!("{a} to {b}"));
            }
            let article = t == ["a", "billion"]
                && tokens(after).first().is_some_and(|w| {
                    matches!(
                        w.as_str(),
                        "parameters" | "tokens" | "shares" | "bytes" | "records" | "requests"
                    )
                });
            format_number(t, false, article)
        }
        Class::Decimal => {
            if !t.contains(&"point") && !t.iter().any(|w| re!(r"^\d+\.\d+$").is_match(w)) {
                None
            } else if t.contains(&"to") {
                let (a, b) = range(t, true)?;
                Some(format!("{a} to {b}"))
            } else {
                format_number(t, true, false)
            }
        }
        Class::Money => money(t),
        Class::Percent if last == "percent" => {
            if t.contains(&"to") {
                if t.iter().filter(|w| **w == "to").count() != 1 {
                    return None;
                }
                let i = t.iter().position(|w| *w == "to")?;
                if i > 0 && t[i - 1] == "percent" {
                    return Some(format!(
                        "{}% to {}%",
                        format_number(&t[..i - 1], true, true)?,
                        format_number(&t[i + 1..t.len() - 1], true, true)?
                    ));
                }
                let (a, b) = range(&t[..t.len() - 1], true)?;
                return Some(format!("{a} to {b}%"));
            }
            Some(format!(
                "{}%",
                format_number(&t[..t.len() - 1], true, true)?
            ))
        }
        Class::Magnitude if matches!(last, "k" | "m" | "x") => Some(format!(
            "{}{}",
            format_number(&t[..t.len() - 1], true, false)?,
            match last {
                "k" => "K",
                "m" => "M",
                _ => "x",
            }
        )),
        Class::Year => year(t).map(|n| n.to_string()),
        Class::Date => date(t),
        Class::Time => time(t, before),
        Class::Ordinal => {
            let n = ordinal(t)?;
            (n >= 10).then(|| format!("{n}{}", suffix(n)))
        }
        Class::Measure => {
            if unit(last) || duration(last) {
                Some(format!(
                    "{} {}",
                    format_number(&t[..t.len() - 1], unit(last), false)?,
                    original
                        .split_whitespace()
                        .last()?
                        .trim_end_matches(|c| ".,;:!?".contains(c))
                ))
            } else {
                let at = tokens(after);
                let u = at.first()?;
                if unit(u) || duration(u) {
                    format_number(t, unit(u), false)
                } else {
                    None
                }
            }
        }
        Class::Version => {
            let mut result = version(t)?;
            if t[0] == "v" && original.starts_with('V') {
                result.replace_range(..1, "V");
            }
            Some(result)
        }
        _ => None,
    }
}
