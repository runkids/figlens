# #94 instruction dev split

Picks the Qwen3-Embedding query instruction without touching the frozen suite one level up.
The Qwen3-Embedding card recommends a task-specific English instruction; `instructions.json`
holds the three candidates and the selection rule, all written before the first dev run.

- `corpus.json`, `queries.json`: original synthetic data in other domains (travel, calendar,
  media, health); no concept or query is shared with `../corpus.json`/`../queries.json`.
  50 queries, 10 per category, gates from `../thresholds.json`.
- The chosen instruction is committed before the frozen suite runs once with it; the frozen
  corpus, queries and thresholds stay unchanged.

```sh
PREFLIGHT_SET=dev python3 -B scripts/semantic-preflight/evaluate.py <model-dir> 0 <report.json>
```
