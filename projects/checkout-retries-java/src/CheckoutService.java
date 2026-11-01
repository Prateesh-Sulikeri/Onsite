import java.util.concurrent.TimeoutException;

/** Takes payment for an order. Called by the web layer, often twice when the user double-clicks. */
public class CheckoutService {

    private static final int MAX_ATTEMPTS = 3;
    private static final long BACKOFF_MS = 200;

    private final PaymentGateway gateway;
    private final OrderRepo orders;

    public CheckoutService(PaymentGateway gateway, OrderRepo orders) {
        this.gateway = gateway;
        this.orders = orders;
    }

    /** Returns the charge id. */
    public String checkout(String orderId) {
        OrderRepo.Order order = orders.find(orderId)
                .orElseThrow(() -> new IllegalArgumentException("no such order: " + orderId));

        if (order.status() == OrderRepo.Status.PAID) {
            return order.chargeId();
        }

        PaymentGateway.Receipt receipt = chargeWithRetry(order);
        order.markPaid(receipt.chargeId());
        return receipt.chargeId();
    }

    private PaymentGateway.Receipt chargeWithRetry(OrderRepo.Order order) {
        int attempt = 0;
        while (true) {
            attempt++;
            try {
                return gateway.charge(order.customerId, order.totalCents);
            } catch (TimeoutException e) {
                if (attempt >= MAX_ATTEMPTS) {
                    throw new IllegalStateException("payment timed out for order " + order.id, e);
                }
                sleep(BACKOFF_MS * attempt);
            } catch (IllegalStateException declined) {
                order.markFailed();
                throw declined;
            }
        }
    }

    private void sleep(long ms) {
        try {
            Thread.sleep(ms);
        } catch (InterruptedException ignored) {
        }
    }
}
