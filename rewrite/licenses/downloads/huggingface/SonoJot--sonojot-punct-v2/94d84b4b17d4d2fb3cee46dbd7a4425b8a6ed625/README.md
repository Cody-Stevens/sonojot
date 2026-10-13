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

# SonoJot Punctuation v2

This small, local model cleans up speech-recognizer output in two ways, in one forward pass:

1. **Punctuation.** For each word it predicts the mark that follows: none, `.`, `,`, `?` or `;`. It reads the
   recognizer's own punctuation as evidence, so it keeps real sentence ends and removes the false breaks that
   recognizers put at hesitation pauses ("The problem is that. I need it").
2. **Written-form spans.** It tags spans that a person would type differently from how they are spoken: "four
   thousand dollars" (MONEY), "ten percent" (PERCENT), "twenty twenty five" (YEAR), "ten K" (MAGNITUDE). The model
   only tags. A deterministic renderer in SonoJot writes `$4,000`, `10%`, `2025` and `10K`. It also checks that the
   written form reads back as the spoken words, and otherwise keeps them.

The model never adds, removes or changes words by itself. v2 continues training from
[sonojot-punct-v1](https://huggingface.co/SonoJot/sonojot-punct-v1) and adds the span head.

| | |
| --- | --- |
| Base model | [sonojot-punct-v1](https://huggingface.co/SonoJot/sonojot-punct-v1) ← [Horizon-Labs/punctuation-restoration-small](https://huggingface.co/Horizon-Labs/punctuation-restoration-small) (mmBERT-small, 140M parameters) |
| Runtime file | `model.int8.onnx`, 268 MB: int8 embeddings, fp32 compute |
| Speed | About 0.17 s per 300 words when warm, on one desktop CPU with 2 threads (v1: about 0.16 s) |
| Training language | English |

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
  - `span_head.pt`: the span classifier (a linear layer on the same hidden state).
  - `span_labels.json`: the span labels.

## Input encoding

This is the same contract as v1:

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

The house style decides the written form; the model does not:
- Zero to nine stay as words in prose.
- Digits from 10.
- Symbols for money and percent.
- Idioms and approximate amounts ("a hundred bucks", "twenty-four seven") stay as words.

SonoJot uses the span head for the numeric classes. AI model names, versions and AMP names come from a rules
lexicon in the app, because the model tends to cut model names short. This combination measured best (below).

## Results

### Punctuation

Measured on 60 long real dictations (24,659 words), held out from training and scored against GPT-6.1 Sol references.
Two annotators agree at sentence F1 0.940 and comma F1 0.907.

| System | Sentence F1 | Mid-clause false breaks / 1,000 words | Comma F1 |
| --- | ---: | ---: | ---: |
| Recognizer output | 0.747 | 16.7 | 0.744 |
| v1 | 0.888 | 0.8 | 0.876 |
| **v2** | **0.886** | **0.9** | **0.884** |

Sentence breaks are on par with v1. Commas are slightly better: +0.008, with a paired bootstrap 95% interval of
+0.000 to +0.015.

### Written form

The score is span F1 on held-out data: a span counts only if its boundaries, class and rendered text all match.

| Test set | Rules only | Span head only | **Span head + rules (SonoJot)** |
| --- | ---: | ---: | ---: |
| 88 real dictations (GPT-6.1 Sol labels) | 0.806 | 0.798 | **0.864** |
| Earnings-21, real recognizer output | 0.622 | 0.918 | **0.914** |
| Earnings-22, spoken-form text | 0.769 | 0.949 | **0.950** |
| Synthetic dictation-style text | 0.957 | 1.000 | **0.984** |

On the dictations, SonoJot's combination has 92% span precision. It changed one of the 50 dictations that needed no
change.

## Training data

Training ran for 3 epochs on an RTX 5070, starting from v1. The loss is masked punctuation cross-entropy plus masked
span cross-entropy.

- **Earnings-21** (Rev.com, [revdotcom/speech-datasets](https://github.com/revdotcom/speech-datasets), CC BY-SA 4.0).
  The calls were transcribed with Qwen3-ASR 1.7B through SonoJot's production pipeline.
  - Punctuation labels come from the professional references, normalized to the house style by GPT-6.1 Sol.
  - Span labels come from the dataset's entity tags and spoken-form normalizations, aligned to the recognizer's
    words. A span is accepted only when the recognizer's words match the written value.
- **Earnings-22** (same source and licence). The spoken-form input is built from the references and their spoken
  normalizations. These rows train span labels only.
- **Synthetic dictation-style text.**
  - Built from 450 hand-written carrier sentences with typed slots: money, dates, times, versions, units, AI model
    names (including common recognizer mishearings) and AMP names.
  - 27% of the carriers are keep cases, where number words must stay as words.
  - The rows are checked against the held-out evaluation set for overlap (none share an 8-word sequence).
- **Dictation.** About 40,000 words of one consenting user's own dictation, labeled by GPT-6.1 Sol. This text is not
  distributed.

## License and attribution

The weights are released under **CC BY-SA 4.0**, because the training labels derive from the Earnings-21 and
Earnings-22 transcripts (CC BY-SA 4.0, © Rev.com). The base model chain is Apache-2.0 (Horizon-Labs, built on
jhu-clsp/mmBERT-small, MIT). If you redistribute this model or derivatives, keep this attribution and the
share-alike terms.

## Limitations

- **Language.** English only.
- **Casing.** The model predicts marks and spans, not casing; SonoJot capitalizes with rules.
- **Run-on sentences.** It tends to keep long "and then … and then" chains as one sentence.
- **Span coverage.**
  - Few real dictation examples exist for times, ordinals and measures.
  - AI model names change quickly, which is why SonoJot renders them from a lexicon instead of trusting the span head.
- **Domain.** It is tuned for one speaker's dictation style, plus earnings-call speech.
