package com.acme.payments.events;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import java.io.IOException;
import java.sql.Connection;
import java.sql.PreparedStatement;
import java.sql.SQLException;
import javax.sql.DataSource;

/** Records a payment event and adds the amount to the order's paid total. */
public class PaymentEventHandler implements EventHandler {
    private static final ObjectMapper JSON = new ObjectMapper();

    private final DataSource db;

    public PaymentEventHandler(DataSource db) {
        this.db = db;
    }

    @Override
    public void handle(String eventId, String payload) {
        try (Connection conn = db.getConnection()) {
            JsonNode body = JSON.readTree(payload);
            String orderId = body.get("order_id").asText();
            long amountCents = body.get("amount_cents").asLong();

            try (PreparedStatement insert = conn.prepareStatement(
                    "INSERT INTO payment_events (event_id, order_id, amount_cents) VALUES (?, ?, ?)")) {
                insert.setString(1, eventId);
                insert.setString(2, orderId);
                insert.setLong(3, amountCents);
                insert.executeUpdate();
            }
            try (PreparedStatement update = conn.prepareStatement(
                    "UPDATE orders SET paid_cents = paid_cents + ? WHERE id = ?")) {
                update.setLong(1, amountCents);
                update.setString(2, orderId);
                update.executeUpdate();
            }
        } catch (SQLException | IOException e) {
            throw new IllegalStateException("could not apply event " + eventId, e);
        }
    }
}
