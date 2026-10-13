use super::*;

#[test]
fn reference_style_and_regressions() {
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("reference_cases.json")).unwrap();
    let mut failures = Vec::new();
    for row in cases.as_array().unwrap() {
        let text = row[0].as_str().unwrap();
        let Ok(class) = row[1].as_str().unwrap().parse::<Class>() else {
            continue;
        };
        if class == Class::Model {
            continue;
        } // Superseded by the model grammar.
        let expected = row[2].as_str();
        let got = render_span(text, class, None, None);
        if let Some(expected) = expected {
            if got.as_deref().unwrap_or(text) != expected {
                failures.push(format!(
                    "{text:?} {class:?}: {got:?}, expected {expected:?}"
                ));
            }
        } else if got.is_some() {
            failures.push(format!("{text:?} {class:?}: unsafe {got:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn lexicon_examples() {
    let lex: serde_json::Value = serde_json::from_str(include_str!("models.json")).unwrap();
    let mut failures = Vec::new();
    for family in lex["families"].as_array().unwrap() {
        for row in family["examples"].as_array().unwrap() {
            let text = row["spoken"].as_str().unwrap();
            let expected = row["written"].as_str().unwrap();
            // The assignment's explicit ambiguity rule overrides these four
            // uncued examples in the supplied lexicon (see PROVENANCE.md).
            if family["id"] == "jev" || text == "clef" {
                assert_eq!(render_span(text, Class::Model, None, None), None);
                continue;
            }
            let got = render_span(text, Class::Model, None, None);
            if got.as_deref() != Some(expected) {
                failures.push(format!("{text:?}: {got:?}, expected {expected:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn all_amp_names() {
    for name in numbers::AMP_NAMES {
        let spoken = name
            .chars()
            .map(|c| {
                if c == '&' {
                    "and".into()
                } else {
                    c.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(
            render_span(&spoken, Class::Amp, None, None).as_deref(),
            Some(name)
        );
    }
}

#[test]
fn context_and_guard() {
    assert_eq!(
        render_span("nine thirty", Class::Time, Some("arrive at"), None).as_deref(),
        Some("9:30")
    );
    assert_eq!(
        render_span("ten", Class::Cardinal, Some("step"), None),
        None
    );
    assert_eq!(
        render_span("ten", Class::Cardinal, Some("number"), None),
        None
    );
    assert_eq!(
        render_span("five", Class::Measure, None, Some("milliseconds")).as_deref(),
        Some("5")
    );
    assert_eq!(
        render_span("five", Class::Measure, None, Some("seconds")),
        None
    );
    assert_eq!(
        render_with_guard("ten dollars", Class::Money, None, None, |_, _, _| false),
        None
    );
}

#[test]
fn bio_contract_and_alignment() {
    for class in Class::ALL {
        for label in [BioLabel::B(class), BioLabel::I(class)] {
            let json = serde_json::to_string(&label).unwrap();
            assert_eq!(serde_json::from_str::<BioLabel>(&json).unwrap(), label);
        }
    }
    assert!(serde_json::from_str::<BioLabel>("\"B-UNKNOWN\"").is_err());
    assert_eq!(apply("ten dollars", Some(&[BioLabel::O])), "ten dollars");
    assert_eq!(
        apply("ten dollars", Some(&[BioLabel::I(Class::Money); 2])),
        "$10"
    );
    assert_eq!(apply("ten dollars", Some(&[BioLabel::O; 2])), "ten dollars");
    assert_eq!(
        spans(
            &["ten", "dollars"],
            Some(&[BioLabel::B(Class::Money), BioLabel::I(Class::Money)])
        ),
        vec![Span {
            start: 0,
            end: 2,
            class: Class::Money
        }]
    );
    assert_eq!(
        apply(
            "  Keep\tTHE name; pay ten dollars\t a month.\r\n",
            Some(&[
                BioLabel::O,
                BioLabel::O,
                BioLabel::O,
                BioLabel::O,
                BioLabel::B(Class::Money),
                BioLabel::I(Class::Money),
                BioLabel::O,
                BioLabel::O
            ])
        ),
        "  Keep\tTHE name; pay $10\t a month.\r\n"
    );
}

#[test]
fn rules_examples() {
    for (source, expected) in [
        ("Ten percent of it", "10% of it"),
        ("four thousand. Dollars", "$4,000"),
        ("GPT six point one. Soul", "GPT-6.1 Sol"),
        ("GLM. 5.3 flash", "GLM-5.3-Flash"),
        ("Opus 5.5", "Opus 5.5"),
        ("4.2 million dollars", "$4.2 million"),
        ("ten million shares", "10 million shares"),
        ("fifty-eight thousand", "58,000"),
        ("two hundred gigs", "200 gigs"),
        ("three to six months", "three to six months"),
        ("ten to twenty", "10 to 20"),
        ("ten thousand dollar investment", "$10,000 investment"),
        ("two dollars and fifty cents", "$2.50"),
        ("in twenty twenty five", "in 2025"),
        ("since two thousand five", "since 2005"),
        ("April thirtieth, twenty twenty seven", "April 30, 2027"),
        ("the thirtieth of April", "the 30th of April"),
        ("twenty first century", "21st century"),
        ("at nine thirty", "at 9:30"),
        ("three thirty pm", "3:30 PM"),
        ("version two point three point one", "version 2.3.1"),
        (".NET ten", ".NET 10"),
        ("Python three twelve", "Python 3.12"),
        ("sixteen GB", "16 GB"),
        ("R and D", "R&D"),
        ("S and P five hundred", "S&P 500"),
        ("Devon CLI", "Devin CLI"),
        ("GPT six Astra on Ultra", "GPT-6 Astra on Ultra"),
        ("GPT six Astra on x high", "GPT-6 Astra on xhigh"),
        ("GPT six extra high", "GPT-6 extra high"),
    ] {
        assert_eq!(apply(source, None), expected, "{source}");
    }
}

#[test]
fn preserve_negative_style_rows_and_literals() {
    for source in [
        "two checks",
        "three days",
        "one of them",
        "the one I sent",
        "one of these days",
        "no one",
        "number one",
        "step two",
        "step ten",
        "the first time",
        "third quarter",
        "fifth",
        "a hundred bucks",
        "one hundred and",
        "one hundred and and five",
        "February thirtieth",
        "April thirty one",
        "a couple",
        "several thousand",
        "hundreds of",
        "a million times",
        "one or two",
        "several thousand dollar bill",
        "a few bucks",
        "spelled S O U L",
        "a half",
        "three quarters",
        "end to end",
        "the nineties",
        "nine thirty",
        "three o'clock",
        "five seconds",
        "A and B",
        "research and development",
        "one two hundred",
        "one hundred hundred",
        "one million million",
        "soul",
        "the cloud is high",
        "a clef",
        "a jev",
        "cloud code",
        "`ten dollars`",
        "```rust\nten dollars\n```",
        "\"ten dollars\"",
        "‘ten percent’",
        "'ten dollars'",
        "https://example.com/ten_percent",
        "somebody@example.com",
        "C:\\ten\\dollars",
        " ten\n dollars ",
    ] {
        assert_eq!(apply(source, None), source, "{source}");
    }
    assert_eq!(
        apply("é  café\t ten dollars,\r\n🦀", None),
        "é  café\t $10,\r\n🦀"
    );
    assert_eq!(
        apply("\"ten dollars\" then ten dollars", None),
        "\"ten dollars\" then $10"
    );
    assert_eq!(
        apply(
            "`ten dollars`",
            Some(&[BioLabel::B(Class::Money), BioLabel::I(Class::Money)])
        ),
        "`ten dollars`"
    );
}

#[test]
fn reverse_verbalizer_examples() {
    for (written, spoken) in [
        ("$4,000", "four thousand dollars"),
        ("$2.50", "two dollars and fifty cents"),
        ("$2.05", "two dollars and five cents"),
        ("$4.2 million", "four point two million dollars"),
        ("€20", "twenty euros"),
        ("100%", "a hundred percent"),
        ("0.5%", "point five percent"),
        ("4K", "four K"),
        ("10x", "ten x"),
        ("2025", "twenty twenty five"),
        ("2005", "two thousand five"),
        ("April 30, 2027", "April thirtieth twenty twenty seven"),
        ("the 30th of April", "the thirtieth of April"),
        ("3:30 PM", "three thirty PM"),
        ("9 AM", "nine AM"),
        ("version 2.3.1", "version two point three point one"),
        ("Python 3.12", "Python three twelve"),
        (".NET 10", ".NET ten"),
        ("v2", "v two"),
        ("256 gigabytes", "two hundred fifty six gigabytes"),
        ("21st", "twenty first"),
        ("R&D", "R and D"),
    ] {
        assert!(
            verbalize(written).iter().any(|s| s == spoken),
            "{written} => {spoken}"
        );
    }
    assert!(verbalize("").is_empty());
    assert_eq!(verbalize("unknown text"), vec!["unknown text"]);
}

#[test]
fn compositional_models_not_a_known_name_lookup() {
    for (spoken, written) in [
        ("GPT six point nine Sol", "GPT-6.9 Sol"),
        ("Grok four point two zero", "Grok 4.20"),
        ("Grok 4.20", "Grok 4.20"),
        ("GLM six point two v turbo", "GLM-6.2V-Turbo"),
        (
            "Qwen four point two coder thirty two b a four b",
            "Qwen4.2-Coder-32B-A4B",
        ),
        ("Gemma five n e eight b", "Gemma 5n E8B"),
        (
            "Nemotron four Super one twenty seven b a twelve b",
            "Nemotron 4 Super 127B A12B",
        ),
        ("Claude six point five sonnet", "Claude 6.5 Sonnet"),
        ("Opus six point five", "Opus 6.5"),
        ("gpt six astra on Extra High", "GPT-6 Astra on Extra High"),
        ("swe two high", "SWE-2 high"),
        ("gpt six astra on maximum", "GPT-6 Astra on maximum"),
        ("gpt six astra on x high", "GPT-6 Astra on xhigh"),
        ("gpt six x high", "GPT-6 xhigh"),
        ("gpt six with ex high", "GPT-6 with ex high"),
        ("swe two on ex high", "SWE-2 on ex high"),
    ] {
        assert_eq!(
            render_span(spoken, Class::Model, None, None).as_deref(),
            Some(written),
            "{spoken}"
        );
    }
    for text in [
        "soul",
        "extra high",
        "gpt astra six",
        "gpt six soul pro",
        "gpt six nonsense",
        "qwen three zero b",
        "gpt oss six",
        "whisper two",
        "parakeet three",
        "o three point five",
        "gemma three point one",
        "gpt one point two point three",
        "grok point six",
        "grok minus four",
        "grok",
        "cloud",
        "queen",
        "croak",
        "Devon",
        "Jeff model",
        "TypeSafe Jev",
        "cloud model",
        "croak model",
        "Devon model",
        "sweet",
        "sweet model",
        "claude sonnet opus five",
    ] {
        assert_eq!(render_span(text, Class::Model, None, None), None, "{text}");
    }
}

#[test]
fn product_aliases_are_whole_and_ambiguous_products_stay() {
    let lex: serde_json::Value = serde_json::from_str(include_str!("models.json")).unwrap();
    for product in lex["products"].as_array().unwrap() {
        let canonical = product["canonical"].as_str().unwrap();
        for alias in product["spoken"]
            .as_array()
            .unwrap()
            .iter()
            .chain(product["misheard"].as_array().unwrap())
        {
            let alias = alias.as_str().unwrap();
            let ambiguous = product["ambiguous"] == true;
            // Groq/Grok is a family alias too, but needs a version/tier.
            let result = render_span(alias, Class::Model, None, None);
            if ambiguous || canonical == "Claude Code" && alias.starts_with("cloud") {
                assert_eq!(result, None, "{alias}");
            } else {
                assert_eq!(result.as_deref(), Some(canonical), "{alias}");
            }
        }
    }
    assert_eq!(
        render_span("cloud code", Class::Model, None, Some("agent reads it")).as_deref(),
        Some("Claude Code")
    );
    assert_eq!(
        render_span("cloud code", Class::Model, None, Some("for Google")),
        None
    );
}

#[test]
fn rules_do_not_rescue_bad_fragments_or_idioms() {
    for text in [
        "cloud nine",
        "I'm on cloud nine after two checks.",
        "the eleventh hour",
        "A tenth of a second matters.",
        "The Fourth of July party.",
        "two gigs this weekend",
        "The codex is on display at the museum.",
        "Ubuntu twenty four oh four",
        "several thousand dollars",
        "one million million",
        "twenty thirteen",
        "one two hundred",
        "two hundred hundred",
        "a hundred bucks",
        "two dollars and one hundred cents",
        "ten dollars and one hundred cents",
        "two dollars one hundred cents",
    ] {
        assert_eq!(apply(text, None), text, "{text}");
    }
    for (text, expected) in [
        ("a five dollar fee", "a $5 fee"),
        ("ten dollars a month", "$10 a month"),
        ("two dollars and fees", "$2 and fees"),
        ("a one M window", "a 1M window"),
        ("a twenty-seven inch monitor", "a 27 inch monitor"),
        ("ten or fifteen minutes", "10 or 15 minutes"),
        ("in June twenty twenty four", "in June 2024"),
        ("Croak four point six first", "Grok 4.6 first"),
        ("Python three. Twelve first", "Python 3.12 first"),
    ] {
        assert_eq!(apply(text, None), expected, "{text}");
    }
}

#[test]
fn mixed_digits_and_exact_decimal_precision() {
    for (text, class, expected) in [
        ("4.2 million dollars", Class::Money, "$4.2 million"),
        ("4 million dollars", Class::Money, "$4 million"),
        ("4.20 percent", Class::Percent, "4.20%"),
        (
            "0.000000000000000001 dollars",
            Class::Money,
            "$0.000000000000000001",
        ),
        (
            "9007199254740993 dollars",
            Class::Money,
            "$9,007,199,254,740,993",
        ),
        ("2 dollars and 5 cents", Class::Money, "$2.05"),
        ("16 GB", Class::Measure, "16 GB"),
        ("version 2.3.1", Class::Version, "version 2.3.1"),
        ("GLM 5.3 flash", Class::Model, "GLM-5.3-Flash"),
    ] {
        assert_eq!(
            render_span(text, class, None, None).as_deref(),
            Some(expected),
            "{text}"
        );
    }
    for text in ["9".repeat(300), format!("0.{} dollars", "1".repeat(80))] {
        for class in Class::ALL {
            assert_eq!(render_span(&text, class, None, None), None, "{class:?}");
        }
    }
}

#[test]
fn spans_and_replacements_are_stable() {
    let text = "  Keep café.\tPay four thousand dollars; use GPT six point one Soul.\r\nDone 🦀.  ";
    let expected = "  Keep café.\tPay $4,000; use GPT-6.1 Sol.\r\nDone 🦀.  ";
    assert_eq!(apply(text, None), expected);
    assert_eq!(apply(expected, None), expected);
    assert_eq!(apply(" \t\r\n", None), " \t\r\n");
    assert_eq!(
        apply(
            "ten\n dollars",
            Some(&[BioLabel::B(Class::Money), BioLabel::I(Class::Money)])
        ),
        "ten\n dollars"
    );
    assert_eq!(
        spans(
            &["five", "ten"],
            Some(&[BioLabel::B(Class::Cardinal), BioLabel::B(Class::Cardinal)])
        )
        .len(),
        2
    );
}

#[test]
fn round2_renderer_cases() {
    let cases: Vec<(String, Class, Option<String>)> =
        serde_json::from_str(include_str!("round2_cases.json")).unwrap();
    let mut failures = Vec::new();
    for (text, class, expected) in cases {
        let got = render_span(&text, class, None, None);
        if got != expected {
            failures.push(format!(
                "{class:?} {text:?}: {got:?}, expected {expected:?}"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn round2_rules_keep_ranges_whole() {
    for (text, expected) in [
        (
            "Keep THE range: six to ten thousand dollars, please!",
            "Keep THE range: $6,000 to $10,000, please!",
        ),
        ("six to ten thousand", "6,000 to 10,000"),
        ("two to three million", "2 to 3 million"),
        ("two to three billion euros", "€2 to €3 billion"),
        ("ten to fifteen percent growth", "10 to 15% growth"),
        ("ten percent to fifteen percent", "10% to 15%"),
        ("V four and v two", "V4 and v2"),
        ("60 000 records", "60,000 records"),
        ("one million, 200 and thirty thousand", "1,230,000"),
        ("a billion parameters", "1 billion parameters"),
    ] {
        assert_eq!(apply(text, None), expected, "{text}");
    }
    for text in [
        "six to ten thousand thousand",
        "six to ten thousand and five",
        "1500 to two thousand",
        "one thousand to two",
        "one to two to three million",
        "one to two thousand thousand dollars",
        "two euros to five dollars",
        "two dollars to three million dollars",
        "two hundred to three hundred thousand",
        "ten to fifteen percent percent",
        "GA four",
        "ga four",
        "V two point point three",
        "60 00",
        "60 0000",
        "6000 000",
        "60 000 00",
        "one million, 201 and thirty thousand",
        "a billion times",
        "a billion dreams",
    ] {
        assert_eq!(apply(text, None), text, "{text}");
    }
}

#[test]
fn round2_context_and_guard() {
    for noun in [
        "parameters",
        "tokens",
        "shares",
        "bytes",
        "records",
        "requests",
    ] {
        assert_eq!(
            render_span("a billion", Class::Cardinal, None, Some(noun)).as_deref(),
            Some("1 billion")
        );
    }
    for noun in ["times", "dreams", "things", "", "years"] {
        assert_eq!(
            render_span("a billion", Class::Cardinal, None, Some(noun)),
            None
        );
    }
    assert_eq!(
        render_with_guard(
            "six to ten thousand dollars",
            Class::Money,
            None,
            None,
            |_, _, _| false
        ),
        None
    );
    assert_eq!(
        apply(
            "six to ten thousand",
            Some(&[
                BioLabel::B(Class::Cardinal),
                BioLabel::O,
                BioLabel::B(Class::Cardinal),
                BioLabel::I(Class::Cardinal)
            ])
        ),
        "6,000 to 10,000"
    );
    assert_eq!(
        apply(
            "one thousand to twenty",
            Some(&[
                BioLabel::B(Class::Cardinal),
                BioLabel::I(Class::Cardinal),
                BioLabel::O,
                BioLabel::B(Class::Cardinal)
            ])
        ),
        "one thousand to twenty"
    );
}

#[test]
fn routed_special_classes_use_rules_even_with_all_o_labels() {
    for (text, class, expected) in [
        ("GPT six Astra", Class::Model, "GPT-6 Astra"),
        ("version two point three", Class::Version, "version 2.3"),
        ("R and D", Class::Amp, "R&D"),
    ] {
        let words: Vec<_> = text.split_whitespace().collect();
        let labels = vec![BioLabel::O; words.len()];
        assert_eq!(
            ModelProposer::new(&labels).spans(&words),
            vec![Span {
                start: 0,
                end: words.len(),
                class,
            }],
            "{text}"
        );
        assert_eq!(apply(text, Some(&labels)), expected, "{text}");
    }
}

#[test]
fn decimal_thousands_round_trip_matches_reference_without_rounding() {
    for (spoken, written) in [
        ("twelve point five thousand", "12,500"),
        ("one point zero zero one thousand", "1,001"),
        ("minus forty two point one two five thousand", "-42,125"),
    ] {
        assert_eq!(render_span(spoken, Class::Decimal, None, None), Some(written.into()));
        assert!(verbalize(written).iter().any(|value| value == spoken));
    }
    assert_eq!(render_span("twelve point five thousand", Class::Decimal, Some("ten to"), None), None);
}

#[test]
fn explicit_model_cue_preserves_source_case_like_reference() {
    for cue in ["model", "Model", "MODEL"] {
        let source = format!("GLM four {cue}");
        assert_eq!(render_span(&source, Class::Model, None, None), Some(format!("GLM-4 {cue}")));
        let labels = vec![BioLabel::O; 3];
        assert_eq!(apply(&source, Some(&labels)), format!("GLM-4 {cue}"));
    }
}

#[test]
fn routed_numeric_classes_keep_complete_model_spans() {
    // Class selection must not depend on whether the renderer accepts a span.
    for class in Class::ALL {
        if matches!(class, Class::Model | Class::Version | Class::Amp) {
            continue;
        }
        assert_eq!(
            spans(&["widget"], Some(&[BioLabel::B(class)])),
            vec![Span {
                start: 0,
                end: 1,
                class,
            }]
        );
    }

    // A year without a rules cue still uses the model's class and boundaries.
    let text = "twenty twenty five";
    let labels = [
        BioLabel::B(Class::Year),
        BioLabel::I(Class::Year),
        BioLabel::I(Class::Year),
    ];
    assert_eq!(apply(text, None), text);
    assert_eq!(apply(text, Some(&labels)), "2025");

    // The rules would include the currency, but a complete model cardinal wins.
    let labels = [BioLabel::B(Class::Cardinal), BioLabel::O];
    assert_eq!(apply("ten dollars", Some(&labels)), "10 dollars");
}

#[test]
fn routed_numeric_fallback_requires_non_o_inside_the_rule_span() {
    let text = "ten dollars then twenty percent";
    let labels = [
        BioLabel::O,
        BioLabel::O,
        BioLabel::I(Class::Money),
        BioLabel::O,
        BioLabel::O,
    ];
    assert_eq!(apply(text, Some(&labels)), text);
    assert_eq!(apply(text, None), "$10 then 20%");
    assert!(spans(&text.split_whitespace().collect::<Vec<_>>(), Some(&labels)).is_empty());

    // Any class is evidence, including special-class orphan continuations.
    for class in Class::ALL {
        for labels in [
            [BioLabel::I(class), BioLabel::O],
            [BioLabel::O, BioLabel::I(class)],
            [BioLabel::I(class), BioLabel::I(class)],
        ] {
            assert_eq!(apply("ten dollars", Some(&labels)), "$10", "{labels:?}");
        }
    }
}

#[test]
fn routed_orphans_never_start_model_spans() {
    assert!(spans(&["widget"], Some(&[BioLabel::I(Class::Money)])).is_empty());
    assert_eq!(apply("five", Some(&[BioLabel::I(Class::Cardinal)])), "five");

    // A mismatched continuation cannot extend the preceding complete span.
    // It can license the adjacent rules span without inheriting its class.
    let text = "marker ten dollars";
    let labels = [
        BioLabel::B(Class::Decimal),
        BioLabel::I(Class::Year),
        BioLabel::O,
    ];
    assert_eq!(
        spans(&text.split_whitespace().collect::<Vec<_>>(), Some(&labels)),
        vec![
            Span {
                start: 0,
                end: 1,
                class: Class::Decimal
            },
            Span {
                start: 1,
                end: 3,
                class: Class::Money
            },
        ]
    );
    assert_eq!(apply(text, Some(&labels)), "marker $10");
}

#[test]
fn routed_unrenderable_different_class_spans_block_fallback() {
    let text = "ten dollars";
    for class in [Class::Decimal, Class::Model, Class::Version, Class::Amp] {
        for labels in [
            [BioLabel::B(class), BioLabel::O],
            [BioLabel::B(class), BioLabel::I(class)],
            [BioLabel::O, BioLabel::B(class)],
        ] {
            assert_eq!(apply(text, Some(&labels)), text, "{labels:?}");
            assert!(apply_with_edits(text, Some(&labels)).1.is_empty());
        }
    }

    // Half-open boundaries: a different-class rejection blocks only overlaps.
    let text = "ten dollars twenty percent";
    let labels = [
        BioLabel::B(Class::Decimal),
        BioLabel::I(Class::Decimal),
        BioLabel::I(Class::Money),
        BioLabel::O,
    ];
    assert_eq!(apply(text, Some(&labels)), "ten dollars 20%");
}

#[test]
fn routed_fallback_requires_every_overlapping_model_span_to_be_unrenderable() {
    let labels = [
        BioLabel::B(Class::Cardinal),
        BioLabel::B(Class::Money),
        BioLabel::O,
    ];
    // The MONEY span fails, but the adjacent renderable CARDINAL still wins.
    assert_eq!(
        apply("twenty five dollars", Some(&labels)),
        "20 five dollars"
    );
    let labels = [
        BioLabel::B(Class::Money),
        BioLabel::B(Class::Decimal),
        BioLabel::O,
    ];
    assert_eq!(apply("twenty five dollars", Some(&labels)), "twenty five dollars");
    let labels = [
        BioLabel::B(Class::Money),
        BioLabel::B(Class::Money),
        BioLabel::O,
    ];
    assert_eq!(apply("twenty five dollars", Some(&labels)), "$25");
    let labels = [
        BioLabel::B(Class::Money),
        BioLabel::B(Class::Money),
        BioLabel::I(Class::Money),
    ];
    // A renderable same-class overlap still blocks the larger rules span.
    assert_eq!(apply("twenty five dollars", Some(&labels)), "twenty $5");
}

#[test]
fn routed_public_sentence_recovers_truncated_money_span() {
    let text = "Send ten percent of the four thousand dollars to GPT six point one Soul by twenty twenty five.";
    let labels = [
        "O",
        "B-PERCENT",
        "I-PERCENT",
        "O",
        "O",
        "B-MONEY",
        "O",
        "O",
        "O",
        "B-MODEL",
        "I-MODEL",
        "I-MODEL",
        "I-MODEL",
        "I-MODEL",
        "O",
        "B-YEAR",
        "I-YEAR",
        "I-YEAR",
    ]
    .map(|label| label.parse().unwrap());
    assert_eq!(
        apply(text, Some(&labels)),
        "Send 10% of the $4,000 to GPT-6.1 Sol by 2025."
    );
}

#[test]
fn routed_rules_specials_win_cross_route_collisions_whole() {
    for (special, class, expected) in [
        ("GPT six Astra", Class::Model, "GPT-6 Astra"),
        ("version two point three", Class::Version, "version 2.3"),
        ("R and D", Class::Amp, "R&D"),
    ] {
        let text = format!("ten dollars then {special} afterwards twenty percent");
        let words: Vec<_> = text.split_whitespace().collect();
        let mut labels = vec![BioLabel::I(Class::Money); words.len()];
        labels[0] = BioLabel::B(Class::Money);
        let selected = vec![
            Span {
                start: 0,
                end: 2,
                class: Class::Money,
            },
            Span {
                start: 3,
                end: 3 + special.split_whitespace().count(),
                class,
            },
        ];
        assert_eq!(spans(&words, Some(&labels)), selected, "{special}");
        assert_eq!(
            apply(&text, Some(&labels)),
            format!("$10 then {expected} afterwards twenty percent")
        );
        // The displaced MONEY span permits MONEY fallback, but blocks PERCENT.
        assert_eq!(apply_with_edits(&text, Some(&labels)).1.len(), 2);
    }

    // Equal starts and a model span contained inside a special both keep the
    // entire lexicon span. The model never splits or trims either candidate.
    let text = "GPT six Astra";
    for labels in [
        [
            BioLabel::B(Class::Cardinal),
            BioLabel::I(Class::Cardinal),
            BioLabel::I(Class::Cardinal),
        ],
        [BioLabel::O, BioLabel::B(Class::Cardinal), BioLabel::O],
    ] {
        assert_eq!(apply(text, Some(&labels)), "GPT-6 Astra");
    }
}

#[test]
fn routed_output_is_ordered_disjoint_and_keeps_adjacent_routes() {
    let text = "ten dollars GPT six Astra then twenty percent R and D version two";
    let words: Vec<_> = text.split_whitespace().collect();
    let mut labels = vec![BioLabel::O; words.len()];
    labels[0] = BioLabel::B(Class::Money);
    labels[1] = BioLabel::I(Class::Money);
    labels[6] = BioLabel::I(Class::Percent);
    let selected = spans(&words, Some(&labels));
    assert_eq!(
        selected,
        vec![
            Span {
                start: 0,
                end: 2,
                class: Class::Money
            },
            Span {
                start: 2,
                end: 5,
                class: Class::Model
            },
            Span {
                start: 6,
                end: 8,
                class: Class::Percent
            },
            Span {
                start: 8,
                end: 11,
                class: Class::Amp
            },
            Span {
                start: 11,
                end: 13,
                class: Class::Version
            },
        ]
    );
    assert_eq!(
        apply_spans(text, &selected),
        "$10 GPT-6 Astra then 20% R&D version 2"
    );
}

#[test]
fn routed_misalignment_fails_closed_including_special_rules() {
    for text in ["GPT six Astra", "version two", "R and D", "ten dollars", ""] {
        let words: Vec<_> = text.split_whitespace().collect();
        for length in [words.len().saturating_sub(1), words.len() + 1] {
            if length == words.len() {
                continue;
            }
            let labels = vec![BioLabel::O; length];
            assert!(spans(&words, Some(&labels)).is_empty(), "{text}");
            assert_eq!(apply_with_edits(text, Some(&labels)), (text.into(), vec![]));
        }
    }
}

#[test]
fn preselected_spans_preserve_external_filtering_without_rerouting() {
    let text = "GPT six Astra and ten dollars";
    let selected = [Span {
        start: 4,
        end: 6,
        class: Class::Money,
    }];
    assert_eq!(apply_spans(text, &selected), "GPT six Astra and $10");
    assert_eq!(apply_spans(text, &[]), text);
    assert_eq!(apply_spans_with_edits(text, &[]), (text.into(), vec![]));
}

#[test]
fn preselected_invalid_batches_fail_closed_without_partial_edits() {
    let text = "ten dollars then twenty percent";
    let first = Span {
        start: 0,
        end: 2,
        class: Class::Money,
    };
    let last = Span {
        start: 3,
        end: 5,
        class: Class::Percent,
    };
    for selected in [
        vec![
            first,
            Span {
                start: 3,
                end: 6,
                class: Class::Percent,
            },
        ],
        vec![
            first,
            Span {
                start: 3,
                end: 3,
                class: Class::Percent,
            },
        ],
        vec![
            first,
            Span {
                start: 4,
                end: 3,
                class: Class::Percent,
            },
        ],
        vec![
            first,
            Span {
                start: 1,
                end: 5,
                class: Class::Money,
            },
        ],
        vec![first, first],
        vec![last, first],
        vec![Span {
            start: usize::MAX,
            end: usize::MAX,
            class: Class::Money,
        }],
        vec![Span {
            start: 0,
            end: usize::MAX,
            class: Class::Money,
        }],
    ] {
        assert_eq!(
            apply_spans_with_edits(text, &selected),
            (text.into(), vec![])
        );
    }
    assert_eq!(apply_spans("", &[first]), "");
}

#[test]
fn routed_and_preselected_spans_keep_protected_bytes() {
    // These proposed spans contain only renderable number words; delimiters
    // outside the span must still protect them.
    for text in [
        "\"literal ten dollars literal\"",
        "“literal ten dollars literal”",
        "'literal ten dollars literal'",
        "‘literal ten dollars literal’",
        "`literal ten dollars literal`",
        "```text\nten dollars\n```",
        "`literal ten dollars",
    ] {
        let selected = [Span {
            start: 1,
            end: 3,
            class: Class::Money,
        }];
        let mut labels = vec![BioLabel::O; text.split_whitespace().count()];
        labels[1] = BioLabel::B(Class::Money);
        labels[2] = BioLabel::I(Class::Money);
        assert_eq!(apply(text, Some(&labels)), text);
        assert_eq!(apply_spans(text, &selected), text);
    }
    for text in [
        "\"ten dollars\"",
        "“ten dollars”",
        "'ten dollars'",
        "‘ten dollars’",
        "`ten dollars`",
        "```text\nten dollars\n```",
        "`ten dollars",
        "https://example.com/ten dollars",
        "ten\n dollars",
        "ten\r\ndollars",
        "ten\rdollars",
    ] {
        let words: Vec<_> = text.split_whitespace().collect();
        let selected = [Span {
            start: 0,
            end: words.len(),
            class: Class::Money,
        }];
        let mut labels = vec![BioLabel::I(Class::Money); words.len()];
        labels[0] = BioLabel::B(Class::Money);
        assert_eq!(
            apply_with_edits(text, Some(&labels)),
            (text.into(), vec![]),
            "{text}"
        );
        assert_eq!(
            apply_spans_with_edits(text, &selected),
            (text.into(), vec![]),
            "{text}"
        );
    }

    // An unrenderable model span across a protected token can license a rule
    // wholly outside that token. Preselected spans still preserve the original.
    for prefix in [
        "www.example.com",
        "somebody@example.com",
        "C:\\ten\\dollars",
    ] {
        let text = format!("{prefix} ten dollars");
        let labels = [
            BioLabel::B(Class::Money),
            BioLabel::I(Class::Money),
            BioLabel::I(Class::Money),
        ];
        assert_eq!(apply(&text, Some(&labels)), format!("{prefix} $10"));
        assert_eq!(
            apply_spans(
                &text,
                &[Span {
                    start: 0,
                    end: 3,
                    class: Class::Money
                }]
            ),
            text
        );
    }

    // Protection still applies to special rules and orphan-backed fallbacks.
    let text = "\"GPT six Astra\" and `ten dollars` then twenty percent";
    let labels = vec![BioLabel::I(Class::Percent); text.split_whitespace().count()];
    let (output, edits) = apply_with_edits(text, Some(&labels));
    assert_eq!(output, "\"GPT six Astra\" and `ten dollars` then 20%");
    assert_eq!(
        edits,
        vec![Edit {
            start: 7,
            end: 9,
            class: Class::Percent,
            rendered: "20%".into()
        }]
    );
}

#[test]
fn accepted_edit_metadata_rebuilds_output_with_original_word_offsets() {
    let text = "  café 🦀\t ten dollars, then twenty percent; use GPT six Astra.\r\nDone  ";
    let words: Vec<_> = text.split_whitespace().collect();
    let mut labels = vec![BioLabel::O; words.len()];
    labels[2] = BioLabel::B(Class::Money);
    labels[3] = BioLabel::I(Class::Money);
    labels[5] = BioLabel::I(Class::Decimal);
    let (output, edits) = apply_with_edits(text, Some(&labels));
    assert_eq!(
        output,
        "  café 🦀\t $10, then 20%; use GPT-6 Astra.\r\nDone  "
    );
    assert_eq!(
        edits,
        vec![
            Edit {
                start: 2,
                end: 4,
                class: Class::Money,
                rendered: "$10,".into()
            },
            Edit {
                start: 5,
                end: 7,
                class: Class::Percent,
                rendered: "20%;".into()
            },
            Edit {
                start: 8,
                end: 11,
                class: Class::Model,
                rendered: "GPT-6 Astra.".into()
            },
        ]
    );
    assert_eq!(
        serde_json::from_str::<Vec<Edit>>(&serde_json::to_string(&edits).unwrap()).unwrap(),
        edits
    );

    let mut rebuilt = String::new();
    let mut cursor = 0;
    for edit in edits {
        let start = words[edit.start].as_ptr() as usize - text.as_ptr() as usize;
        let last = words[edit.end - 1];
        let end = last.as_ptr() as usize - text.as_ptr() as usize + last.len();
        rebuilt.push_str(&text[cursor..start]);
        rebuilt.push_str(&edit.rendered);
        cursor = end;
    }
    rebuilt.push_str(&text[cursor..]);
    assert_eq!(rebuilt, output);
}

#[test]
fn accepted_edit_metadata_omits_rejected_protected_and_unchanged_spans() {
    let text = "\"ten dollars\" then 16 GB then ten dollars then twenty percent";
    let selected = [
        Span {
            start: 0,
            end: 2,
            class: Class::Money,
        },
        Span {
            start: 3,
            end: 5,
            class: Class::Measure,
        },
        Span {
            start: 6,
            end: 8,
            class: Class::Decimal,
        },
        Span {
            start: 9,
            end: 11,
            class: Class::Percent,
        },
    ];
    let (output, edits) = apply_spans_with_edits(text, &selected);
    assert_eq!(
        output,
        "\"ten dollars\" then 16 GB then ten dollars then 20%"
    );
    assert_eq!(
        edits,
        vec![Edit {
            start: 9,
            end: 11,
            class: Class::Percent,
            rendered: "20%".into()
        }]
    );
    assert_eq!(apply_with_edits("16 GB", None), ("16 GB".into(), vec![]));
}
