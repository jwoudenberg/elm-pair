#!/usr/bin/env bash

set -euxo pipefail

curl -d "p_order_id=123&p_event_time=2022-01-01%2012:04:12" localhost:8080/v1/generate-license-key
