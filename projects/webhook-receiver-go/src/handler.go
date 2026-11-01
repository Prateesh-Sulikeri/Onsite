package webhook

import (
	"context"
	"encoding/json"
	"io"
	"log"
	"net/http"
)

const maxBodyBytes = 1 << 20

// Event is the JSON the payment provider posts to us.
type Event struct {
	ID   string          `json:"id"`
	Type string          `json:"type"`
	Data json.RawMessage `json:"data"`
}

// Processor does the real work for one event, such as marking an order as paid.
type Processor interface {
	Process(ctx context.Context, e Event) error
}

type Handler struct {
	secret    []byte
	store     Store
	processor Processor
}

func NewHandler(secret []byte, store Store, processor Processor) *Handler {
	return &Handler{secret: secret, store: store, processor: processor}
}

func (h *Handler) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	body, err := io.ReadAll(http.MaxBytesReader(w, r.Body, maxBodyBytes))
	if err != nil {
		http.Error(w, "body too large", http.StatusRequestEntityTooLarge)
		return
	}
	if err := Verify(h.secret, r.Header.Get("X-Signature"), body); err != nil {
		http.Error(w, "invalid signature", http.StatusUnauthorized)
		return
	}

	var e Event
	if err := json.Unmarshal(body, &e); err != nil || e.ID == "" {
		http.Error(w, "bad event", http.StatusBadRequest)
		return
	}

	seen, err := h.store.Seen(r.Context(), e.ID)
	if err != nil {
		http.Error(w, "store error", http.StatusInternalServerError)
		return
	}
	if seen {
		w.WriteHeader(http.StatusOK)
		return
	}
	if err := h.processor.Process(r.Context(), e); err != nil {
		log.Printf("event %s failed: %v", e.ID, err)
		http.Error(w, "processing failed", http.StatusInternalServerError)
		return
	}
	if err := h.store.MarkSeen(r.Context(), e.ID); err != nil {
		log.Printf("event %s handled but not recorded: %v", e.ID, err)
	}
	w.WriteHeader(http.StatusOK)
}
