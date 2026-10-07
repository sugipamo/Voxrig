# Checked survival API and ownership

`checked::SurvivalCapabilities::for_version()` and
`java_1_21_11::checked::SurvivalCapabilities::for_version(version)` describe static
adapter support. `client.java_1_21_11()?.checked_survival()` returns a session-bound checked handle, or
`Unsupported` before I/O for Java 1.16.1. Its legacy `Bot` and `survival` module
remain unchanged. Java 1.21.11 implements `ObservedDryCubeV1` and the separately advertised
`PredictedDryCubeV1`; existing observed entry points keep their contract.

Support is not readiness or permission. Each call still validates native state,
identity, generation, geometry, inventory and unresolved intents. The façade
forwards to the original implementation; it adds no physics, fallback, automatic
retry, relaxed admission or second operation history. Data types currently share
the native 1.21.11 representation and retain their version/session provenance;
this is not a promise that another version already implements these semantics.

| Voxrig | Consumer |
| --- | --- |
| Version selection and capability contract | Select supported deployment/version |
| Received state, collision, aim, bounded motion prediction | Choose route, building geometry and action sequence |
| Before-I/O intent and single-operation observations | Site/edit permissions and material reservations |
| Exact session retirement and fresh connection validation | Decide whether recovery is appropriate; replan afterward |
| Non-restorable in-process handles and diagnostic history | Durable jobs, restart diagnosis and user-facing errors |

The checked handle offers received player/history/standing observations,
hypothetical scenes, bounded walking/jumping, main/hotbar plain-stack swaps,
passive full-cube placement, empty-hand dirt/stone mining and motion reassessment.
It exposes no commands, creative inventory or raw packet sending. The explicit
version-specific API remains for other operations. See [movement](survival-movement-foundation.md),
[hypothetical scenes](survival-hypothetical-scenes.md), and
[mining retirement](survival-mining-retirement.md) for admission/evidence limits.

## Explicit mining lifecycle

1. `operations.prepare_mining_retirement(&intent, &observer).await` returns a
   `MiningRetirement` handle after registering the exact independent watch.
   No close or reconnect occurs in preparation.
2. Retain that handle before calling `close_source().await`. A cancelled close
   or wait does not authorize another mining action. `source_history()` remains
   accessible even when observer inspection fails.
3. `observe()` / `wait(duration)` return native pending, retired or inspection
   evidence, including the existing `recovery_started` marker. Waits are read-only
   and resumable on the same handle; dropping the handle performs no recovery.
4. Only after retirement, explicitly call `reconnect(config, expected_target)`.
   Native validation records the attempt before network I/O. Cloned handles share
   that guard; cancellation/failure cannot authorize a second login.
5. The result contains `client`, checked `operations` and `evidence` on the same
   fresh session. The consumer must inspect/replan its own work. No Blueprint,
   durable job, edit permission or material reservation crosses this boundary.

These are direct, unmodified vanilla 1.21.11 retirement semantics. The façade does
not expand support to proxies/plugins or infer retirement from missing entities,
wall time, target air, ABORT or local closure alone. Existing low-level methods
coexist for callers that explicitly manage the same lifecycle themselves.

## Explicit single-client choices

`start_predicted_survival_path` and `start_previewed_predicted_survival_motion`
select prediction-based continuation without observer watches. A `Predicted`
record retains a fully dispatched model endpoint, not a measured server pose or
stop acknowledgement. Its 1/16-block `planning_reserve` is a model-space policy,
not a physical error bound. Fresh native standing, generation, loading, posture,
correction and geometry checks remain mandatory. Hypothetical plans declare the
same contract, and prediction cannot satisfy an independent observation obligation.
See [prediction contract and source evidence](survival-predicted-motion.md).

`prepare_mining_profile_recovery` prepares a `MiningProfileRecovery` without I/O.
Explicitly close the source, then call its once-only `reconnect(config, target)`.
This requires exclusive profile ownership and direct unmodified vanilla 1.21.11.
Successful same-profile login and fresh admission establish the boundary; local
closure alone does not. `MiningRecoveryEvidence.boundary` preserves the distinction
between independent removal receipts and same-profile login. Both methods share
the original before-I/O claim, including cancellation and failure. Neither restores
old operation authority or a durable job. See [same-profile recovery](survival-single-profile-recovery.md).
