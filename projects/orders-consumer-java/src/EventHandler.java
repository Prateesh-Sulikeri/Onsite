package com.acme.payments.events;

/** Applies one payment event to our own tables. */
public interface EventHandler {
    void handle(String eventId, String payload);
}
