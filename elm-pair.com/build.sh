#!/usr/bin/env bash

set -euxo pipefail

./changelog-to-news.py
zola build
