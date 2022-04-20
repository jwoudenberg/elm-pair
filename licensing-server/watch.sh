#!/usr/bin/env nix-shell
#! nix-shell -i bash -p entr
# shellcheck shell=bash

export ELM_PAIR_LICENSING_SERVER_PORT="8080"
# These are test credentials, so don't worry / get your hopes up!

export ELM_PAIR_LICENSING_SERVER_SIGNING_KEY='-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEICmsW6EitQMuynQs3FwoATwc/VyQJJk3np1xPvlSlJR3
-----END PRIVATE KEY-----'
export ELM_PAIR_LICENSING_SERVER_PADDLE_KEY='-----BEGIN PUBLIC KEY-----
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAyAk9bR43XI1XXnDXsFHh
NPM7Q9tx9oJ7uE5ZQa+DhXswSqHOQIeLMAL1hPH3aafNSuLR614qLcR+Lp2cZFyx
+Qp0NynJn1TO2BIUoBePhBL//TMNrYAv2D3iA/YDt+y9NvVtFGpNBeUFn34WpeQX
DjFxEKl8Qfe5ndvY1oVltBKvE2keUlpqNjd9roUUlWBB7g8qFk76R/lSQv9nXQxS
Uib9P30h8MIiAYTNqkTmhMDfmKChsjcAHZIBJwRqmX165efdkI7GNEjDMiM3fnUg
+gn/kQ8zm/iGzrZEo1BBsMeHTb0Md05mIP1zq3upPxgRlnZjzBCV8xTBrFF5jfdv
IQIDAQAB
-----END PUBLIC KEY-----'
export ELM_PAIR_LICENSING_SERVER_HEALTH_CHECKS_IO_UUID='3546eacf-2698-4f5a-bfbb-09a9e0372313'

# Watch source files and recompile when any change.
git ls-files | entr -ccr -s "go build && go run ."
