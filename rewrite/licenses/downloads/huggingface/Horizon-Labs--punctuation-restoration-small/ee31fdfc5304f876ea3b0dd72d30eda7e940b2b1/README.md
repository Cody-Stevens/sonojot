---
license: apache-2.0
language:
- multilingual
- af
- am
- ar
- az
- be
- bg
- bn
- bs
- ca
- ckb
- cs
- cy
- da
- de
- el
- en
- eo
- es
- et
- eu
- fa
- fi
- fo
- fr
- ga
- gl
- gu
- ha
- he
- hi
- hr
- hu
- hy
- id
- is
- it
- ja
- jv
- ka
- kk
- kn
- ko
- ky
- lb
- lt
- lv
- mk
- ml
- mn
- mr
- ms
- mt
- ne
- nl
- 'no'
- or
- pa
- pl
- ps
- pt
- ro
- ru
- rw
- sd
- si
- sk
- sl
- so
- sq
- sr
- su
- sv
- sw
- ta
- te
- tg
- tl
- tr
- tt
- ug
- uk
- ur
- uz
- vi
- xh
- yo
- zh
- zu
library_name: transformers
pipeline_tag: token-classification
base_model: jhu-clsp/mmBERT-small
datasets:
- HuggingFaceFW/fineweb-2
- HuggingFaceFW/fineweb
tags:
- punctuation
- punctuation-restoration
- asr
- speech-recognition
- transcription
- token-classification
- multilingual
- modernbert
- mmbert
- onnx
- transformers.js
widget:
- text: "hello how are you today i hope everything is fine"
  example_title: "English"
- text: "hola cómo estás espero que todo vaya bien en el trabajo"
  example_title: "Spanish"
- text: "ich habe heute keine zeit aber morgen können wir uns treffen"
  example_title: "German"
---

# Punctuation Restoration (small, 140M): 88 languages

