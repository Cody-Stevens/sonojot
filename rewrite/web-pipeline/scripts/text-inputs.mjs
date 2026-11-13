import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const rewrite = resolve(import.meta.dirname, "../..");
const read = (name) => JSON.parse(readFileSync(resolve(rewrite, name), "utf8"));
const jsonl = (name) => readFileSync(resolve(rewrite, name), "utf8").split(/\r?\n/).filter(Boolean).map(JSON.parse);

// These four public files are the complete input allowlist; no evaluation cache is read.
export function syntheticInputs() {
  const rows = [];
  const add = (source, input) => {
    if (!rows.some((row) => row.input === input)) {
      rows.push({
        id: `text-${String(rows.length + 1).padStart(3, "0")}`,
        source,
        input,
      });
    }
  };
  for (const row of read("evaluation/cleanup-v4/synthetic.json")) add("cleanup-v4/synthetic.json", row.text);
  for (const row of jsonl("evaluation/round2-review-pack/regression_additions.jsonl")) {
    if (row.input_text) add("round2-review-pack/regression_additions.jsonl", row.input_text);
  }
  for (const row of jsonl("evaluation/fixtures/smart-layout-2026.jsonl")) {
    if (row.input_text) add("fixtures/smart-layout-2026.jsonl", row.input_text);
  }
  const catalog = read("evaluation/fidelity-pack/baseline/prompt-review-catalog.json");
  const example = catalog.dictation.system.match(/Example: (.*?) ->/s)?.[1];
  if (example) add("fidelity-pack/baseline/prompt-review-catalog.json:dictation example", example);
  const authored = [
    "",
    " ",
    "\t\n  ",
    "hello",
    "um",
    "uh um",
    "um please send the folder",
    "uh we can start now",
    "you know you know it works",
    "i mean i mean we should wait",
    "sort of sort of blue",
    "kind of kind of useful",
    "the the report is ready",
    "in the in the box",
    "on the on the shelf",
    "the. The. Report is ready",
    "I I think this is fine",
    "very very good",
    "no no do not change it",
    "UM is a unit",
    "use 20 um today",
    "use 20 um uh tomorrow",
    "teh file was recieved becuase thier copy was seperate",
    "use twelve, actually, fifteen files today",
    "set twenty one, sorry, twenty two items aside",
    "we need 8, no, 9 pages",
    "use ten, or rather, eleven tickets",
    "we need negative ten, actually, twelve degrees",
    "pay five dollars today",
    "pay four thousand dollars today",
    "pay two dollars and fifty cents",
    "pay two dollars and one hundred cents",
    "pay four point two million dollars",
    "it costs three euros and five cents",
    "we saved twenty five percent",
    "the rate is zero point zero five percent",
    "allow two to three million dollars",
    "allow twelve to fifteen thousand dollars",
    "between two and three files",
    "we counted twenty one apples",
    "we counted nine apples",
    "we counted one hundred and five apples",
    "there are one thousand two hundred thirty four records",
    "there are a billion parameters",
    "use minus five point two volts",
    "we measured point five millimeters",
    "the code is one two three four",
    "we met in twenty twenty four",
    "back in ninety nine we started",
    "August of twenty four was warm",
    "winter of oh five was cold",
    "since twenty four people arrived",
    "on February twenty ninth twenty twenty four we left",
    "on February twenty ninth twenty twenty three we left",
    "the fourth of July is a holiday",
    "on the twenty first of March twenty twenty five we leave",
    "meet at two thirty p.m. on Friday",
    "meet at two thirty p.m. Then we leave",
    "we start at nine a.m. Maria brings the slides",
    "we start at nine a.m. Tuesday with the team",
    "wait until ten fifty",
    "arrive at zero fifteen",
    "arrive at twenty three fifty nine",
    "arrive at two o clock",
    "the eleventh item is blue",
    "this is the first item",
    "at the eleventh hour we stopped",
    "I am on cloud nine",
    "use version two point zero point one",
    "use V four today",
    "Python three eleven is installed",
    "use GPT six point one Soul today",
    "use Claude Opus four point seven",
    "use Eleven Labs V four narration",
    "use R and D. But ask first",
    "we held a Q and A session",
    "COVID nineteen was mentioned",
    "email sam at comet dot io today",
    "email today at comet dot io",
    "email sam at tomorrow dot example dot org",
    "visit today dot net slash tomorrow",
    "meet at two thirty p\u0085m on Friday",
    "we have ٢٣ ten dollars",
    "we have ٢٣ and twenty apples",
    "keep a\u200db unchanged",
    "\ufeffhello\u0085world",
    "email sam at comet.",
    "email Sam at Comet dot IO please",
    "visit example dot com slash docs",
    "visit https colon slash slash example dot org slash api question mark page equals digit two",
    "maybe email sam at comet dot io",
    "keep these words literal. email sam at comet dot io",
    "do not email sam at comet dot io yet",
    "keep https://example.com/a_b?q=2 unchanged",
    "send to demo@example.org now",
    "keep C:\\notes\\draft.txt unchanged",
    'keep `ten dollars` and "ten dollars" as spoken',
    "keep 'ten dollars' and pay ten dollars",
    "use the. Feature for ten dollars",
    "hello... world",
    "hello — world",
    "hello--world",
    "hello 😀 world",
    "is this ready",
    "when can we leave?",
    "bonjour café déjà vu",
    "naïve façade coöperate Straße",
    "İstanbul ist groß",
    "中文测试 日本語 한국어",
    "the résumé costs twenty dollars",
    "keep cafe\u0301 unchanged",
    "  hello   world  ",
    "hello\n\nworld",
    "one hundred\n twenty dollars",
    "don't change John's spelling",
    "let’s meet tomorrow",
    "ask J. Smith today",
    "use p.m. exactly",
    "Dr. Smith arrived. Then we left",
    "save 1,234.56 and 2.50 today",
    "version 1.2.3 is ready",
    "we have ٢٣ apples",
    "we have Ⅳ markers",
    "ß is a letter",
    "um so i think we should meet at three thirty p.m. on march fifth and uh spend about twenty five dollars",
    "try Qwen three thirty b a three b and Whisper large v three",
    "use Gemini two point five Flash Lite",
    "use GLM four point six v",
    "we tested GPT six Astra on extra high",
    "The word is `do not delete um`.",
  ];
  for (const input of authored) add("new synthetic", input);
  for (const text of [
    "Please send the report. We need twenty five copies. Can you do that?",
    "I need the red folder. Actually, use the blue folder.",
    "The budget is four thousand dollars. We meet at nine thirty AM.",
  ])
    add("new synthetic ASR-style", text.toLowerCase().replace(/[.?!]/g, ""));
  for (const count of [119, 120, 121, 180, 241, 330]) {
    const words =
      "um we need to review the next report and send twenty five dollars to the office before nine thirty tomorrow morning".split(
        " ",
      );
    add(
      `new synthetic ${count}-word window boundary`,
      Array.from({ length: count }, (_, i) => words[i % words.length]).join(" "),
    );
  }
  return rows;
}
