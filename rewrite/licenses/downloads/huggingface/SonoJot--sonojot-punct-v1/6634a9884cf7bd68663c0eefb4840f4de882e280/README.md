---
license: cc-by-sa-4.0
language:
- en
library_name: onnx
pipeline_tag: token-classification
base_model: Horizon-Labs/punctuation-restoration-small
tags:
- punctuation
- punctuation-restoration
- speech-recognition
- asr-post-processing
- dictation
- onnx
---

# SonoJot Punctuation v1

This small, local model repairs punctuation in speech-recognizer output. SonoJot uses it after Qwen3-ASR to remove
the false sentence breaks that recognizers put at hesitation pauses ("The problem is that. I need it"). It also
places commas, question marks and the occasional semicolon.

It is a token classifier. For each word it predicts the mark that follows: none, `.`, `,`, `?` or `;`. It reads the
recognizer's own punctuation as evidence, so it keeps real sentence ends and removes pause artifacts. It never adds,
removes or changes words.

| | |
| --- | --- |
| Base model | [Horizon-Labs/punctuation-restoration-small](https://huggingface.co/Horizon-Labs/punctuation-restoration-small) (mmBERT-small, 140M parameters) |
| Runtime file | `model.int8.onnx`, 268 MB: int8 embeddings, fp32 compute (full int8 costs about 2 F1 points) |
| Speed | About 0.5 ms per word on one desktop CPU with 2 threads (about 0.17 s per 300 words when warm) |
| Training language | English |

## Files

- `model.int8.onnx`: inputs `input_ids` and `attention_mask` (int64, `[1, T]`); output `logits` (`[1, T, 5]`).
- `tokenizer.json`: Hugging Face tokenizers format.
- `punct-config.json`: labels, window and hop sizes, special-token ids, and the exact input encoding.
- `manifest.json`: byte sizes and SHA-256 of the three runtime files. SonoJot verifies these before loading.
- `pytorch/`: the fine-tuned safetensors weights and config, for further fine-tuning.

## Input encoding

1. Split the text into words (`[^\W_]+(?:['’][^\W_]+)*`) and lowercase each one, except I, I'm, I'll, I've and I'd.
2. For each word, append the tokens of `" " + word`. If the recognizer wrote a mark after the word, also append that
   mark's tokens (`.` for `.`/`!`, `?`, or `,` for `,;:` and dashes; glued marks such as `1.2` and `Next.js` don't
   count). The last word always carries a mark.
3. Wrap the sequence in CLS … SEP. Read each word's label at its last word token.
4. Long texts use windows of 120 words with a hop of 60. For each word, keep the prediction from the window where it
   is farthest from an edge.

`punct-config.json` repeats this contract.

## Results

**Test set.** 60 long real dictations (24,659 words), produced by Qwen3-ASR followed by SonoJot's local cleanup rules.
None were used in training. They were scored against an independent GPT-6.1 Sol reference made under the house style
below. Two annotators agree with each other at sentence F1 0.940 and comma F1 0.907.

| System | Sentence-boundary F1 | Mid-clause false breaks / 1,000 words | Comma F1 |
| --- | ---: | ---: | ---: |
| Recognizer output | 0.747 | 16.7 | 0.744 |
| Qwen3-4B-Instruct rewrite (guarded) | 0.859 | 4.0 | 0.764 |
| Gemma 4 E4B rewrite (guarded) | 0.881 | 2.8 | 0.785 |
| **This model** | **0.887** | **0.8** | **0.876** |

Held-out Earnings-21 calls (real recognizer output): sentence F1 0.895. Synthetic held-out documents with
dictation-rate recognizer errors: F1 0.928, comma F1 0.891.

## House style

The labels follow SonoJot's house style:

- No dashes or ellipses.
- Commas separate restarts and asides ("put it on your, your sheet").
- Semicolons appear only between two complete clauses spoken as one thought, with no conjunction between them.
- Every sentence end is followed by a capital letter. The app applies capitalization and dash removal; the model
  predicts the marks.

## Training data

Training ran for 3 epochs on an RTX 5070.

- **Earnings-21** (Rev.com, [revdotcom/speech-datasets](https://github.com/revdotcom/speech-datasets), transcripts
  CC BY-SA 4.0). 41 of the 44 calls were transcribed with Qwen3-ASR 1.7B through SonoJot's production pipeline. The
  professional reference punctuation was normalized to the house style by GPT-6.1 Sol, then projected onto the
  recognizer's words through alignment.
- **Synthetic errors.** The same references, corrupted with false periods, dropped commas and missed sentence ends,
  at rates measured on real dictation.
- **Dictation.** About 39,000 words of one consenting user's own dictation, labeled by GPT-6.1 Sol. This text is
  not distributed.

## License and attribution

The weights are released under **CC BY-SA 4.0**, because the training labels derive from the Earnings-21
transcripts (CC BY-SA 4.0, © Rev.com). The base model is Apache-2.0 (Horizon-Labs, built on jhu-clsp/mmBERT-small,
MIT). If you redistribute this model or derivatives, keep this attribution and the share-alike terms.

## Limitations

- **Language.** English only. Other languages pass through with weaker results.
- **Casing.** The model predicts punctuation, not casing; SonoJot capitalizes with rules.
- **Run-on sentences.** It tends to keep long "and then … and then" chains as one sentence (about 5 missed breaks
  per 1,000 words against the reference).
- **Domain.** It is tuned for one speaker's dictation style plus earnings-call speech.
