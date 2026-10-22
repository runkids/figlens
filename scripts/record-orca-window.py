#!/usr/bin/env python3
"""Record one verified Orca window through its permissioned capture helper."""

import argparse
import json
from pathlib import Path
import shutil
import subprocess
import time


parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("pid", type=int)
parser.add_argument("window_id", type=int)
parser.add_argument("output_dir", type=Path)
parser.add_argument("--seconds", type=float, default=360)
args = parser.parse_args()
args.output_dir.mkdir(parents=True)
assert not list(args.output_dir.iterdir()), "Use an empty recording directory"
frames = []
started = time.monotonic()
print(f"Recording Orca window {args.window_id}; create {args.output_dir / 'stop'} to finish", flush=True)

# shortcut: periodic window snapshots capture terminal work; use a native recorder for smooth motion.
while time.monotonic() - started < args.seconds and not (args.output_dir / "stop").exists():
    tick = time.monotonic()
    run = subprocess.run(
        ["orca", "computer", "get-app-state", "--app", f"pid:{args.pid}",
         "--window-id", str(args.window_id), "--json"],
        capture_output=True, text=True, check=True, timeout=15,
    )
    result = json.loads(run.stdout)["result"]
    snapshot = result["snapshot"]
    assert snapshot["window"]["id"] == args.window_id, "Capture target changed"
    assert snapshot["app"]["bundleId"] == "com.stablyai.orca", "Capture is not Orca"
    assert result["screenshotStatus"]["metadata"]["windowId"] == args.window_id
    png = Path(result["screenshot"]["path"])
    assert png.read_bytes().startswith(b"\x89PNG\r\n\x1a\n"), "Invalid capture"
    name = f"frame-{len(frames):05d}.png"
    shutil.copyfile(png, args.output_dir / name)
    frames.append({"file": name, "seconds": time.monotonic() - started})
    if len(frames) % 20 == 0:
        print(f"Captured {len(frames)} frames in {frames[-1]['seconds']:.1f}s", flush=True)
    time.sleep(max(0, 0.35 - (time.monotonic() - tick)))

elapsed = time.monotonic() - started
assert len(frames) >= 2, "Not enough frames to record"
receipt = {"window_id": args.window_id, "app_pid": args.pid, "elapsed_seconds": elapsed,
           "capture": "Real Orca window snapshots; original timing preserved", "frames": frames}
(args.output_dir / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
concat = []
for i, frame in enumerate(frames):
    end = frames[i + 1]["seconds"] if i + 1 < len(frames) else elapsed
    concat.extend([f"file '{frame['file']}'", f"duration {max(0.04, end - frame['seconds']):.6f}"])
concat.append(f"file '{frames[-1]['file']}'")
plan = args.output_dir / "frames.ffconcat"
plan.write_text("\n".join(concat) + "\n")
video = args.output_dir / "recording.mp4"
subprocess.run(
    ["ffmpeg", "-hide_banner", "-loglevel", "error", "-f", "concat", "-safe", "0",
     "-i", str(plan), "-vf", "fps=30,format=yuv420p", "-c:v", "libx264", "-preset", "fast",
     "-crf", "20", "-movflags", "+faststart", str(video)], check=True,
)
print(f"Saved {video} ({elapsed:.1f}s, {len(frames)} real captures)", flush=True)
