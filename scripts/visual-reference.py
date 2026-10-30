#!/usr/bin/env python3
"""Offline forward parity for visual-preflight, on its exact versioned NCHW tensors.

This checks the Candle vision tower and pooled L2 output against transformers. The parent
normalizer is intentionally tested separately; this is not PIL/bicubic preprocessing parity
or a retrieval-quality benchmark. Install reference dependencies only in an ignored venv.
"""
import base64
import json
import os
from pathlib import Path
import sys
import time

os.environ.update(HF_HUB_OFFLINE="1", TRANSFORMERS_OFFLINE="1")
import numpy as np
import torch
from transformers import SiglipVisionModel
import transformers

MAX_ABS = 1e-4
MIN_COSINE = 0.99999


def main():
    model_dir, replies_path, report_path = map(Path, sys.argv[1:4])
    replies = [json.loads(line) for line in replies_path.read_text().splitlines()]
    singles = [r for r in replies if not r.get('batch')]
    batches = [r for r in replies if r.get('batch')]
    assert singles and len(batches) == 1
    torch.set_num_threads(8)
    started = time.perf_counter()
    model = SiglipVisionModel.from_pretrained(model_dir, local_files_only=True, torch_dtype=torch.float32).eval()
    load_ms = (time.perf_counter() - started) * 1000
    tests, inputs, references = [], [], []
    for result in singles:
        raw = base64.b64decode(result['inputF32le'], validate=True)
        assert len(raw) == 3 * 224 * 224 * 4
        pixels = torch.from_numpy(np.frombuffer(raw, dtype='<f4').copy()).reshape(1, 3, 224, 224)
        assert bool(torch.isfinite(pixels).all()) and float(pixels.abs().max()) <= 1
        with torch.no_grad():
            vector = torch.nn.functional.normalize(model(pixel_values=pixels).pooler_output, dim=-1)[0]
        candidate = torch.tensor(result['vectors'][0], dtype=torch.float32)
        error = float((candidate - vector).abs().max())
        cosine = float(torch.nn.functional.cosine_similarity(candidate, vector, dim=0))
        tests.append({'path': result['path'], 'maxAbs': error, 'cosine': cosine,
                      'pass': error <= MAX_ABS and cosine >= MIN_COSINE})
        inputs.append(pixels)
        references.append(vector)
    candidate = torch.tensor(batches[0]['vectors'], dtype=torch.float32)
    with torch.no_grad():
        reference = torch.nn.functional.normalize(model(pixel_values=torch.cat(inputs[:4])).pooler_output, dim=-1)
    single_reference = torch.stack(references[:4])
    for name, actual, expected in [('candleBatchVsTorch', candidate, reference),
                                    ('torchBatchVsSingle', reference, single_reference),
                                    ('candleBatchVsSingle', candidate, torch.tensor([r['vectors'][0] for r in singles[:4]]))]:
        error = float((actual - expected).abs().max())
        cosine = float(torch.nn.functional.cosine_similarity(actual, expected, dim=1).min())
        tests.append({'case': name, 'maxAbs': error, 'cosine': cosine,
                      'pass': error <= MAX_ABS and cosine >= MIN_COSINE})
    report = {'scope': 'vision forward parity on identical versioned NCHW inputs; not image-processor or retrieval-quality parity',
              'torch': torch.__version__, 'transformers': transformers.__version__, 'referenceLoadMs': load_ms,
              'maxAbsLimit': MAX_ABS, 'minCosine': MIN_COSINE, 'tests': tests, 'pass': all(t['pass'] for t in tests)}
    with report_path.open('x') as out:
        json.dump(report, out, indent=2)
        out.write('\n')
    print(json.dumps({'pass': report['pass'], 'tests': len(tests), 'worstMaxAbs': max(t['maxAbs'] for t in tests)}))
    if not report['pass']:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
