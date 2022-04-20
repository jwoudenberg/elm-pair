package main

import (
	"os"
	"testing"
	"time"
)

func TestGenerateLicenseKey(t *testing.T) {
	signingKeyPem, err := os.ReadFile("testing_private_signing_key.pem")
	if err != nil {
		t.Errorf("Failed to read private key from file: %s", err)
	}

	signingKey, err := parsePrivateKey(signingKeyPem)
	if err != nil {
		t.Errorf("Failed to read private key: %s", err)
	}

	orderId := "123"
	orderTime := time.Unix(1334910171, 0)
	licenseKey, err := generateLicenseKey(signingKey, orderId, orderTime)
	if err != nil {
		t.Errorf("Failed to read generate license key: %s", err)
	}

	expectedKey := "1-123-1334910171-ttB5QH9dWQjx2bN04PVFnqaAa3Ne7DzEN53S17rMD8BzMPGfZzoPc53HsZXyfzwl1CibJBMW03U0hGXEyyteCw=="
	if licenseKey != expectedKey {
		t.Errorf("got license key %s, expected %s", licenseKey, expectedKey)
	}
}
