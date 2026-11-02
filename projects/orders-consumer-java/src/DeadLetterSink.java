package com.acme.payments.events;

import java.nio.charset.StandardCharsets;
import org.apache.kafka.clients.consumer.ConsumerRecord;
import org.apache.kafka.clients.producer.KafkaProducer;
import org.apache.kafka.clients.producer.ProducerRecord;

/** Parks records we cannot process on a side topic so someone can look at them. */
public class DeadLetterSink {
    private final KafkaProducer<String, String> producer;
    private final String topic;

    public DeadLetterSink(KafkaProducer<String, String> producer, String topic) {
        this.producer = producer;
        this.topic = topic;
    }

    public void publish(ConsumerRecord<String, String> record, String reason) {
        ProducerRecord<String, String> out = new ProducerRecord<>(topic, record.key(), record.value());
        out.headers().add("dlq-reason", reason.getBytes(StandardCharsets.UTF_8));
        out.headers().add("dlq-source", (record.topic() + "-" + record.partition() + "@" + record.offset())
                .getBytes(StandardCharsets.UTF_8));
        producer.send(out);
    }
}
