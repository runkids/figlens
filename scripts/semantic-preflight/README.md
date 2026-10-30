# Issue #94 semantic preflight

Not product code. It measures one local embedding candidate against figlens lexical search before
#94 picks a runtime.

- `corpus.json`, `queries.json`, `thresholds.json`: original synthetic data written for this
  preflight (repository license). Frozen in a commit before the first embedding; thresholds are
  not tuned against results.
- `expansions.json`: at most 5 keyword expansions per query, written by one agent pass that saw
  only the query list, never the corpus.
- `src/main.rs`: `fig` writes the synthetic design, `rank` loads a local model and ranks layers
  by exact cosine with timings (BERT encoders with mean pooling, or a Qwen3 decoder with
  last-token pooling batched only across texts of equal token length, since candle's
  `qwen3::Model::forward` takes no padding mask; an XLM-RoBERTa encoder with CLS pooling for
  BGE-M3 dense, pre-registered in `bge-m3.json`),
  `child` is a bounded stdio embedding child, `rerank` scores
  query/text pairs with a one-label XLM-R cross-encoder (its card's pair encoding, raw logit).
- `evaluate.py`: lexical, lexical-with-expansion, semantic and fused (RRF, k=60) metrics against
  the thresholds. With an optional `<reranker-dir>`, each query's fused top 20 is scored once and
  reordered at N=10 and N=20 (registered before the first run); `reranked10` and `reranked20` are
  gated with the same thresholds and reported apart from the fused gates, with the pairs and
  scores in `<report>.rerank-pairs.json` and `<report>.rerank-scores.json`. Its stdout shows the
  child's EOF/SIGTERM results beside the gates; a cross-encoder has no child protocol yet.
- Every BERT and Qwen3 model directory holds `figlens-preflight.json` with its card's `queryPrefix`,
  `passagePrefix` and `maxTokens` (1-512, prefix and special tokens included); no model inherits
  another's prompts. Longer texts are refused, never truncated. `evaluate.py` and `reference.py`
  check it against the directory's `SHA256SUMS` like the weights. `evaluate.py` with 0 warm rounds
  reports no latency.
- `reference.py` checks the preflight against the model card's transformers code within max-abs
  1e-4 and cosine 0.99999, and the contract against the card's `sentence_bert_config.json`,
  `1_Pooling/config.json` (mean or last-token pooling; last-token pads on the left, as the card
  does) and `config_sentence_transformers.json` prompts where they exist. Every reported check
  decides its exit status. Query path: `parity-inputs.json` (synthetic) in one
  batch and one by one through `child`. Passage path: each query's top-50 `rank` scores against the corpus.

  ```sh
  python3 scripts/semantic-preflight/reference.py requests scripts/semantic-preflight/parity-inputs.json > samples/issue-94/parity/requests.jsonl
  # in the dev image, with --network none and the model directory read-only:
  semantic-preflight child <model-dir> < samples/issue-94/parity/requests.jsonl > replies.jsonl
  semantic-preflight rank <model-dir> scripts/semantic-preflight/corpus.json scripts/semantic-preflight/queries.json 0 > rank.json
  <venv>/bin/python -I scripts/semantic-preflight/reference.py <model-dir> scripts/semantic-preflight/parity-inputs.json replies.jsonl rank.json report.json
  ```

  `reference.py rerank <reranker-dir> <report>.rerank-pairs.json <report>.rerank-scores.json
  <out.json>` checks the cross-encoder's logits the same way (batched, and the first 64 alone)
  and its pair token ids. A model is selected only after its parity run passes.

  The venv comes from `requirements-reference.txt`, installed under `samples/` inside the dev
  image, never globally; only that install uses the network.

Model weights, the generated design and every report stay in ignored `samples/issue-94/`.
