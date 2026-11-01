package webhook

import (
	"strconv"
	"testing"
	"time"
)

// How the receiver is meant to behave. Not run in this round.

func header(secret []byte, at time.Time, body []byte) string {
	ts := strconv.FormatInt(at.Unix(), 10)
	return "t=" + ts + ",v1=" + Sign(secret, ts, body)
}

func TestFreshSignatureIsAccepted(t *testing.T) {
	secret := []byte("whsec_test")
	body := []byte(`{"id":"evt_1","type":"payment.captured"}`)
	if err := Verify(secret, header(secret, time.Now(), body), body); err != nil {
		t.Fatalf("expected ok, got %v", err)
	}
}

func TestChangedBodyIsRejected(t *testing.T) {
	secret := []byte("whsec_test")
	h := header(secret, time.Now(), []byte(`{"id":"evt_1"}`))
	if err := Verify(secret, h, []byte(`{"id":"evt_2"}`)); err == nil {
		t.Fatal("expected an error")
	}
}

func TestOldCapturedRequestIsRejected(t *testing.T) {
	// A request signed an hour ago and sent again now.
	// Expected: Verify returns an error even though the signature is valid.
}

func TestSameEventSentTwiceRunsOnce(t *testing.T) {
	// The provider retries evt_1 while the first call is still running.
	// Expected: the processor runs once, and both calls answer 200.
}
