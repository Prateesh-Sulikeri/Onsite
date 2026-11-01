package webhook

import (
	"crypto/hmac"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"strings"
)

var (
	ErrBadHeader    = errors.New("malformed signature header")
	ErrBadSignature = errors.New("signature does not match")
)

// parseHeader reads "t=1700000000,v1=<hex>" and returns the timestamp text and the signature.
func parseHeader(header string) (ts string, sig string, err error) {
	for _, part := range strings.Split(header, ",") {
		kv := strings.SplitN(strings.TrimSpace(part), "=", 2)
		if len(kv) != 2 {
			continue
		}
		switch kv[0] {
		case "t":
			ts = kv[1]
		case "v1":
			sig = kv[1]
		}
	}
	if ts == "" || sig == "" {
		return "", "", ErrBadHeader
	}
	return ts, sig, nil
}

// Sign returns the hex HMAC the sender puts in the header: HMAC-SHA256(secret, ts + "." + body).
func Sign(secret []byte, ts string, body []byte) string {
	mac := hmac.New(sha256.New, secret)
	mac.Write([]byte(ts))
	mac.Write([]byte("."))
	mac.Write(body)
	return hex.EncodeToString(mac.Sum(nil))
}

// Verify checks the X-Signature header against the raw request body.
func Verify(secret []byte, header string, body []byte) error {
	ts, sig, err := parseHeader(header)
	if err != nil {
		return err
	}
	expected := Sign(secret, ts, body)
	if expected != sig {
		return ErrBadSignature
	}
	return nil
}
