---
license: cc-by-sa-4.0
language:
- en
library_name: onnx
pipeline_tag: token-classification
base_model: SonoJot/sonojot-punct-v1
tags:
- punctuation
- punctuation-restoration
- inverse-text-normalization
- speech-recognition
- asr-post-processing
- dictation
- onnx
---

# SonoJot Punctuation v3

v3 has the same architecture, files and input contract as
[sonojot-punct-v2](https://huggingface.co/SonoJot/sonojot-punct-v2). It is retrained with about twice as much real
dictation, and dictation inputs are now built exactly as SonoJot builds them at run time.

In one forward pass, it cleans up speech-recognizer output in two ways:

1. **Punctuation.** For each word it predicts the mark that follows: none, `.`, `,`, `?` or `;`. It reads the
   recognizer's own punctuation as evidence, so it keeps real sentence ends and removes the false breaks that
   recognizers put at hesitation pauses ("The problem is that. I need it").
2. **Written-form spans.** It tags spans that a person would type differently from how they are spoken: "four
   thousand dollars" (MONEY), "ten percent" (PERCENT), "twenty twenty five" (YEAR), "ten K" (MAGNITUDE). The model
   only tags. A deterministic renderer in SonoJot writes `$4,000`, `10%`, `2025` and `10K`. It also checks that the
   written form reads back as the spoken words, and otherwise keeps them.

The model never adds, removes or changes words by itself.

| | |
| --- | --- |
| Base model | [sonojot-punct-v1](https://huggingface.co/SonoJot/sonojot-punct-v1) ← [Horizon-Labs/punctuation-restoration-small](https://huggingface.co/Horizon-Labs/punctuation-restoration-small) (mmBERT-small, 140M parameters) |
| Runtime file | `model.int8.onnx`, 268 MB: int8 embeddings, fp32 compute |
| Speed | About 0.14 s per 300 words when warm, on one desktop CPU with 2 threads (release build, Ryzen 9 9900X; v2 measured the same way: 0.14 s) |
| Training language | English |

## What changed from v2

- **More real dictation.** 310 more dictations (about 56,000 words) from one consenting user, labeled by GPT-6.1
  Sol under the same house style. That roughly doubles the in-domain data.
- **Correct inputs.** Each dictation input is rebuilt from the raw recognizer text with SonoJot's rules alone. Since
  v2 shipped, the saved text already carries v2's punctuation, so it can't serve as input.
- **Training.** v3 retrains from v1 with v2's recipe on v2's data plus the new rows.
  - The recipe: 3 epochs, AdamW at 4e-5, effective batch 16, masked punctuation plus span loss, and new dictation
    windows weighted 5x.
  - Training ran three times with different seeds. The released run is the one with the best development score,
    not the best test score.

## Files

- `model.int8.onnx`:
  - Inputs: `input_ids` and `attention_mask` (int64, `[batch, T]`).
  - Outputs: `logits` (`[batch, T, 5]`, punctuation) and `span_logits` (`[batch, T, 27]`, BIO span labels).
- `tokenizer.json`: Hugging Face tokenizers format.
- `punct-config.json`: punctuation labels, span labels, window and hop sizes, special-token ids and the exact
  input encoding.
- `manifest.json`: byte sizes and SHA-256 of the three runtime files. SonoJot verifies these before loading.
- `pytorch/`: the fine-tuned weights for further fine-tuning:
  - `model.safetensors` and `config.json`: the encoder and the punctuation head.
  - `span_head.pt`: the span classifier.
  - `span_labels.json`: the span labels.

## Input encoding

This is the same contract as v1 and v2:

1. Split the text into words (`[^\W_]+(?:['’][^\W_]+)*`). Lowercase each word, except I, I'm, I'll, I've and I'd.
2. For each word, append the tokens of `" " + word`.
3. If the recognizer wrote a mark after the word, also append that mark's tokens: `.` for `.` and `!`, `?` for `?`,
   or `,` for `,`, `;`, `:` and dashes. The last word always carries a mark.
4. Wrap the sequence in CLS … SEP.
5. Read both heads at each word's last word token. Take the argmax of each head independently.
6. Long texts use windows of 120 words with a hop of 60. For each word, keep the prediction from the window where it
   is farthest from an edge.

## Span classes

`CARDINAL, DECIMAL, MONEY, PERCENT, MAGNITUDE (10K, 4K, 10x), YEAR, DATE, TIME, ORDINAL, MEASURE (number + technical
unit), VERSION, MODEL (AI model names), AMP (R&D-style names)`, in BIO form (27 labels).

The house style is unchanged from v2. SonoJot uses the span head for the numeric classes. AI model names, versions
and AMP names come from a rules lexicon in the app.

## Results

### Punctuation

The test set is 88 real dictations (29,177 words), held out from training. Inputs are rules-only recognizer text, as
SonoJot produces at run time.
- **Reference:** GPT-6.1 Sol labeled the test set. An Opus 5.5 annotator then labeled it blind, and separate judges
  settled every disagreement under the house style.
- **Rows:** the v2 and v3 rows score the published or released models. "Mean of 3 seeds" trains each recipe three
  times.

| System | Sentence F1 | False breaks / 1,000 words | Missed breaks / 1,000 words | Comma F1 |
| --- | ---: | ---: | ---: | ---: |
| v2 (published) | 0.882 | 5.9 | 5.4 | 0.881 |
| v2 recipe, mean of 3 seeds | 0.883 | 6.3 | 5.0 | 0.878 |
| v3 recipe, mean of 3 seeds | 0.894 | 5.9 | 4.5 | 0.884 |
| **v3 (released seed)** | **0.897** | **6.0** | **4.1** | **0.884** |

- **Ranking:** every v3 training run beat every v2 training run on sentence F1 and comma F1.
- **Capitalization errors** fell from 9.9 to 9.2 per 1,000 words (seed means).
- **Ceiling:** two careful annotators agree at about 0.94 sentence F1 and 0.92 comma F1. That is roughly the most any
  model can score against a single reference.

### Written form

The score is span F1 for SonoJot's combination of the span head and rules, on held-out data. A span counts only if
its boundaries, class and rendered text all match.

| Test set | v2 | **v3** |
| --- | ---: | ---: |
| 88 real dictations | 0.870 | **0.848** |
| Earnings-21, real recognizer output | 0.912 | **0.910** |
| Earnings-22, spoken-form text | 0.947 | **0.945** |
| Synthetic dictation-style text | 0.984 | **0.983** |

- **Overall:** written form is about the same as v2. Another v3 training run scored 0.872 on the dictations, so the
  difference is within seed variance on 113 gold spans.
- **Challenge set:** on the 763-case written-form challenge, SonoJot's combination gets 702 exact with v3 against 694
  with v2. The span head alone gets 663 against 639.

## Training data

The data is v2's (see the [v2 card](https://huggingface.co/SonoJot/sonojot-punct-v2)) plus the new dictations:

- **Earnings-21 and Earnings-22** (Rev.com, [revdotcom/speech-datasets](https://github.com/revdotcom/speech-datasets),
  CC BY-SA 4.0). The calls were transcribed with Qwen3-ASR 1.7B through SonoJot's production pipeline. Labels come
  from the professional references, normalized to the house style by GPT-6.1 Sol.
- **Synthetic dictation-style text** for written-form spans, unchanged from v2.
- **Dictation.** About 95,000 words of one consenting user's own dictation, labeled by GPT-6.1 Sol. This text is not
  distributed.

## License and attribution

The weights are released under **CC BY-SA 4.0**, because the training labels derive from the Earnings-21 and
Earnings-22 transcripts (CC BY-SA 4.0, © Rev.com). The base model chain is Apache-2.0 (Horizon-Labs, built on
jhu-clsp/mmBERT-small, MIT). If you redistribute this model or derivatives, keep this attribution and the
share-alike terms.

## Limitations

- **Language.** English only.
- **Casing.** The model predicts marks and spans, not casing; SonoJot capitalizes with rules.
- **Run-on sentences.** It still tends to keep long "and then … and then" chains as one sentence.
- **Span coverage.** Few real examples exist for times, ordinals and measures. AI model names come from the app's
  lexicon.
- **Domain.** It is tuned for one speaker's dictation style, plus earnings-call speech.
