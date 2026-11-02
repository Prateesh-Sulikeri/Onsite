import java.util.concurrent.TimeoutException;
import java.util.concurrent.atomic.AtomicInteger;

/** How the service is exercised. Not run in this round. */
public class CheckoutServiceTest {

    /** A gateway that takes the money, then fails to answer the first time. */
    static class SlowGateway implements PaymentGateway {
        final AtomicInteger charges = new AtomicInteger();
        private boolean first = true;

        public synchronized Receipt charge(String customerId, long amountCents) throws TimeoutException {
            Receipt r = new Receipt("ch_" + charges.incrementAndGet(), amountCents);
            if (first) {
                first = false;
                throw new TimeoutException("no answer");
            }
            return r;
        }

        public Receipt charge(String customerId, long amountCents, String key) throws TimeoutException {
            return charge(customerId, amountCents);
        }
    }

    public static void main(String[] args) {
        SlowGateway gateway = new SlowGateway();
        OrderRepo repo = new OrderRepo();
        repo.save(new OrderRepo.Order("o1", "c1", 4999));
        new CheckoutService(gateway, repo).checkout("o1");
        System.out.println("charges: " + gateway.charges.get());
    }
}
