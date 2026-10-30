# Issue #94 preflight evaluation (not product code). Run in Docker from the repository root:
#   python3 -B scripts/semantic-preflight/evaluate.py <model-dir> <warm-rounds> <report.json> [<reranker-dir>]
# With a cross-encoder, each query's fused top 20 is scored once and reranked at N=10 and N=20
# (both registered before the first run); its pairs and scores go beside the report.
# Prints counts, metrics and timings only; the report stays in ignored samples/.
import json, os, queue, signal, subprocess, sys, threading, time
from collections import defaultdict

HERE = 'scripts/semantic-preflight'
# PREFLIGHT_SET=dev reads the instruction dev split in dev/ against its own generated design.
SET = os.environ.get('PREFLIGHT_SET', '')
DATA = f'{HERE}/{SET}' if SET else HERE
FIG = f'samples/issue-94/{SET or "design"}.fig'
FIGLENS = 'target/release/figlens'
PREFLIGHT = f'{HERE}/target/release/semantic-preflight'
MODEL, WARM, REPORT = sys.argv[1], int(sys.argv[2]), sys.argv[3]
RERANKER = sys.argv[4] if len(sys.argv) > 4 else None
RERANK_N = (10, 20)
K, TOP, RRF = 10, 50, 60

load = lambda name: json.load(open(f'{DATA}/{name}'))
corpus, queries, thresholds = load('corpus.json'), load('queries.json')['queries'], json.load(open(f'{HERE}/thresholds.json'))
# The dev split has no agent expansions: it only picks the query instruction.
expansions = {e['query']: e['expanded'] for e in load('expansions.json')['expansions']} if not SET else None
concept = {}
for layer in corpus['layers']:
    concept[layer['frame']] = concept[layer['node']] = layer['concept']


