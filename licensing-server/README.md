# Elm-pair licensing server

This server provides a webhook that Paddle (the payment provider) calls to obtain a license key after an order is made.

The relevant paddle documentation for this process can be found here:
https://developer.paddle.com/webhook-reference/ZG9jOjI1MzUzOTky-fulfillment-webhook

The license key is a signed token consisting of a key version, the order id, and the order time. This allows Elm-pair to validate a license key without needing to make any HTTP requests.

This server is written in Go, because Go's standard library contains everything necessary for its implementation.
