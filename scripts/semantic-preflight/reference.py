"""Numerical parity of the candle model against the model card's transformers code (mask-aware mean
pooling, or last-token pooling over left padding where the card pools that way, then L2
normalization), offline on the same pinned local files. See README.md for the steps.

  reference.py requests <inputs.json>      child requests: one batch, then each text alone
  reference.py <model-dir> <inputs.json> <child-replies> <rank.json> <report.json>
      child replies cover the query path; `rank ... 0` top-50 scores per query cover the passage path.
      Exit 0 only if every vector and score is within the criteria.
  reference.py rerank <reranker-dir> <pairs.json> <rerank-scores.json> <report.json>
      evaluate.py's cross-encoder pairs and scores against the card's sequence-classification logits.
"""
import hashlib, json, math, os, sys

if sys.argv[1] == "requests":
    texts = json.load(open(sys.argv[2]))
    for request_id, batch in [("batch", texts)] + [(f"single{n}", [t]) for n, t in enumerate(texts)]:
        print(json.dumps({"id": request_id, "texts": batch}, ensure_ascii=False))
    sys.exit(0)

os.environ.update(HF_HUB_OFFLINE="1", TRANSFORMERS_OFFLINE="1")
import torch, torch.nn.functional as F, transformers, tokenizers
from transformers import AutoModel, AutoModelForSequenceClassification, AutoTokenizer, XLMRobertaTokenizerFast

HERE = os.path.dirname(os.path.abspath(__file__))
# Set before the first comparison; never tuned against results.
MAX_ABS, COSINE = 1e-4, 0.99999
RERANK = sys.argv[1] == "rerank"
if RERANK:
    model_dir, pairs_path, scores_path, out_path = sys.argv[2:6]
else:
    model_dir, inputs_path, replies_path, rank_path, out_path = sys.argv[1:6]
at = lambda name: os.path.join(model_dir, name)
fail = lambda message: sys.exit(f"reference: {message}")

# The same pinned files and contract rules as evaluate.py and the preflight's load().
sums = dict(reversed(line.split()) for line in open(at("SHA256SUMS")))
# Tokenizer side files change how transformers tokenizes, so any present must be pinned too.
optional = [n for n in ("tokenizer_config.json", "special_tokens_map.json", "sentence_bert_config.json", "1_Pooling/config.json", "config_sentence_transformers.json") if os.path.exists(at(n))]
# Every file SHA256SUMS lists must be present too, so a removed card file cannot skip its check.
for name in {"config.json", "model.safetensors", "tokenizer.json", "figlens-preflight.json", *optional, *sums}:
    if not os.path.exists(at(name)) or sums.get(name) != hashlib.sha256(open(at(name), "rb").read()).hexdigest():
        fail(f"{name} is missing from or does not match SHA256SUMS")
contract = json.load(open(at("figlens-preflight.json")))
if not all(isinstance(contract.get(k), str) for k in ("queryPrefix", "passagePrefix")):
    fail("figlens-preflight.json needs string queryPrefix and passagePrefix")
if not (isinstance(contract.get("maxTokens"), int) and 1 <= contract["maxTokens"] <= 512):
    fail("figlens-preflight.json maxTokens must be 1-512")
# The contract must say what the card's own sentence-transformers files say, where the card has them.
if os.path.exists(at("sentence_bert_config.json")) and json.load(open(at("sentence_bert_config.json")))["max_seq_length"] != contract["maxTokens"]:
    fail("maxTokens differs from the card's sentence_bert_config.json max_seq_length")
LAST_TOKEN = False
if os.path.exists(at("1_Pooling/config.json")):
    pooling = {k: v for k, v in json.load(open(at("1_Pooling/config.json"))).items() if k.startswith("pooling_mode_")}
    modes = [k for k, v in pooling.items() if v]
    if modes not in (["pooling_mode_mean_tokens"], ["pooling_mode_lasttoken"]):
        fail(f"the card pools with {pooling}, not mean tokens or the last token alone")
    LAST_TOKEN = modes == ["pooling_mode_lasttoken"]
