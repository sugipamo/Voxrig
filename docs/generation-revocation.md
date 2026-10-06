# Independent generation revocation (Java 1.16.1)

An ordinary disconnect waits behind the native connection actor's commands and
writes. `Bot::revoke_connection()` irreversibly revokes this exact generation
without waiting for capture, control or writer queues. It returns a typed local
`GenerationRevocation`, not a clean disconnect or a successful cancellation.

The revocation flag closes later owner admission; the owner and reader tasks are
aborted, and writer shutdown is scheduled on the original runtime. This also
works when the caller is outside the runtime thread. An already admitted write
may have reached the peer partially. Writer shutdown may still be pending, and
runtime scheduling stalls are not bounded. The Bot cannot be reused/reconnected.

The lifecycle remains `ConnectionStateUnknown`. Pending acknowledgement waits
remain `DeliveryUnknown`; lost primitive/batch receipts after revocation also
return `DispatchOutcome::DeliveryUnknown`. Lost acknowledged-operation creation
receipts use `DispatchError::DeliveryUnknown`, separately from pre-write admission
rejection. Lost raw packet/protocol replies carry `ErrorKind::UncertainDispatch`.
No unsent action is acknowledged or automatically replayed.

This is an emergency generation fence. Ordinary disconnect, timeout durations,
observation fairness and the Java 1.21.11 API are unchanged. Holds, inventory
ownership, job assignment and external watchdogs remain consumer responsibilities.

Regression coverage includes a writer lock and full queued command channel,
coherent capture behind a gate, an already written dig with pending acknowledgement,
lost acknowledged-operation creation, generation isolation and repeated revocation.
The fixtures use isolated loopback TCP, not production workers or world resets.