Adds punctuation back to text that has none: speech-recognition (ASR) transcripts, subtitles, chat messages, voice notes,
OCR or scraped text. It predicts, after each word, one of **`.` `,` `?` `:` `-`** or nothing, in **88 languages**, with any
casing (including the all-lowercase output of many ASR systems). Built on
[mmBERT-small](https://huggingface.co/jhu-clsp/mmBERT-small), Apache-2.0, ONNX files for CPU and the browser (transformers.js)
included. [Try it in the browser](https://huggingface.co/spaces/Horizon-Labs/punctuation-restoration).
A larger, more accurate version is available as [punctuation-restoration-base](https://huggingface.co/Horizon-Labs/punctuation-restoration-base).

- Same labels as [oliverguhr/fullstop-punctuation-multilang-large](https://huggingface.co/oliverguhr/fullstop-punctuation-multilang-large)
  and [kredor/punctuate-all](https://huggingface.co/kredor/punctuate-all) (`0 . , ? - :`), so it is a drop-in for the
  [`deepmultilingualpunctuation`](https://github.com/oliverguhr/deepmultilingualpunctuation) package.
- Chinese and Japanese work too (no word segmentation needed) with the included `punctuate.py`, which writes `，。？：` there,
  and `।` (Hindi, Bengali), `، ؟` (Arabic script), `;` for questions in Greek and `። ፣` in Amharic.
- It restores punctuation only. It does not change casing or the words.
- `onnx/model_quantized.onnx` (int8 embeddings, 268 MB) predicts the same label as fp32 for 99.96% of tokens on held-out text.

## Usage

With the included helper (any language; long texts are processed in overlapping chunks):

```python
from huggingface_hub import hf_hub_download
import importlib.util, sys
spec = importlib.util.spec_from_file_location("punctuate", hf_hub_download("Horizon-Labs/punctuation-restoration-small", "punctuate.py"))
punctuate = importlib.util.module_from_spec(spec); spec.loader.exec_module(punctuate)

p = punctuate.Punctuator("Horizon-Labs/punctuation-restoration-small")
print(p("hello how are you today i hope everything is fine"))
print(p("今天天气很好我们去公园散步吧你觉得怎么样"))
```

As a drop-in for `deepmultilingualpunctuation` (space-separated languages; that package needs transformers 4.x, because it
passes `grouped_entities`, which transformers 5 removed):

```python
from deepmultilingualpunctuation import PunctuationModel
model = PunctuationModel(model="Horizon-Labs/punctuation-restoration-small")
print(model.restore_punctuation("my name is anna i live in berlin what about you"))
```

Plain transformers: the label of each word is the label of its last token (`"0"` = no mark).

```python
from transformers import pipeline
tagger = pipeline("token-classification", model="Horizon-Labs/punctuation-restoration-small")
tagger("hello how are you today")
```

transformers.js:

```js
import { pipeline } from "@huggingface/transformers";
const tagger = await pipeline("token-classification", "Horizon-Labs/punctuation-restoration-small", { dtype: "q8" });
console.log(await tagger("hello how are you today i hope everything is fine"));
```

## Evaluation

Punctuation F1: micro-averaged over the five marks (a predicted mark counts only if it is the correct mark at the correct
position), averaged over languages. Every model gets the same input: the paragraph with `. , ? : ; ! -` removed, once with its
original casing and once lowercased (as most ASR output is). Same script for every model (`code/`). The test sets were used only
for evaluation:

- **FLORES-200** devtest: professionally translated news, travel and wiki text; the consecutive sentences of one article form
  a paragraph (about 280 paragraphs per language). Columns: the 12 languages of punctuate-all, and our 88 training languages.
- **TED talks** (TED2020 via OPUS): transcripts and their translations, 5 consecutive sentences, 200 paragraphs in each of 25
  languages. Spoken style.
- **Europarl**: European Parliament proceedings, 5 consecutive sentences, 200 paragraphs in each of 12 languages.
  **punctuate-all and the fullstop models were trained on Europarl**, so this column is in-domain for them; we never trained on it.

| model | licence | languages | FLORES, 12 EU languages (cased) | (lowercased) | FLORES, our 88 languages (cased) | (lowercased) | TED talks, 25 languages (cased) | (lowercased) | Europarl, 12 languages (cased) | (lowercased) |
|---|---|---|---|---|---|---|---|---|---|---|
| **this model** (140M) | Apache-2.0 | 88 | 0.920 | 0.883 | 0.842 | 0.796 | 0.786 | 0.706 | 0.878 | 0.830 |
| [punctuation-restoration-base](https://huggingface.co/Horizon-Labs/punctuation-restoration-base) (307M) | Apache-2.0 | 88 | 0.922 | 0.891 | 0.847 | 0.804 | 0.795 | 0.722 | 0.881 | 0.839 |
| [punctuate-all](https://huggingface.co/kredor/punctuate-all) | MIT | 12 | 0.879 | 0.873 | 0.667 | 0.657 | 0.664 | 0.654 | 0.885 | 0.884 |
| [fullstop-punctuation-multilang-large](https://huggingface.co/oliverguhr/fullstop-punctuation-multilang-large) | MIT | 4 | 0.841 | 0.838 | 0.669 | 0.663 | 0.648 | 0.642 | 0.826 | 0.824 |
| [fullstop-punctuation-multilingual-base](https://huggingface.co/oliverguhr/fullstop-punctuation-multilingual-base) | MIT | 6 | 0.846 | 0.829 | 0.668 | 0.652 | 0.657 | 0.636 | 0.826 | 0.822 |
| [xlm-roberta_punctuation_fullstop_truecase](https://huggingface.co/1-800-BAD-CODE/xlm-roberta_punctuation_fullstop_truecase) | Apache-2.0 | 47 | — | 0.818 | — | 0.689 | — | 0.643 | — | 0.751 |
| [punct_cap_seg_47_language](https://huggingface.co/1-800-BAD-CODE/punct_cap_seg_47_language) | Apache-2.0 | 47 | — | 0.565 | — | 0.448 | — | 0.510 | — | 0.504 |

- The 1-800-BAD-CODE models expect lowercased input and also restore casing and split sentences; they are scored only on
  lowercased input, through the `punctuators` package, by re-reading their output against the input words. Words their output
  changed beyond casing (about 2% of words for xlm-roberta_punctuation_fullstop_truecase, 11% for punct_cap_seg_47_language) could not be
  aligned and count as "no mark", which lowers their scores somewhat.
- On Europarl, punctuate-all (trained on it) is ahead of this model, most clearly on lowercased input.

Per mark (this model; mean over languages, FLORES over our 88):

| mark | FLORES (cased) | FLORES (lowercased) | TED (cased) | TED (lowercased) |
|---|---|---|---|---|
| `.` | 0.945 | 0.882 | 0.900 | 0.788 |
| `,` | 0.725 | 0.695 | 0.684 | 0.626 |
| `?` | 0.771 | 0.640 | 0.798 | 0.709 |
| `:` | 0.442 | 0.407 | 0.571 | 0.504 |
| `-` | 0.175 | 0.168 | 0.072 | 0.062 |

<details>
<summary>Per language (FLORES)</summary>

| language | this model (cased) | this model (lowercased) | punctuate-all (cased) | xlm-roberta_punctuation_fullstop_truecase (lowercased) |
|---|---|---|---|---|
| Afrikaans | 0.846 | 0.811 | 0.738 | 0.767 |
| Albanian | 0.888 | 0.830 | 0.812 | 0.769 |
| Amharic | 0.765 | 0.765 | 0.557 | 0.721 |
| Arabic | 0.744 | 0.743 | 0.623 | 0.652 |
| Armenian | 0.606 | 0.582 | 0.540 | 0.551 |
| Azerbaijani | 0.866 | 0.818 | 0.685 | 0.753 |
| Basque | 0.849 | 0.795 | 0.576 | 0.543 |
| Belarusian | 0.935 | 0.876 | 0.823 | 0.808 |
| Bengali | 0.749 | 0.749 | 0.634 | 0.626 |
| Bosnian | 0.911 | 0.860 | 0.685 | 0.805 |
| Bulgarian | 0.928 | 0.888 | 0.881 | 0.849 |
| Catalan | 0.889 | 0.844 | 0.825 | 0.751 |
| Central Kurdish | 0.619 | 0.619 | 0.175 | 0.147 |
| Chinese (Simplified) | 0.838 | 0.838 | 0.576 | 0.771 |
| Croatian | 0.901 | 0.859 | 0.688 | 0.797 |
| Czech | 0.947 | 0.915 | 0.909 | 0.839 |
| Danish | 0.903 | 0.891 | 0.776 | 0.751 |
| Dutch | 0.926 | 0.899 | 0.899 | 0.835 |
| English | 0.872 | 0.827 | 0.810 | 0.780 |
| Esperanto | 0.903 | 0.848 | 0.779 | 0.713 |
| Estonian | 0.942 | 0.902 | 0.866 | 0.846 |
| Faroese | 0.871 | 0.832 | 0.438 | 0.466 |
| Filipino | 0.861 | 0.784 | 0.684 | 0.658 |
| Finnish | 0.942 | 0.898 | 0.859 | 0.864 |
| French | 0.911 | 0.875 | 0.861 | 0.806 |
| Galician | 0.897 | 0.851 | 0.833 | 0.752 |
| Georgian | 0.782 | 0.781 | 0.615 | 0.652 |
| German | 0.948 | 0.929 | 0.935 | 0.899 |
| Greek | 0.869 | 0.805 | 0.800 | 0.750 |
| Gujarati | 0.746 | 0.746 | 0.682 | 0.690 |
| Hausa | 0.768 | 0.592 | 0.499 | 0.512 |
| Hebrew | 0.791 | 0.793 | 0.744 | 0.611 |
| Hindi | 0.789 | 0.789 | 0.704 | 0.708 |
| Hungarian | 0.925 | 0.869 | 0.799 | 0.818 |
| Icelandic | 0.891 | 0.829 | 0.765 | 0.780 |
| Indonesian | 0.883 | 0.810 | 0.759 | 0.768 |
| Irish | 0.826 | 0.720 | 0.622 | 0.584 |
| Italian | 0.893 | 0.847 | 0.845 | 0.774 |
| Japanese | 0.909 | 0.909 | 0.446 | 0.806 |
| Javanese | 0.801 | 0.667 | 0.527 | 0.536 |
| Kannada | 0.747 | 0.746 | 0.648 | 0.709 |
| Kazakh | 0.902 | 0.842 | 0.711 | 0.752 |
| Kinyarwanda | 0.746 | 0.572 | 0.215 | 0.571 |
| Korean | 0.872 | 0.872 | 0.758 | 0.858 |
| Kyrgyz | 0.904 | 0.850 | 0.643 | 0.792 |
| Latvian | 0.945 | 0.903 | 0.845 | 0.861 |
| Lithuanian | 0.923 | 0.866 | 0.790 | 0.816 |
| Luxembourgish | 0.867 | 0.838 | 0.247 | 0.266 |
| Macedonian | 0.889 | 0.840 | 0.749 | 0.777 |
| Malay | 0.850 | 0.768 | 0.727 | 0.707 |
| Malayalam | 0.693 | 0.693 | 0.576 | 0.658 |
| Maltese | 0.836 | 0.754 | 0.188 | 0.246 |
| Marathi | 0.756 | 0.755 | 0.644 | 0.706 |
| Mongolian | 0.890 | 0.844 | 0.661 | 0.750 |
| Nepali | 0.819 | 0.818 | 0.686 | 0.686 |
| Norwegian Bokmål | 0.876 | 0.846 | 0.832 | 0.785 |
| Odia | 0.622 | 0.623 | 0.662 | 0.717 |
| Pashto | 0.730 | 0.730 | 0.522 | 0.652 |
| Persian | 0.817 | 0.817 | 0.745 | 0.678 |
| Polish | 0.929 | 0.887 | 0.898 | 0.857 |
| Portuguese | 0.900 | 0.857 | 0.848 | 0.793 |
| Punjabi | 0.769 | 0.771 | 0.681 | 0.685 |
| Romanian | 0.905 | 0.866 | 0.848 | 0.793 |
| Russian | 0.952 | 0.913 | 0.831 | 0.850 |
| Serbian | 0.896 | 0.849 | 0.711 | 0.798 |
| Sindhi | 0.722 | 0.723 | 0.587 | 0.646 |
| Sinhala | 0.722 | 0.722 | 0.624 | 0.731 |
| Slovak | 0.947 | 0.910 | 0.909 | 0.848 |
| Slovenian | 0.954 | 0.920 | 0.922 | 0.764 |
| Somali | 0.746 | 0.586 | 0.485 | 0.583 |
| Spanish | 0.880 | 0.838 | 0.835 | 0.778 |
| Sundanese | 0.788 | 0.639 | 0.472 | 0.489 |
| Swahili | 0.823 | 0.721 | 0.660 | 0.671 |
| Swedish | 0.878 | 0.859 | 0.837 | 0.796 |
| Tajik | 0.917 | 0.879 | 0.172 | 0.220 |
| Tamil | 0.719 | 0.721 | 0.599 | 0.688 |
| Tatar | 0.863 | 0.797 | 0.209 | 0.293 |
| Telugu | 0.723 | 0.723 | 0.628 | 0.695 |
| Turkish | 0.841 | 0.809 | 0.678 | 0.753 |
| Ukrainian | 0.927 | 0.894 | 0.846 | 0.848 |
| Urdu | 0.774 | 0.775 | 0.667 | 0.701 |
| Uyghur | 0.767 | 0.767 | 0.644 | 0.757 |
| Uzbek | 0.896 | 0.840 | 0.646 | 0.733 |
| Vietnamese | 0.851 | 0.776 | 0.718 | 0.668 |
| Welsh | 0.844 | 0.763 | 0.669 | 0.648 |
| Xhosa | 0.793 | 0.638 | 0.380 | 0.387 |
| Yoruba | 0.711 | 0.455 | 0.215 | 0.258 |
| Zulu | 0.804 | 0.650 | 0.360 | 0.365 |

</details>

## Training

- **Data**: 1,168,626 text windows (119M words) in 88 languages from
  [FineWeb-2](https://huggingface.co/datasets/HuggingFaceFW/fineweb-2) and [FineWeb](https://huggingface.co/datasets/HuggingFaceFW/fineweb)
  (ODC-BY). The labels come from the text's own punctuation: snippets that end in a full stop, are mostly letters and are not
  lists; 1-4 snippets per window (up to 230 words), 30% cut mid-sentence. 70% of the inputs were lowercased.
- **Spoken-style text** (v1.1): 49,144 windows of edited transcripts of everyday speech (talks, podcasts, interviews,
  meetings, lectures, vlogs, phone calls ...) in 69 languages, written by Qwen3.8-27B (Apache-2.0) for this purpose; transcripts
  that shared a sentence with any test set were removed.
- **Normalisation**: `!`, `;`, `…` and script full stops (`。 । ። ۔` ...) count as `.`; `，、،` as `,`; `？؟` (and `;` in Greek) as `?`;
  a free-standing dash as `-`. A `.` before a lowercase word (abbreviations) counts as no mark.
- **Model**: mmBERT-small token classification, one label per word on its last token, 2 epochs, learning rate 5e-5.
  Checkpoint and number of epochs (2 over 1) chosen on held-out web text, not on the test sets.
- Code: `code/` in this repository.

## Changelog

- **v1.1 (2026-10-09)**: + 49k windows of synthetic spoken-style transcripts (69 languages). TED talks (spoken, 25 languages), two training seeds: punctuation F1 .772 -> .786 (cased) and .688 -> .706 (lowercased). FLORES (written text, 88 languages) unchanged (.841 / .795).
- **v1.0 (2026-10-03)**: first release.

## Limitations

- Only `. , ? : -` are predicted. Exclamation marks, semicolons, quotes, brackets and Spanish inverted marks are not restored.
  `:` and especially `-` (a free-standing dash) are much less reliable than `.` `,` `?` (see the per-mark table).
- Accuracy on lowercased text is lower than on cased text, because capital letters reveal sentence starts.
- Trained mostly on written web text, plus synthetic spoken-style transcripts. Disfluent speech (fillers, false starts, repetitions) is
  harder, and punctuation conventions vary by writer. Thai, Lao, Khmer, Burmese and Tibetan are not supported.
- Low-resource languages and languages outside the 88 score lower (see the per-language table).
