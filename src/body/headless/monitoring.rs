use super::*;

impl HeadlessBody {
    /// Refresh one retained action. Acceptance stays immutable once terminal.
    /// Java 1.16.1 and 1.21.11 share this evidence reducer. No packets are sent.
    pub async fn action_observation(&self, id: ActionId) -> Result<ActionObservation> {
        let job = self.inner.job(id)?;
        self.inner.update(id, |m| m.expire(Instant::now()));
        let _refresh = self.inner.monitor_refresh.lock().await;
        self.refresh_monitors_locked(Some(id)).await?;
        let acceptance = job.acceptance.lock().unwrap().status.clone();
        let outcome = job
            .late_correction
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| acceptance.clone());
        let monitoring = job.monitoring.lock().unwrap().clone();
        Ok(ActionObservation {
            acceptance,
            outcome,
            monitoring,
        })
    }

    /// Refresh all retained terminal Accepted/Unknown actions, then read the
    /// dedicated log. A log gap is independent of BodyEvents.gap. Read failures
    /// become retryable transitions; evidence/lifetime loss permanently ends tracking.
    pub async fn monitor_changes_after(
        &self,
        cursor: MonitorCursor,
        maximum: usize,
    ) -> Result<MonitorChanges> {
        if !(1..=4096).contains(&maximum)
            || (cursor.backend != 0 && cursor.backend != self.inner.id)
            || (cursor.backend == 0 && cursor.ordinal != 0)
        {
            return Err(invalid("invalid body monitor cursor or maximum (1..4096)"));
        }
        {
            let records = self.inner.records.lock().unwrap();
            if cursor.ordinal > records.monitor_ordinal {
                return Err(invalid("body monitor cursor is ahead of this backend"));
            }
        }
        self.refresh_monitors(None).await?;
        let records = self.inner.records.lock().unwrap();
        let changes: Vec<_> = records
            .monitor_changes
            .iter()
            .filter(|c| c.cursor.ordinal > cursor.ordinal)
            .take(maximum)
            .cloned()
            .collect();
        Ok(MonitorChanges {
            cursor: changes.last().map_or(
                MonitorCursor {
                    backend: self.inner.id,
                    ordinal: cursor.ordinal,
                },
                |c| c.cursor,
            ),
            gap: cursor.ordinal < records.monitor_dropped,
            changes,
        })
    }

    pub(super) async fn refresh_monitors(&self, only: Option<ActionId>) -> Result<()> {
        // Serialize reads and commits: a slower old snapshot must not overwrite a
        // newer boundary or turn a permanent gap into CompleteThrough.
        let _refresh = self.inner.monitor_refresh.lock().await;
        self.refresh_monitors_locked(only).await
    }

    async fn refresh_monitors_locked(&self, only: Option<ActionId>) -> Result<()> {
        let monitors: Vec<_> = {
            let records = self.inner.records.lock().unwrap();
            records
                .jobs
                .iter()
                .filter_map(|(id, job)| {
                    if only.is_some_and(|wanted| wanted != *id)
                        || job.monitoring.lock().unwrap().ended()
                        || !matches!(
                            job.acceptance.lock().unwrap().status,
                            ActionStatus::Accepted | ActionStatus::Unknown { .. }
                        )
                        || !matches!(
                            job.acceptance.lock().unwrap().dispatch_status(),
                            Some(SendStatus::Sent)
                        )
                    {
                        return None;
                    }
                    job.monitor.lock().unwrap().clone().map(|m| (*id, m))
                })
                .collect()
        };
        let Some(cursor) = monitors.iter().map(|(_, m)| m.cursor).min() else {
            return Ok(());
        };
        let mut blocks: Vec<_> = monitors
            .iter()
            .flat_map(|(_, m)| m.selection.blocks.iter().copied())
            .collect();
        blocks.sort_unstable();
        blocks.dedup();
        let ops = self.operations()?;
        let result = timeout(
            self.inner.config.settle_window,
            ops.body_evidence(
                self.inner.id,
                &ObserveRequest {
                    blocks,
                    region: None,
                },
                Some(cursor),
            ),
        )
        .await;
        match result {
            Ok(Ok(batch)) => {
                let retained_from = batch.events.first().map_or(batch.cursor, |e| e.ordinal - 1);
                for (id, monitor) in monitors {
                    let status = if batch.gap && monitor.cursor < retained_from {
                        MonitoringStatus::Gap {
                            after: monitor.cursor,
                            retained_from,
                        }
                    } else if batch.snapshot.world != monitor.world {
                        MonitoringStatus::WorldChanged {
                            after: monitor.cursor,
                            world: batch.snapshot.world,
                        }
                    } else {
                        self.inspect_monitor(id, &monitor, &batch)?
                    };
                    self.inner.record_monitor(id, status);
                }
            }
            failure => {
                let reason = match failure {
                    Err(_) => "body evidence refresh timed out".to_owned(),
                    Ok(Err(e)) => e.to_string(),
                    Ok(Ok(_)) => unreachable!(),
                };
                for (id, monitor) in monitors {
                    self.inner.record_monitor(
                        id,
                        MonitoringStatus::TemporarilyUnavailable {
                            after: monitor.cursor,
                            reason: reason.clone(),
                        },
                    );
                }
            }
        }
        Ok(())
    }

    fn inspect_monitor(
        &self,
        id: ActionId,
        monitor: &Monitor,
        batch: &EvidenceBatch,
    ) -> Result<MonitoringStatus> {
        // Process original events in order: never cross a lost chunk/world lifetime.
        let mut own = batch.clone();
        own.events.clear();
        for event in batch.events.iter().filter(|e| e.ordinal > monitor.cursor) {
            match event.update {
                Update::World => {
                    return Ok(MonitoringStatus::WorldChanged {
                        after: monitor.cursor,
                        world: batch.snapshot.world,
                    });
                }
                Update::Chunk(chunk)
                    if monitor
                        .selection
                        .blocks
                        .iter()
                        .any(|pos| [pos[0].div_euclid(16), pos[2].div_euclid(16)] == chunk) =>
                {
                    return Ok(MonitoringStatus::ChunkChanged {
                        after: monitor.cursor,
                        at: event.ordinal,
                        chunk,
                    });
                }
                _ => {}
            }
            own.events.clear();
            own.events.push(event.clone());
            if let Some(corrected @ ActionStatus::Corrected { .. }) =
                correction(&own, monitor.expected.as_ref(), &monitor.action)?
            {
                self.inner.record_late(id, corrected);
                return Ok(MonitoringStatus::CorrectionObserved { at: event.ordinal });
            }
        }
        if let Ok(job) = self.inner.job(id) {
            if let Some(monitor) = job.monitor.lock().unwrap().as_mut() {
                monitor.cursor = batch.cursor;
            }
        }
        Ok(MonitoringStatus::CompleteThrough {
            cursor: batch.cursor,
        })
    }
}
impl Inner {
    fn record_monitor(&self, id: ActionId, monitoring: MonitoringStatus) {
        let mut records = self.records.lock().unwrap();
        let Some(job) = records.jobs.get(&id).cloned() else {
            return;
        };
        let mut current = job.monitoring.lock().unwrap();
        if current.ended() || *current == monitoring {
            return;
        }
        *current = monitoring.clone();
        records.monitor_ordinal += 1;
        let cursor = MonitorCursor {
            backend: self.id,
            ordinal: records.monitor_ordinal,
        };
        records.monitor_changes.push_back(MonitorChange {
            cursor,
            action: id,
            monitoring,
        });
        if records.monitor_changes.len() > self.config.retention {
            records.monitor_dropped = records.monitor_changes.pop_front().unwrap().cursor.ordinal;
        }
    }
}
