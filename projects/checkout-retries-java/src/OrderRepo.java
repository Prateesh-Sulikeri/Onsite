import java.util.Map;
import java.util.Optional;
import java.util.concurrent.ConcurrentHashMap;

/** Orders kept in memory for this exercise. A real one would sit on PostgreSQL. */
public class OrderRepo {

    public enum Status { NEW, PAID, FAILED }

    public static final class Order {
        public final String id;
        public final String customerId;
        public final long totalCents;
        private volatile Status status = Status.NEW;
        private volatile String chargeId;

        public Order(String id, String customerId, long totalCents) {
            this.id = id;
            this.customerId = customerId;
            this.totalCents = totalCents;
        }

        public Status status() { return status; }
        public String chargeId() { return chargeId; }

        void markPaid(String chargeId) {
            this.chargeId = chargeId;
            this.status = Status.PAID;
        }

        void markFailed() {
            this.status = Status.FAILED;
        }
    }

    private final Map<String, Order> orders = new ConcurrentHashMap<>();

    public void save(Order order) {
        orders.put(order.id, order);
    }

    public Optional<Order> find(String id) {
        return Optional.ofNullable(orders.get(id));
    }
}
