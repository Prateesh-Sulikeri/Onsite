import com.acme.wallet.Account;
import com.acme.wallet.AccountStore;
import com.acme.wallet.TransferService;

/** How the service is exercised. Not run in this round. */
public class TransferServiceTest {

    public static void main(String[] args) {
        AccountStore store = new AccountStore();
        store.add(new Account("a", "u1", 100));
        store.add(new Account("b", "u2", 100));
        TransferService service = new TransferService(store);

        // Scenario 1: 10 transfers of 0.10 from a to b, then compare the total with 200.
        // Expected: the total is exactly 200.
        // Scenario 2: 1000 threads each top up "a" by 1 while 1000 threads move 1 from a to b.
        // Expected: a ends at 100 + 1000 - 1000, b at 100 + 1000.
        // Scenario 3: one thread moves a to b while another moves b to a, 10,000 times each.
        // Expected: both finish.
        System.out.println("total: " + store.total());
    }
}