if os.path.exists(at("config_sentence_transformers.json")):
    prompts = json.load(open(at("config_sentence_transformers.json"))).get("prompts", {})
    if prompts and (prompts.get("query", "") != contract["queryPrefix"] or prompts.get("document", "") != contract["passagePrefix"]):
        fail("the prefixes differ from the card's config_sentence_transformers.json prompts")

if os.path.exists(at("tokenizer_config.json")):
    tok = AutoTokenizer.from_pretrained(model_dir, local_files_only=True, trust_remote_code=False)
else:
    tok = XLMRobertaTokenizerFast(tokenizer_file=at("tokenizer.json"))
raw = tokenizers.Tokenizer.from_file(at("tokenizer.json"))
if LAST_TOKEN:
    # The card's code pads on the left, so the last position is every text's final token.
    tok.padding_side = "left"

if RERANK:
    # The card's transformers usage: tokenizer(queries, texts) pairs, logits as scores.
    pairs = json.load(open(pairs_path))["pairs"]
    if not pairs:
        fail("no pairs to compare")
    candle_scores = json.load(open(scores_path))["scores"]
    model = AutoModelForSequenceClassification.from_pretrained(model_dir, local_files_only=True, trust_remote_code=False, torch_dtype=torch.float32).eval()

    def score(chunk):
        q = [contract["queryPrefix"] + p[0] for p in chunk]
        t = [contract["passagePrefix"] + p[1] for p in chunk]
        enc = tok(q, t, padding=True, truncation=False, return_tensors="pt")
        if enc["attention_mask"].sum(dim=1).max().item() > contract["maxTokens"]:
            fail(f"a pair is longer than {contract['maxTokens']} tokens")
        with torch.no_grad():
            return model(**enc).logits[:, 0].tolist()

    batched = [x for n in range(0, len(pairs), 32) for x in score(pairs[n:n + 32])]
    single = [score([p])[0] for p in pairs[:64]]
    diffs = [abs(a - b) for a, b in zip(candle_scores, batched)]
    single_diffs = [abs(a - b) for a, b in zip(candle_scores, single)]
    ids_same = all(raw.encode(contract["queryPrefix"] + p[0], contract["passagePrefix"] + p[1]).ids == tok(contract["queryPrefix"] + p[0], contract["passagePrefix"] + p[1])["input_ids"] for p in pairs[:200])
    ok = ids_same and len(candle_scores) == len(pairs) and all(math.isfinite(x) for x in diffs + single_diffs) and max(diffs + single_diffs) <= MAX_ABS
    report = {"versions": {"torch": torch.__version__, "transformers": transformers.__version__, "tokenizers": tokenizers.__version__},
              "criteria": {"scoreMaxAbs": MAX_ABS}, "pass": ok, "contract": contract, "pairs": len(pairs),
              "batchedScoreMaxAbs": max(diffs), "singleScoreMaxAbs (first 64)": max(single_diffs), "pairIdsMatchFastTokenizer (first 200)": ids_same}
    json.dump(report, open(out_path, "w"), ensure_ascii=False, indent=1)
    print(json.dumps(report))
    sys.exit(0 if ok else 1)

model = AutoModel.from_pretrained(model_dir, local_files_only=True, trust_remote_code=False, torch_dtype=torch.float32).eval()

def embed(texts, prefix):
    batch = [prefix + t for t in texts]
    # Never truncated: the preflight refuses a longer text, so the reference does too.
    enc = tok(batch, padding=True, truncation=False, return_tensors="pt")
    if enc["attention_mask"].sum(dim=1).max().item() > contract["maxTokens"]:
        fail(f"an input is longer than {contract['maxTokens']} tokens")
    with torch.no_grad():
        hidden = model(**enc).last_hidden_state
    if LAST_TOKEN:
        # The card's last_token_pool.
        mask = enc["attention_mask"]
        if mask[:, -1].sum() == mask.shape[0]:
            pooled = hidden[:, -1]
        else:
            pooled = hidden[torch.arange(hidden.shape[0]), mask.sum(dim=1) - 1]
        return F.normalize(pooled, p=2, dim=1), enc
    hidden = hidden.masked_fill(~enc["attention_mask"][..., None].bool(), 0.0)
    pooled = hidden.sum(dim=1) / enc["attention_mask"].sum(dim=1)[..., None]
    return F.normalize(pooled, p=2, dim=1), enc

