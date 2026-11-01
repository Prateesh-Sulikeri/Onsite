package com.acme.wallet;

import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;

/** Holds every wallet in memory. The real service loads these from the database at start. */
public class AccountStore {
    private final Map<String, Account> accounts = new ConcurrentHashMap<>();

    public void add(Account account) {
        accounts.put(account.id, account);
    }

    public Account get(String id) {
        Account account = accounts.get(id);
        if (account == null) {
            throw new IllegalArgumentException("unknown account " + id);
        }
        return account;
    }

    public double total() {
        double sum = 0;
        for (Account account : accounts.values()) {
            sum += account.getBalance();
        }
        return sum;
    }
}
