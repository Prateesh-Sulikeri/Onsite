package com.acme.payments.events;

import java.time.Duration;
import java.util.List;
import java.util.concurrent.atomic.AtomicBoolean;
import org.apache.kafka.clients.consumer.ConsumerRecord;
import org.apache.kafka.clients.consumer.ConsumerRecords;
import org.apache.kafka.clients.consumer.KafkaConsumer;
import org.apache.kafka.common.TopicPartition;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

/** Reads payment events from one topic and hands each one to the handler. */
public class EventConsumer implements Runnable {
    private static final Logger log = LoggerFactory.getLogger(EventConsumer.class);
    private static final Duration POLL_TIMEOUT = Duration.ofMillis(500);

    private final KafkaConsumer<String, String> consumer;
    private final EventHandler handler;
    private final DeadLetterSink deadLetters;
    private final String topic;
    private final AtomicBoolean running = new AtomicBoolean(true);

    public EventConsumer(KafkaConsumer<String, String> consumer, EventHandler handler,
                         DeadLetterSink deadLetters, String topic) {
        this.consumer = consumer;
        this.handler = handler;
        this.deadLetters = deadLetters;
        this.topic = topic;
    }

    @Override
    public void run() {
        consumer.subscribe(List.of(topic));
        try {
            while (running.get()) {
                ConsumerRecords<String, String> records = consumer.poll(POLL_TIMEOUT);
                if (records.isEmpty()) {
                    continue;
                }
                consumer.commitSync();
                for (ConsumerRecord<String, String> record : records) {
                    if (!process(record)) {
                        break;
                    }
                }
            }
        } finally {
            consumer.close();
        }
    }

    private boolean process(ConsumerRecord<String, String> record) {
        if (record.key() == null) {
            deadLetters.publish(record, "missing key");
            return true;
        }
        try {
            handler.handle(record.key(), record.value());
            return true;
        } catch (RuntimeException e) {
            log.error("failed to handle {}-{}@{}", record.topic(), record.partition(), record.offset(), e);
            consumer.seek(new TopicPartition(record.topic(), record.partition()), record.offset());
            return false;
        }
    }

    public void stop() {
        running.set(false);
        consumer.wakeup();
    }
}
