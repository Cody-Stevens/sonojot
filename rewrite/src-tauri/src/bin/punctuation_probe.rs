//! Opt-in per-word parity and text-boundary latency checks through the production
//! punctuation worker. Never opens the application store or capture devices.
use serde::Deserialize;
use serde_json::json;
use sonojot_core::{
    domain::{MemoryMode, ModelRetention, Settings},
    punctuation, resource_manager,
    written_form::BioLabel,
};
use std::{
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, BufWriter, Write},
    path::Path,
    sync::{atomic::AtomicBool, Arc},
    time::Instant,
};

#[derive(Deserialize)]
struct Case {
    id: String,
    input_text: String,
    #[serde(default)]
    labels: Option<Vec<String>>,
    #[serde(default)]
    spans: Option<Vec<BioLabel>>,
}

fn differences<T: PartialEq>(
    actual: &[T],
    expected: Option<&[T]>,
) -> Result<Option<usize>, String> {
    let Some(expected) = expected else {
        return Ok(None);
    };
    if actual.len() != expected.len() {
        return Err(format!(
            "Label count {} != {}",
            actual.len(),
            expected.len()
        ));
    }
    Ok(Some(
        actual.iter().zip(expected).filter(|(a, b)| a != b).count(),
    ))
}

fn span_differences(
    actual: Option<&[BioLabel]>,
    expected: Option<&[BioLabel]>,
) -> Result<Option<usize>, String> {
    match (actual, expected) {
        (Some(actual), expected) => differences(actual, expected),
        (None, Some(_)) => Err("Expected span labels, but the worker returned no span head".into()),
        (None, None) => Ok(None),
    }
}

