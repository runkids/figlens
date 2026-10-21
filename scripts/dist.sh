#!/bin/sh
set -eu

cd "$(dirname "$0")/.."
docker compose run --build --rm dist scripts/build-dist.sh
