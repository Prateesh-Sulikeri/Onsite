import java.util.ArrayList;
import java.util.List;

/** How the pieces are meant to behave. Not run in this round. */
public class EventConsumerTest {

    /** Records what the handler saw, and fails on one chosen event id. */
    static class RecordingHandler implements com.acme.payments.events.EventHandler {
        final List<String> seen = new ArrayList<>();
        private final String failOn;

        RecordingHandler(String failOn) {
            this.failOn = failOn;
        }

        public void handle(String eventId, String payload) {
            if (eventId.equals(failOn)) {
                throw new IllegalStateException("bad payload");
            }
            seen.add(eventId);
        }
    }

    public static void main(String[] args) {
        // Scenario 1: the process dies after poll() returns 3 records, before the handler ran.
        // Expected: all 3 are redelivered after restart.
        // Scenario 2: the broker redelivers evt-1 after a rebalance.
        // Expected: orders.paid_cents counts it once.
        // Scenario 3: evt-2 has a payload nobody can parse.
        // Expected: evt-3 and later events still get handled.
        RecordingHandler handler = new RecordingHandler("evt-2");
        System.out.println("handler ready: " + handler.seen.size());
    }
}
