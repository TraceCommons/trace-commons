#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

TRACE_COMMONS_PIPELINE_REPORT_VERSION=6 \
  "${ROOT}/scripts/operator/run-compatibility-pipeline-corpus.sh"
