package main

import (
	"bytes"
	"crypto"
	"crypto/ed25519"
	"crypto/rsa"
	"crypto/sha1"
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

	healthChecksIoUuid := os.Getenv("ELM_PAIR_LICENSING_SERVER_HEALTH_CHECKS_IO_UUID")
	if healthChecksIoUuid == "" {
		log.Fatal("not set: ELM_PAIR_LICENSING_SERVER_HEALTH_CHECKS_IO_UUID")
		return
	}

	httpClient := http.Client{Timeout: 10 * time.Second}

	http.HandleFunc("/v1/ping",
		func(writer http.ResponseWriter, r *http.Request) {
			responder := Responder{writer, httpClient, healthChecksIoUuid}
			responder.success("pong")
		})
	http.HandleFunc("/v1/generate-license-key",
		func(writer http.ResponseWriter, r *http.Request) {
			responder := Responder{writer, httpClient, healthChecksIoUuid}
			generateLicenseKeyHandler(pkey, paddleKey, responder, r)
		})
	log.Fatal(http.ListenAndServe(fmt.Sprintf(":%s", port), nil))
}

func generateLicenseKeyHandler(
	pkey ed25519.PrivateKey,
	paddleKey *rsa.PublicKey,
	w Responder,
	r *http.Request,
) {
	r.Body = http.MaxBytesReader(w.writer, r.Body, 1024*1024)
	if err := r.ParseForm(); err != nil {
		w.error("failed to parse formdata")
		return
	}

	err := verifyPaddleSig(r.Form, paddleKey)
	if err != nil {
		w.error("invalid paddle signature")
		return
	}

	orderId := r.FormValue("p_order_id")
	if orderId == "" {
		w.error("missing p_order_id field")
		return
	}

	eventTimeStr := r.FormValue("event_time")
	if eventTimeStr == "" {
		w.error("missing event_time field")
		return
	}

	layout := "2006-01-02 15:04:05"
	eventTime, err := time.Parse(layout, eventTimeStr)
	if err != nil {
		w.error(fmt.Sprintf("failed to parse event_time %s: %s", eventTimeStr, err))
		return
	}

	licenseKey, err := generateLicenseKey(pkey, orderId, eventTime)
	if err != nil {
		w.error(fmt.Sprintf("failed to generate license key: %s", err))
	}

	w.success(licenseKey)
}

type Responder struct {
	writer             http.ResponseWriter
	httpClient         http.Client
	healthChecksIoUuid string
}

func (w Responder) success(res string) {
	url := fmt.Sprintf("https://hc-ping.com/%s", w.healthChecksIoUuid)
	_, err := w.httpClient.Head(url)
	if err != nil {
		log.Println(err)
	}
	fmt.Fprintf(w.writer, "%s", res)
}

func (w Responder) error(msg string) {
	url := fmt.Sprintf("https://hc-ping.com/%s/fail", w.healthChecksIoUuid)
	log.Println(url)
	_, err := w.httpClient.Post(url, "text/plain;charset=UTF-8", bytes.NewBuffer([]byte(msg)))
	if err != nil {
		log.Println(err)
	}
	http.Error(w.writer, "Internal Server Error", http.StatusInternalServerError)
}

func generateLicenseKey(pkey ed25519.PrivateKey, orderId string, orderTime time.Time) (string, error) {
	licenseVersion := 1
	licenseKey := fmt.Sprintf("%d-%s-%d", licenseVersion, orderId, orderTime.Unix())
	signature := ed25519.Sign(pkey, []byte(licenseKey))

	encodedSignature := base64.StdEncoding.EncodeToString(signature)
	return fmt.Sprintf("%s-%s", licenseKey, encodedSignature), nil
}

func readPrivateKeyFromEnv() (ed25519.PrivateKey, error) {
	pkeyPem := os.Getenv("ELM_PAIR_LICENSING_SERVER_SIGNING_KEY")
	if pkeyPem == "" {
		return nil, errors.New("not set: ELM_PAIR_LICENSING_SERVER_SIGNING_KEY")
	}

	pkeyX509, _ := pem.Decode([]byte(pkeyPem))
	data, err := x509.ParsePKCS8PrivateKey(pkeyX509.Bytes)
	if err != nil {
		return nil, err
	}

	pkey, ok := data.(ed25519.PrivateKey)
	if !ok {
		return nil, errors.New("Could not parse ed25119 private key")
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
