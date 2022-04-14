package main

import (
	"bytes"
	"crypto"
	"crypto/ecdsa"
	"crypto/rand"
	"crypto/rsa"
	"crypto/sha1"
	"crypto/sha256"
	"crypto/x509"
	"encoding/base64"
	"encoding/pem"
	"errors"
	"fmt"
	"log"
	"net/http"
	"net/url"
	"os"
	"sort"
	"strconv"
	"time"
)

func main() {
	pkey, err := readPrivateKeyFromEnv()
	if err != nil {
		log.Fatal(err)
		return
	}

	paddleKey, err := readPaddleKeyFromEnv()
	if err != nil {
		log.Fatal(err)
		return
	}

	port := os.Getenv("ELM_PAIR_LICENSING_SERVER_PORT")
	if port == "" {
		log.Fatal("not set: ELM_PAIR_LICENSING_SERVER_PORT")
		return
	}

	http.HandleFunc("/v1/generate-license-key",
		func(w http.ResponseWriter, r *http.Request) {
			handler(pkey, paddleKey, w, r)
		})
	log.Fatal(http.ListenAndServe(fmt.Sprintf(":%s", port), nil))
}

func handler(
	pkey *ecdsa.PrivateKey,
	paddleKey *rsa.PublicKey,
	w http.ResponseWriter,
	r *http.Request,
) {
	r.Body = http.MaxBytesReader(w, r.Body, 1024*1024)
	if err := r.ParseForm(); err != nil {
		writeErrorResponse(w, "failed to parse formdata")
		return
	}

	err := verifyPaddleSig(r.Form, paddleKey)
	if err != nil {
		writeErrorResponse(w, "invalid paddle signature")
		return
	}

	orderId := r.FormValue("p_order_id")
	if orderId == "" {
		writeErrorResponse(w, "missing p_order_id field")
		return
	}

	eventTimeStr := r.FormValue("p_event_time")
	if eventTimeStr == "" {
		writeErrorResponse(w, "missing p_event_time field")
		return
	}

	layout := "2006-01-02 15:04:05"
	eventTime, err := time.Parse(layout, eventTimeStr)
	if err != nil {
		writeErrorResponse(w, fmt.Sprintf("failed to parse p_event_time %s: %s", eventTimeStr, err))
		return
	}

	licenseKey, err := generateLicenseKey(pkey, orderId, eventTime)
	if err != nil {
		writeErrorResponse(w, fmt.Sprintf("failed to generate license key: %s", err))
	}

	fmt.Fprintf(w, "%s", licenseKey)
}

func writeErrorResponse(w http.ResponseWriter, err string) {
	http.Error(w, err, http.StatusBadRequest)
	return
}

func generateLicenseKey(pkey *ecdsa.PrivateKey, orderId string, orderTime time.Time) (string, error) {
	licenseVersion := 1
	licenseKey := fmt.Sprintf("%d-%s-%d", licenseVersion, orderId, orderTime.Unix())
	hash := sha256.Sum256([]byte(licenseKey))
	signature, err := ecdsa.SignASN1(rand.Reader, pkey, hash[:])
	if err != nil {
		return "", err
	}

	encodedSignature := base64.StdEncoding.EncodeToString(signature)
	return fmt.Sprintf("%s-%s", licenseKey, encodedSignature), nil
}

func readPrivateKeyFromEnv() (*ecdsa.PrivateKey, error) {
	pkeyPem := os.Getenv("ELM_PAIR_LICENSING_SERVER_SIGNING_KEY")
	if pkeyPem == "" {
		return nil, errors.New("not set: ELM_PAIR_LICENSING_SERVER_SIGNING_KEY")
	}

	pkeyX509, _ := pem.Decode([]byte(pkeyPem))
	pkey, err := x509.ParseECPrivateKey(pkeyX509.Bytes)
	if err != nil {
		return nil, err
	}

	return pkey, nil
}

func readPaddleKeyFromEnv() (*rsa.PublicKey, error) {
	keyPem := os.Getenv("ELM_PAIR_LICENSING_SERVER_PADDLE_KEY")
	if keyPem == "" {
		return nil, errors.New("not set: ELM_PAIR_LICENSING_SERVER_PADDLE_KEY")
	}

	keyX509, _ := pem.Decode([]byte(keyPem))
	if keyX509 == nil {
		return nil, errors.New("Could not parse paddle key pem")
	}

	pub, err := x509.ParsePKIXPublicKey(keyX509.Bytes)
	if err != nil {
		return nil, errors.New("Could not parse paddle key x509")
	}

	key, ok := pub.(*rsa.PublicKey)
	if !ok {
		return nil, errors.New("Could not get public paddle key")
	}

	return key, nil
}

// Adapted from:
// https://gist.github.com/haseebq/adc51aaeb4e612c205291a411a7a8872#file-paddle_hook_verify-go
func verifyPaddleSig(values url.Values, signingKey *rsa.PublicKey) error {
	sig, err := base64.StdEncoding.DecodeString(values.Get("p_signature"))
	if err != nil {
		return err
	}

	// Delete p_signature
	values.Del("p_signature")

	// Sort the keys
	sortedKeys := make([]string, 0, len(values))
	for k := range values {
		sortedKeys = append(sortedKeys, k)
	}
	sort.Strings(sortedKeys)

	// Php Serialize in sorted order
	var sbuf bytes.Buffer
	sbuf.WriteString("a:")
	sbuf.WriteString(strconv.Itoa(len(sortedKeys)))
	sbuf.WriteString(":{")
	encodeString := func(s string) {
		sbuf.WriteString("s:")
		sbuf.WriteString(strconv.Itoa(len(s)))
		sbuf.WriteString(":\"")
		sbuf.WriteString(s)
		sbuf.WriteString("\";")
	}
	for _, k := range sortedKeys {
		encodeString(k)
		encodeString(values.Get(k))
	}
	sbuf.WriteString("}")

	sha1Sum := sha1.Sum(sbuf.Bytes())
	err = rsa.VerifyPKCS1v15(signingKey, crypto.SHA1, sha1Sum[:], sig)
	if err != nil {
		return err
	}

	return nil
}
