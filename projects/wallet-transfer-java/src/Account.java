package com.acme.wallet;

import java.util.concurrent.locks.ReentrantLock;

/** One wallet. Balance is in rupees. */
public class Account {
    public final String id;
    public final String ownerId;
    public final ReentrantLock lock = new ReentrantLock();
    private double balance;

    public Account(String id, String ownerId, double openingBalance) {
        this.id = id;
        this.ownerId = ownerId;
        this.balance = openingBalance;
    }

    public double getBalance() {
        return balance;
    }

    public void setBalance(double balance) {
        this.balance = balance;
    }

    @Override
    public String toString() {
        return "Account{" + id + ", " + balance + "}";
    }
}