fn configure(retention: ModelRetention) {
    resource_manager::configure(&Settings {
        model_retention: retention,
        memory_mode: MemoryMode::Sequential,
        ..Settings::default()
    });
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 5 || !matches!(args[0].as_str(), "labels" | "benchmark") {
        return Err("Use punctuation_probe labels|benchmark MODELS_ROOT PACKAGE_DIR INPUT.jsonl OUTPUT.jsonl".into());
    }
    let root = Path::new(&args[1]);
    std::env::set_var("SONOJOT_PUNCT_MODEL_DIR", &args[2]);
    let input = BufReader::new(File::open(&args[3])?);
    let mut output = BufWriter::new(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&args[4])?,
    );
    let cancel = Arc::new(AtomicBool::new(false));
    configure(ModelRetention::OnExit);
    let mut total_differences = 0;
    let mut total_span_differences = 0;
    let mut checked_labels = 0;
    let mut checked_spans = 0;
    let mut span_head_rows = 0;
    let mut total_words = 0;
    let mut rows = 0;
    for line in input.lines() {
        let case: Case = serde_json::from_str(&line?)?;
        if args[0] == "labels" {
            let (labels, spans) =
                punctuation::evaluation_prediction(root, &case.input_text, &cancel)?;
            let differences = differences(&labels, case.labels.as_deref())?;
            let span_differences = span_differences(spans.as_deref(), case.spans.as_deref())?;
            total_differences += differences.unwrap_or(0);
            total_span_differences += span_differences.unwrap_or(0);
            if differences.is_some() {
                checked_labels += labels.len();
            }
            if span_differences.is_some() {
                checked_spans += labels.len();
            }
            if spans.is_some() {
                span_head_rows += 1;
            }
            total_words += labels.len();
            rows += 1;
            writeln!(
                output,
                "{}",
                json!({"id":case.id, "labels":labels, "spans":spans,
                    "differences":differences, "span_differences":span_differences})
            )?;
        } else {
            configure(ModelRetention::OnExit);
            punctuation::release_cached();
            let start = Instant::now();
            let cold = punctuation::run(root, &case.input_text, &[], &cancel);
            let cold_wall_ms = start.elapsed().as_secs_f64() * 1000.0;
            if cold.stage != "punctuation_model" {
                return Err("Cold model fell back to rules".into());
            }
            // Validate the actual production tokenizer's word count. This
            // diagnostic call is outside the measured cold interval.
            let (labels, spans) =
                punctuation::evaluation_prediction(root, &case.input_text, &cancel)?;
            let word_count = labels.len();
            if word_count != 300 {
                return Err(
                    format!("Benchmark requires 300 regex words; received {word_count}").into(),
                );
            }
            let span_head = spans.is_some();
            let evidence = punctuation::evaluation_wait_ready(root, &cancel)?;
            writeln!(
                output,
                "{}",
                json!({"id":case.id, "phase":"cold", "wall_ms":cold_wall_ms,
                "stage_ms":cold.elapsed_ms, "stage":cold.stage, "words":word_count,
                "span_head":span_head, "parent_pid":std::process::id(), "runtime":evidence})
            )?;
            punctuation::release_cached();
            // Capture starts the same prewarm guard. Waiting here models enough
            // spoken audio to finish loading before Stop supplies the text.
            for retention in [ModelRetention::OnExit, ModelRetention::Immediately] {
                configure(retention);
                let start = Instant::now();
                let prewarm = punctuation::prewarm(root, cancel.clone());
                let evidence = punctuation::evaluation_wait_ready(root, &cancel)?;
                let worker_pid = evidence.pid;
                let prewarm_ms = start.elapsed().as_secs_f64() * 1000.0;
                writeln!(
                    output,
                    "{}",
                    json!({"id":case.id, "phase":"prewarm", "retention":retention,
                    "wall_ms":prewarm_ms, "words":word_count, "span_head":span_head,
                    "parent_pid":std::process::id(), "runtime":evidence})
                )?;
                for pass in 0..10 {
                    let start = Instant::now();
                    let warm = punctuation::run(root, &case.input_text, &[], &cancel);
                    let wall_ms = start.elapsed().as_secs_f64() * 1000.0;
                    if warm.stage != "punctuation_model" {
                        return Err("Warm model fell back to rules".into());
                    }
                    if warm.text != cold.text {
                        return Err("Warm output differs from cold output".into());
                    }
                    if punctuation::evaluation_wait_ready(root, &cancel)?.pid != worker_pid {
                        return Err("Warm benchmark replaced its prewarmed worker".into());
                    }
                    writeln!(
                        output,
                        "{}",
                        json!({"id":case.id, "phase":"warm", "retention":retention,
                        "pass":pass, "wall_ms":wall_ms, "stage_ms":warm.elapsed_ms, "stage":warm.stage,
                        "words":word_count, "span_head":span_head})
                    )?;
                }
                drop(prewarm);
                let remained_cached = punctuation::release_cached();
                if retention == ModelRetention::Immediately && remained_cached {
                    return Err("Immediate retention left the helper cached".into());
                }
            }
        }
        output.flush()?;
    }
    if args[0] == "labels" {
        let evidence = punctuation::evaluation_wait_ready(root, &cancel)?;
        let summary = json!({"rows":rows, "words":total_words, "differing_labels":total_differences,
            "differing_spans":total_span_differences, "checked_labels":checked_labels,
            "checked_spans":checked_spans, "span_head_rows":span_head_rows,
            "parent_pid":std::process::id(), "runtime":evidence});
        println!("{summary}");
        writeln!(output, "{summary}")?;
        output.flush()?;
        if total_differences != 0 || total_span_differences != 0 {
            return Err("Label parity failed".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sonojot_core::written_form::Class;

    #[test]
    fn parity_checks_both_heads_and_refuses_missing_or_misaligned_spans() {
        assert_eq!(
            differences(&["0", "."], Some(&["0", ","][..])).unwrap(),
            Some(1)
        );
        assert!(differences(&["0"], Some(&["0", "."][..])).is_err());
        let spans = [BioLabel::B(Class::Money), BioLabel::I(Class::Money)];
        assert_eq!(
            span_differences(Some(&spans), Some(&spans)).unwrap(),
            Some(0)
        );
        assert!(span_differences(None, Some(&spans)).is_err());
        assert!(span_differences(Some(&spans), Some(&spans[..1])).is_err());
        assert_eq!(span_differences(None, None).unwrap(), None);
        assert_eq!(span_differences(Some(&spans), None).unwrap(), None);
    }

    #[test]
    fn legacy_parity_rows_need_no_span_expectations() {
        let row: Case = serde_json::from_value(json!({
            "id":"public-v1", "input_text":"hello", "labels":["."]
        }))
        .unwrap();
        assert_eq!(row.labels.unwrap(), ["."]);
        assert!(row.spans.is_none());
        let row: Case = serde_json::from_value(json!({
            "id":"public-v2", "input_text":"ten dollars", "spans":["B-MONEY", "I-MONEY"]
        }))
        .unwrap();
        assert!(row.labels.is_none());
        assert_eq!(
            row.spans.unwrap(),
            [BioLabel::B(Class::Money), BioLabel::I(Class::Money)]
        );
    }
}

fn main() {
    punctuation::dispatch_worker();
    let result = run();
    punctuation::release_cached();
    if let Err(error) = result {
        eprintln!("punctuation_probe: {error}");
        std::process::exit(1);
    }
}
