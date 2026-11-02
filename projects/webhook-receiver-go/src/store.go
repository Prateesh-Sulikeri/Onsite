package webhook

import (
	"context"
	"sync"
)

// Store remembers which event ids we have already handled.
type Store interface {
	Seen(ctx context.Context, id string) (bool, error)
	MarkSeen(ctx context.Context, id string) error
}

// MemoryStore is used in tests and local runs. Production uses a table with a unique id column.
type MemoryStore struct {
	mu   sync.Mutex
	seen map[string]bool
}

func NewMemoryStore() *MemoryStore {
	return &MemoryStore{seen: make(map[string]bool)}
}

func (m *MemoryStore) Seen(ctx context.Context, id string) (bool, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	return m.seen[id], nil
}

func (m *MemoryStore) MarkSeen(ctx context.Context, id string) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.seen[id] = true
	return nil
}
