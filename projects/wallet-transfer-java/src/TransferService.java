package com.acme.wallet;

/** Moves money between wallets and adds money from a top-up. */
public class TransferService {
    private final AccountStore store;

    public TransferService(AccountStore store) {
        this.store = store;
    }

    public void transfer(String fromId, String toId, double amount) {
        if (amount <= 0) {
            throw new IllegalArgumentException("amount must be positive");
        }
        if (fromId.equals(toId)) {
            throw new IllegalArgumentException("cannot transfer to the same account");
        }
        Account from = store.get(fromId);
        Account to = store.get(toId);

        from.lock.lock();
        try {
            to.lock.lock();
            try {
                if (from.getBalance() < amount) {
                    throw new InsufficientFundsException(fromId, amount);
                }
                from.setBalance(from.getBalance() - amount);
                to.setBalance(to.getBalance() + amount);
            } finally {
                to.lock.unlock();
            }
        } finally {
            from.lock.unlock();
        }
    }

    public void topUp(String accountId, double amount) {
        if (amount <= 0) {
            throw new IllegalArgumentException("amount must be positive");
        }
        Account account = store.get(accountId);
        double current = account.getBalance();
        account.setBalance(current + amount);
    }

    public static class InsufficientFundsException extends RuntimeException {
        public InsufficientFundsException(String accountId, double amount) {
            super("account " + accountId + " cannot cover " + amount);
        }
    }
}