texts = json.load(open(inputs_path))
ref_batch, enc = embed(texts, contract["queryPrefix"])
ref_single = torch.cat([embed([t], contract["queryPrefix"])[0] for t in texts])
candle = {}
for line in open(replies_path):
    r = json.loads(line)
    if "error" in r:
        fail(f"the child refused request {r.get('id')}: {r['error']}")
    if "id" in r:
        candle[r["id"]] = torch.tensor(r["vectors"], dtype=torch.float32)
c_batch = candle["batch"]
c_single = torch.cat([candle[f"single{n}"] for n in range(len(texts))])

def compare(a, b):
    return {"maxAbs": (a - b).abs().max(dim=1).values.tolist(), "cosine": (a * b).sum(dim=1).tolist()}

within = lambda v: all(math.isfinite(x) for x in v["maxAbs"] + v["cosine"]) and max(v["maxAbs"]) <= MAX_ABS and min(v["cosine"]) >= COSINE
pairs = {"candleBatch_vs_refBatch": compare(c_batch, ref_batch), "candleSingle_vs_refSingle": compare(c_single, ref_single),
         "candleBatch_vs_candleSingle": compare(c_batch, c_single), "refBatch_vs_refSingle": compare(ref_batch, ref_single)}

# Passage path: the corpus embedded with passagePrefix in one batch, scored against each query alone.
layers = json.load(open(os.path.join(HERE, "corpus.json")))["layers"]
docs, _ = embed([l["text"] for l in layers], contract["passagePrefix"])
index = {l["concept"]: n for n, l in enumerate(layers)}
score_diffs, top_same = [], 0
rankings = json.load(open(rank_path))["rankings"]
for r in rankings:
    q, _ = embed([r["query"]], contract["queryPrefix"])
    ref_scores = (docs @ q[0]).tolist()
    score_diffs += [abs(x["score"] - ref_scores[index[x["concept"]]]) for x in r["ranked"]]
    top_same += r["ranked"][0]["concept"] == layers[max(range(len(layers)), key=ref_scores.__getitem__)]["concept"]
scores_ok = all(math.isfinite(x) for x in score_diffs) and max(score_diffs) <= MAX_ABS

ids = [raw.encode(contract["queryPrefix"] + t).ids for t in texts]
ids_same = all(ids[n] == tok(contract["queryPrefix"] + t)["input_ids"] for n, t in enumerate(texts))
rank = lambda m: (m @ m.T).argsort(dim=1, descending=True).tolist()
ranking_same = rank(c_batch) == rank(ref_batch)
# Every reported check decides the exit status: vectors, passage scores, token ids and both rankings.
ok = all(within(v) for v in pairs.values()) and scores_ok and ids_same and ranking_same and top_same == len(rankings)
report = {
    "versions": {"torch": torch.__version__, "transformers": transformers.__version__, "tokenizers": tokenizers.__version__},
    "criteria": {"maxAbs": MAX_ABS, "cosine": COSINE}, "pass": ok, "contract": contract, "texts": texts,
    "tokenIds": ids, "tokenIdsMatchFastTokenizer": ids_same,
    "paddedLengths": enc["attention_mask"].sum(dim=1).tolist(), "batchWidth": enc["input_ids"].shape[1],
    "pairs": pairs, "rankingSame": ranking_same,
    "cosineMatrixMaxAbs": ((c_batch @ c_batch.T) - (ref_batch @ ref_batch.T)).abs().max().item(),
    "passagePath": {"queries": len(rankings), "scores": len(score_diffs), "scoreMaxAbs": max(score_diffs), "top1Same": top_same},
}
json.dump(report, open(out_path, "w"), ensure_ascii=False, indent=1)
print(json.dumps({k: report[k] for k in ("pass", "tokenIdsMatchFastTokenizer", "rankingSame", "cosineMatrixMaxAbs", "passagePath")}))
for k, v in pairs.items():
    print(k, "maxAbs<=", f"{max(v['maxAbs']):.3e}", "cosine>=", f"{min(v['cosine']):.9f}")
sys.exit(0 if ok else 1)
