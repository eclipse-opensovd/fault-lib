#!/usr/bin/env bash

# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 The Contributors to Eclipse OpenSOVD (see CONTRIBUTORS)
#
# See the NOTICE file(s) distributed with this work for additional
# information regarding copyright ownership.
#
# This program and the accompanying materials are made available under the
# terms of the Apache License Version 2.0 which is available at
# https://www.apache.org/licenses/LICENSE-2.0

# Regenerate SVG diagrams from PlantUML sources.
# Requires: plantuml (https://plantuml.com/download)
#
# Usage: ./docs/puml/generate_svg.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if ! command -v plantuml &>/dev/null; then
    echo "error: plantuml not found. Install via: apt install plantuml" >&2
    exit 1
fi

for puml in "$SCRIPT_DIR"/*.puml; do
    echo "Generating SVG for $(basename "$puml")"
    plantuml -tsvg "$puml"
done

echo "Done. SVGs written to $SCRIPT_DIR/"