def peak(args):
    """Runs args, returning stdout and the child's peak RSS in MiB (it has no children)."""
    p = subprocess.Popen(args, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    out = p.stdout.read()
    _, status, usage = os.wait4(p.pid, 0)
    if os.waitstatus_to_exitcode(status):
        sys.exit(f'failed: {args[:2]} {p.stderr.read()[-300:]!r}')
    return out, usage.ru_maxrss // 1024


def lexical(query):
    out = subprocess.run([FIGLENS, 'search', FIG, query, '--limit', str(TOP)], capture_output=True,
                         check=True, timeout=60).stdout
    ranked = []
    for r in json.loads(out)['results']:
        c = concept.get(r['ref']['nodeId'])
        if c and c not in ranked:
            ranked.append(c)
    return ranked


def fuse(*lists):
    score = defaultdict(float)
    for ranked in lists:
        for rank, c in enumerate(ranked, 1):
            score[c] += 1 / (RRF + rank)
    return [c for c, _ in sorted(score.items(), key=lambda kv: (-kv[1], kv[0]))]


def metrics(rows):
    n = len(rows)
    rr = [next((1 / i for i, c in enumerate(r['ranked'][:TOP], 1) if c in r['relevant']), 0) for r in rows]
    return {
        'n': n,
        'recallAt10': round(sum(any(c in r['relevant'] for c in r['ranked'][:K]) for r in rows) / n, 3),
        'mrr': round(sum(rr) / n, 3),
        'wrongTop1': round(sum(bool(r['ranked']) and r['ranked'][0] not in r['relevant'] for r in rows) / n, 3),
        'noResult': round(sum(not r['ranked'] for r in rows) / n, 3),
    }


# The model directory must be the pinned revision with its recorded hashes; nothing downloads.
def verified(model):
    contract = () if json.load(open(f'{model}/config.json')).get('model_type') == 'model2vec' else ('figlens-preflight.json',)
    sums = {}
    for line in open(f'{model}/SHA256SUMS'):
        digest, name = line.split()
        sums[name] = digest
    # A BERT or Qwen3 model's contract changes what is embedded, so it is pinned like the weights.
    for name in ('config.json', 'model.safetensors', 'tokenizer.json', *contract):
        if name not in sums:
            sys.exit(f'{name} is not listed in SHA256SUMS')
        got = subprocess.run(['sha256sum', f'{model}/{name}'], capture_output=True, text=True, check=True)
        if got.stdout.split()[0] != sums[name]:
            sys.exit(f'{name} does not match SHA256SUMS')
    return sums, bool(contract)


sums, has_contract = verified(MODEL)
# Tokens one BERT or Qwen3 input may hold under its contract; a static model has no limit.
LIMIT = json.load(open(f'{MODEL}/figlens-preflight.json'))['maxTokens'] if has_contract else None
rerank_sums = verified(RERANKER)[0] if RERANKER else None
# Semantic rankings and timings: one process, peak RSS from wait4.
out, rss = peak([PREFLIGHT, 'rank', MODEL, f'{DATA}/corpus.json', f'{DATA}/queries.json', str(WARM)])
sem = json.loads(out)
semantic = {r['query']: [x['concept'] for x in r['ranked']] for r in sem.pop('rankings')}

modes = defaultdict(list)
lex_ms = []
for q in queries:
    t = time.perf_counter()
    base = lexical(q['query'])
    lex_ms.append((time.perf_counter() - t) * 1000)
    runs = [('lexical', base), ('semantic', semantic[q['query']]), ('fused', fuse(base, semantic[q['query']]))]
    if expansions:
        runs.append(('lexical_agent_expansion', fuse(base, *[lexical(e) for e in expansions[q['query']]])))
    for mode, ranked in runs:
        modes[mode].append({'category': q['category'], 'relevant': q['relevant'], 'ranked': ranked})

rerank = None
if RERANKER:
    text = {layer['concept']: layer['text'] for layer in corpus['layers']}
    tops = [(q['query'], r['ranked'][:max(RERANK_N)]) for q, r in zip(queries, modes['fused'])]
    pairs = [[query, text[c]] for query, top in tops for c in top]
    with open(f'{REPORT}.rerank-pairs.json', 'w') as f:
        json.dump({'pairs': pairs}, f, ensure_ascii=False)
    out, rerank_rss = peak([PREFLIGHT, 'rerank', RERANKER, f'{REPORT}.rerank-pairs.json'])
    scored = json.loads(out)
    with open(f'{REPORT}.rerank-scores.json', 'w') as f:
        json.dump(scored, f)
    scores = iter(scored.pop('scores'))
    for (query, top), row in zip(tops, modes['fused']):
        score = {c: next(scores) for c in top}
        for n in RERANK_N:
            # The top n by score (ties keep the fused order); everything below n stays where it was.
            head = sorted(row['ranked'][:n], key=lambda c: -score[c])
            modes[f'reranked{n}'].append({**row, 'ranked': head + row['ranked'][n:]})
    # Selection still needs `reference.py rerank` to pass on these same pairs and scores.
    rerank = {'model': RERANKER, 'modelSha256': rerank_sums, **scored, 'processPeakRssMiB': rerank_rss,
              'cost': f'one pass over each query\'s top {max(RERANK_N)}; N=10 is not timed separately',
              'protocol': 'no child protocol: EOF/SIGTERM behaviour of a cross-encoder is unmeasured'}

table = {mode: {cat: metrics([r for r in rows if r['category'] == cat])
                for cat in sorted({r['category'] for r in rows})} for mode, rows in modes.items()}
gate = thresholds['per_semantic_category']
lex = table['lexical']


def gated(mode):
    m = table[mode]
    g = {cat: {'recallAt10': m[cat]['recallAt10'] >= gate['recall_at_10_min'],
               'mrr': m[cat]['mrr'] >= gate['mrr_min'],
               'wrongTop1': m[cat]['wrongTop1'] <= gate['wrong_top1_max']}
         for cat in thresholds['semantic_categories']}
    g['exact_text'] = {'recallAt10': m['exact_text']['recallAt10'] >= lex['exact_text']['recallAt10'],
                       'mrr': m['exact_text']['mrr'] >= lex['exact_text']['mrr']}
    return g


gates = gated('fused')
if rerank:
    rerank['gates'] = {f'reranked{n}': gated(f'reranked{n}') for n in RERANK_N}
    rerank['gatesPass'] = {mode: all(all(v.values()) for v in g.values()) for mode, g in rerank['gates'].items()}


# The bounded child, read by a thread so a full stdout pipe never blocks it.
def child_run(stop):
    p = subprocess.Popen([PREFLIGHT, 'child', MODEL], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                         stderr=subprocess.DEVNULL, text=True)
    lines = queue.Queue()
    threading.Thread(target=lambda: [lines.put(l) for l in p.stdout] + [lines.put(None)], daemon=True).start()
    reply = lambda: json.loads(lines.get(timeout=60))
    send = lambda r: (p.stdin.write(json.dumps(r) + '\n'), p.stdin.flush())
    try:
        t = time.perf_counter()
        ready = reply()
        ready_ms = (time.perf_counter() - t) * 1000
        t = time.perf_counter()
        send({'id': 1, 'texts': ['結帳', 'Checkout']})
        first = reply()
        request_ms = (time.perf_counter() - t) * 1000
        refused = []
        for bad in (['x'] * 65, [], ['x' * 2049]):
            send({'id': 2, 'texts': bad})
            refused.append('error' in reply())
        over = None
        if LIMIT:
            send({'id': 2, 'texts': ['x ' * (LIMIT + 1)]})
            over = 'error' in reply()
        # A full batch in flight, then the stop: EOF lets it finish and answer; SIGTERM ends it.
        # Each `long text ` is two tokens, so the batch stays within the contract's limit.
        send({'id': 3, 'texts': ['long text ' * (min(200, (LIMIT - 16) // 2) if LIMIT else 200)] * 64})
        t = time.perf_counter()
        if stop == 'eof':
            p.stdin.close()
        else:
            p.send_signal(signal.SIGTERM)
        _, status, usage = os.wait4(p.pid, 0) if wait_exit(p, 10) else (0, -1, None)
        exit_ms = (time.perf_counter() - t) * 1000
        answered = []
        while (line := lines.get(timeout=5)) is not None:
            r = json.loads(line)
            if 'vectors' in r:
                answered.append(r.get('id'))
        return {'stop': stop, 'readyMs': round(ready_ms, 1), 'dim': ready['dim'], 'requestMs': round(request_ms, 2),
                'vectors': len(first.get('vectors', [])), 'refusedOversizedEmptyLong': refused, 'refusedOverTokenLimit': over,
                'inFlightAnswered': 3 in answered, 'exitMs': round(exit_ms, 1),
                'exitStatus': status, 'peakRssMiB': usage.ru_maxrss // 1024 if usage else None}
    finally:
        if p.poll() is None:
            p.kill()
            p.wait()


def wait_exit(p, seconds):
    """True once p has exited within `seconds` (status still unreaped); else kills it."""
    end = time.time() + seconds
    while time.time() < end:
        if os.waitid(os.P_PID, p.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT):
            return True
        time.sleep(0.001)
    p.kill()
    return False


def q(xs):
    xs = sorted(xs)
    return {'n': len(xs), 'p50': round(xs[len(xs) // 2], 1), 'p95': round(xs[-(-len(xs) * 95 // 100) - 1], 1)}


report = {
    'model': MODEL, 'modelSha256': sums, 'evaluation': f'synthetic preflight{" dev split" if SET else ""}: {len(corpus["layers"])} concepts, concept-level ranks; not node-level product fusion or capacity', 'semantic': {**sem, 'processPeakRssMiB': rss}, 'lexicalProcessMs': q(lex_ms),
    'metrics': table, 'gates': gates, 'gatesPass': all(all(g.values()) for g in gates.values()),
    'rerank': rerank, 'child': [] if SET else [child_run('eof'), child_run('sigterm')], 'cpus': os.cpu_count(),
    'loadavg': open('/proc/loadavg').read().split()[:3],
}
json.dump(report, open(REPORT, 'w'), ensure_ascii=False, indent=1)
# The child's protocol results print beside the gates: an EOF timeout is a failure the gates do not show.
print(json.dumps({'gates': gates, 'gatesPass': report['gatesPass'],
                  'child': [{k: c[k] for k in ('stop', 'inFlightAnswered', 'exitStatus')} for c in report['child']],
                  'rerank': rerank and {k: rerank[k] for k in ('gates', 'gatesPass', 'protocol')}}, indent=1))
