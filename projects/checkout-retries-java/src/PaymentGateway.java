import java.util.concurrent.TimeoutException;

/** Talks to the card processor over HTTP. Slow under load. */
public interface PaymentGateway {

    record Receipt(String chargeId, long amountCents) {}

    /**
     * Charges the customer's saved card.
     *
     * @throws TimeoutException when the processor did not answer in time. The charge may or may not
     *                          have gone through.
     * @throws IllegalStateException when the card is declined.
     */
    Receipt charge(String customerId, long amountCents) throws TimeoutException;

    /**
     * Same as {@link #charge(String, long)}. The processor remembers {@code key} for 24 hours and
     * returns the first result for a key it has already seen.
     */
    Receipt charge(String customerId, long amountCents, String key) throws TimeoutException;
}
