//! Actor-owned exclusive normal-dispatch gate for bounded common movement/mining.
use super::*;
type Admission<T> = Result<T, OperationAdmissionError>;

pub(super) enum MotionCommand {
    CursorReturn {
        run_id: u64,
        step: u16,
        slot: u16,
        comparison: Option<crate::versions::java_1_16_1::ItemStack>,
        reply: oneshot::Sender<crate::Result<()>>,
    },
    FinishCursorReturn {
        run_id: u64,
        step: u16,
        reply: oneshot::Sender<Admission<()>>,
    },
    CursorClose {
        run_id: u64,
        reply: oneshot::Sender<crate::Result<()>>,
    },
    FinishCursorClose {
        run_id: u64,
        reply: oneshot::Sender<Admission<()>>,
    },
    RecipePlacement {
        run_id: u64,
        expected_revision: u64,
        payload: Vec<u8>,
        reply: oneshot::Sender<crate::Result<()>>,
    },
    FinishRecipePlacement {
        run_id: u64,
        reply: oneshot::Sender<Admission<()>>,
    },
    OpenContainer {
        run_id: u64,
        expected_revision: u64,
        target: crate::BlockPos,
        face: u8,
        cursor: [f32; 3],
        reply: oneshot::Sender<crate::Result<()>>,
    },
    FinishContainerOpen {
        run_id: u64,
        reply: oneshot::Sender<Admission<()>>,
    },
    #[cfg(test)]
    CloseContainer {
        expected_revision: u64,
        window: i8,
        reply: oneshot::Sender<crate::Result<()>>,
    },
    Revision {
        reply: oneshot::Sender<Admission<u64>>,
    },
    Begin {
        run_id: u64,
        expected_revision: u64,
        reply: oneshot::Sender<Admission<()>>,
    },
    Position {
        run_id: u64,
        payload: Vec<u8>,
        reply: oneshot::Sender<crate::Result<()>>,
    },
    MiningBegin {
        run_id: u64,
        expected_revision: u64,
        target: crate::BlockPos,
        face: u8,
        reply: oneshot::Sender<Admission<()>>,
    },
    MiningDig {
        run_id: u64,
        action: crate::client::survival::MiningAction,
        reply: oneshot::Sender<crate::Result<()>>,
    },
    Place {
        run_id: u64,
        expected_revision: u64,
        support: crate::BlockPos,
        face: u8,
        cursor: [f32; 3],
        reply: oneshot::Sender<crate::Result<()>>,
    },
    FinishPlacement {
        run_id: u64,
        reply: oneshot::Sender<Admission<()>>,
    },
    InventoryClick {
        run_id: u64,
        slot: u16,
        button: u8,
        comparison: Option<crate::versions::java_1_16_1::ItemStack>,
        reply: oneshot::Sender<crate::Result<()>>,
    },
    InventoryTransfer {
        run_id: u64,
        slot: u16,
        button: u8,
        comparison: Option<crate::versions::java_1_16_1::ItemStack>,
        reply: oneshot::Sender<crate::Result<()>>,
    },
    FinishInventoryClick {
        run_id: u64,
        reply: oneshot::Sender<Admission<()>>,
    },
    FinishInventoryTransfer {
        run_id: u64,
        reply: oneshot::Sender<Admission<()>>,
    },
    InventorySwap {
        run_id: u64,
        main_slot: u16,
        hotbar: u8,
        comparison: crate::versions::java_1_16_1::ItemStack,
        reply: oneshot::Sender<crate::Result<()>>,
    },
    FinishInventorySwap {
        run_id: u64,
        reply: oneshot::Sender<Admission<()>>,
    },
    Finish {
        run_id: u64,
        reply: oneshot::Sender<Admission<()>>,
    },
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Owner {
    CursorClose {
        run_id: u64,
        window: i8,
        steps: u16,
        next: u16,
        active: Option<CursorStep>,
        closed: bool,
    },
    RecipePlacement {
        run_id: u64,
        sent: bool,
    },
    ContainerOpen {
        run_id: u64,
        sent: bool,
    },
    Motion(u64),
    Placement(u64),
    InventoryClick {
        result_take: bool,
        run_id: u64,
        action: i16,
        window: i8,
        sent: bool,
    },
    InventoryTransfer {
        run_id: u64,
        action: i16,
        window: i8,
        sent: bool,
    },
    InventorySwap {
        run_id: u64,
        action: i16,
        window: i8,
        sent: bool,
    },
    Mining {
        run_id: u64,
        target: crate::BlockPos,
        face: u8,
        started: bool,
        finished: bool,
        aborted: bool,
    },
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CursorStep {
    number: u16,
    action: i16,
    sent: bool,
}
#[derive(Default)]
pub(super) struct MotionGate {
    owner: Option<Owner>,
    normal_revision: u64,
}
impl MotionGate {
    pub(super) async fn begin_cursor_close(
        &mut self,
        identity: (u64, i8, u16),
        expected_revision: u64,
        state: ConnectionState,
        control: &Arc<RwLock<crate::snapshot::Versioned<crate::ControlState>>>,
        pending: bool,
    ) -> Admission<()> {
        self.can_begin(state, control, pending).await?;
        let (run_id, window, steps) = identity;
        if run_id == 0 || window <= 0 || steps > 36 || expected_revision != self.normal_revision {
            return Err(OperationAdmissionError::InvalidOperation);
        }
        self.normal_revision = self
            .normal_revision
            .checked_add(1)
            .ok_or(OperationAdmissionError::InvalidOperation)?;
        self.owner = Some(Owner::CursorClose {
            run_id,
            window,
            steps,
            next: 0,
            active: None,
            closed: false,
        });
        Ok(())
    }
    pub(super) fn reserve_cursor_return(
        &mut self,
        identity: (u64, u16),
        state: ConnectionState,
        actions: &mut HashMap<i8, i16>,
    ) -> Admission<i16> {
        admit_lifecycle(state, OperationClass::Normal)?;
        let Some(Owner::CursorClose {
            run_id,
            window,
            steps,
            next,
            active,
            closed,
        }) = self.owner.as_mut()
        else {
            return Err(OperationAdmissionError::InvalidOperation);
        };
        if *run_id != identity.0
            || *next != identity.1
            || *next >= *steps
            || active.is_some()
            || *closed
        {
            return Err(OperationAdmissionError::InvalidOperation);
        }
        let previous = actions.entry(*window).or_insert(0);
        let action = previous
            .checked_add(1)
            .filter(|a| *a > 0)
            .ok_or(OperationAdmissionError::InvalidOperation)?;
        *previous = action;
        *active = Some(CursorStep {
            number: identity.1,
            action,
            sent: false,
        });
        Ok(action)
    }

    pub(super) async fn begin_inventory_swap(
        &mut self,
        identity: (u64, i8),
        expected_revision: u64,
        state: ConnectionState,
        control: &Arc<RwLock<crate::snapshot::Versioned<crate::ControlState>>>,
        pending: bool,
        next_actions: &mut HashMap<i8, i16>,
    ) -> Admission<i16> {
        let (run_id, window) = identity;
        self.can_begin(state, control, pending).await?;
        if run_id == 0 || window < 0 || expected_revision != self.normal_revision {
            return Err(OperationAdmissionError::InvalidOperation);
        }
        let next = next_actions.entry(window).or_insert(0);
        // Never recycle a transaction number for common exchanges in this source.
        let action = next
            .checked_add(1)
            .filter(|v| *v > 0)
            .ok_or(OperationAdmissionError::InvalidOperation)?;
        *next = action;
        self.owner = Some(Owner::InventorySwap {
            run_id,
            action,
            window,
            sent: false,
        });
        Ok(action)
    }
    pub(super) async fn begin_inventory_click(
        &mut self,
        identity: (u64, i8, bool),
        expected_revision: u64,
        state: ConnectionState,
        control: &Arc<RwLock<crate::snapshot::Versioned<crate::ControlState>>>,
        pending: bool,
        next_actions: &mut HashMap<i8, i16>,
    ) -> Admission<i16> {
        let (run_id, window, result_take) = identity;
        self.can_begin(state, control, pending).await?;
        if run_id == 0 || window < 0 || expected_revision != self.normal_revision {
            return Err(OperationAdmissionError::InvalidOperation);
        }
        let next = next_actions.entry(window).or_insert(0);
        // Never recycle a transaction number for common exchanges in this source.
        let action = next
            .checked_add(1)
            .filter(|v| *v > 0)
            .ok_or(OperationAdmissionError::InvalidOperation)?;
        *next = action;
        self.owner = Some(Owner::InventoryClick {
            result_take,
            run_id,
            action,
            window,
            sent: false,
        });
        Ok(action)
    }
    pub(super) async fn begin_inventory_transfer(
        &mut self,
        identity: (u64, i8),
        expected_revision: u64,
        state: ConnectionState,
        control: &Arc<RwLock<crate::snapshot::Versioned<crate::ControlState>>>,
        pending: bool,
        next_actions: &mut HashMap<i8, i16>,
    ) -> Admission<i16> {
        let (run_id, window) = identity;
        self.can_begin(state, control, pending).await?;
        if run_id == 0 || window < 0 || expected_revision != self.normal_revision {
            return Err(OperationAdmissionError::InvalidOperation);
        }
        let next = next_actions.entry(window).or_insert(0);
        // Never recycle a transaction number for common exchanges in this source.
        let action = next
            .checked_add(1)
            .filter(|v| *v > 0)
            .ok_or(OperationAdmissionError::InvalidOperation)?;
        *next = action;
        self.owner = Some(Owner::InventoryTransfer {
            run_id,
            action,
            window,
            sent: false,
        });
        Ok(action)
    }
    pub(super) fn normal_admission(&mut self, class: OperationClass) -> Admission<()> {
        if class == OperationClass::Normal {
            if self.owner.is_some() {
                return Err(self.owner_error());
            }
            self.normal_revision = self
                .normal_revision
                .checked_add(1)
                .ok_or(OperationAdmissionError::InvalidOperation)?;
        }
        Ok(())
    }
    pub(super) fn admit(
        &mut self,
        generation: ConnectionGeneration,
        state: ConnectionState,
        active_output: Option<u64>,
        context: OperationContext,
        class: OperationClass,
    ) -> Admission<()> {
        super::admit(generation, state, active_output, context, class)?;
        self.normal_admission(class)
    }
    // Returns true only after possible delivery fails: the caller terminates the actor.
    pub(super) async fn process(
        &mut self,
        command: MotionCommand,
        state: ConnectionState,
        writer: &Arc<Mutex<crate::versions::java_1_16_1::client::PacketWriter>>,
        control: &Arc<RwLock<crate::snapshot::Versioned<crate::ControlState>>>,
        pending: bool,
    ) -> bool {
        match command {
            MotionCommand::CursorReturn {
                run_id,
                step,
                slot,
                comparison,
                reply,
            } => {
                let payload = admit_lifecycle(state, OperationClass::Normal).and_then(|()| {
                    let Some(Owner::CursorClose {
                        run_id: id,
                        window,
                        active: Some(active),
                        closed: false,
                        ..
                    }) = self.owner.as_mut()
                    else {
                        return Err(OperationAdmissionError::InvalidOperation);
                    };
                    if *id != run_id
                        || active.number != step
                        || active.sent
                        || slot >= 4096
                        || comparison.as_ref().is_some_and(|s| {
                            !crate::client::inventory::transfer_policy::legacy_comparison_supported(
                                s,
                            )
                        })
                    {
                        return Err(OperationAdmissionError::InvalidOperation);
                    }
                    active.sent = true;
                    let mut payload = vec![*window as u8];
                    payload.extend((slot as i16).to_be_bytes());
                    payload.push(0);
                    payload.extend(active.action.to_be_bytes());
                    payload.push(0);
                    crate::versions::java_1_16_1::inventory::write_slot(
                        &mut payload,
                        comparison.as_ref(),
                    );
                    Ok(payload)
                });
                let admitted = payload.is_ok();
                let result = match payload {
                    Err(e) => Err(crate::client::inventory::unavailable(format!(
                        "cursor return actor rejected: {e:?}"
                    ))),
                    Ok(payload) => {
                        let mut writer = writer.lock().await;
                        let compression = writer.compression;
                        crate::protocol::write_packet(
                            &mut writer.inner,
                            compression,
                            0x09,
                            &payload,
                        )
                        .await
                        .map_err(crate::Error::from)
                    }
                };
                let failed = admitted && result.is_err();
                let _ = reply.send(result);
                return failed;
            }
            MotionCommand::FinishCursorReturn {
                run_id,
                step,
                reply,
            } => {
                let result = admit_lifecycle(state, OperationClass::Normal).and_then(|()| {
                    let Some(Owner::CursorClose {
                        run_id: id,
                        next,
                        active,
                        closed: false,
                        ..
                    }) = self.owner.as_mut()
                    else {
                        return Err(OperationAdmissionError::InvalidOperation);
                    };
                    if *id != run_id || !active.is_some_and(|a| a.number == step && a.sent) {
                        return Err(OperationAdmissionError::InvalidOperation);
                    }
                    *next = next
                        .checked_add(1)
                        .ok_or(OperationAdmissionError::InvalidOperation)?;
                    *active = None;
                    Ok(())
                });
                let _ = reply.send(result);
            }
            MotionCommand::CursorClose { run_id, reply } => {
                let payload = admit_lifecycle(state, OperationClass::Normal).and_then(|()| {
                    let Some(Owner::CursorClose {
                        run_id: id,
                        window,
                        steps,
                        next,
                        active: None,
                        closed,
                    }) = self.owner.as_mut()
                    else {
                        return Err(OperationAdmissionError::InvalidOperation);
                    };
                    if *id != run_id || *closed || *next != *steps {
                        return Err(OperationAdmissionError::InvalidOperation);
                    }
                    *closed = true;
                    Ok([*window as u8])
                });
                let admitted = payload.is_ok();
                let result = match payload {
                    Err(e) => Err(crate::client::inventory::unavailable(format!(
                        "owned close actor rejected: {e:?}"
                    ))),
                    Ok(payload) => {
                        let mut writer = writer.lock().await;
                        let compression = writer.compression;
                        crate::protocol::write_packet(
                            &mut writer.inner,
                            compression,
                            0x0a,
                            &payload,
                        )
                        .await
                        .map_err(crate::Error::from)
                    }
                };
                let failed = admitted && result.is_err();
                let _ = reply.send(result);
                return failed;
            }
            MotionCommand::FinishCursorClose { run_id, reply } => {
                let result = admit_lifecycle(state, OperationClass::Normal).and_then(|()| {
                    if !matches!(self.owner, Some(Owner::CursorClose { run_id: id, closed: true, active: None, .. }) if id == run_id) { return Err(OperationAdmissionError::InvalidOperation); }
                    self.owner = None; Ok(())
                });
                let _ = reply.send(result);
            }
            MotionCommand::RecipePlacement {
                run_id,
                expected_revision,
                payload,
                reply,
            } => {
                let valid_payload = (|| -> crate::Result<()> {
                    let mut bytes = payload.as_slice();
                    if bytes.is_empty() || bytes[0] > 127 {
                        return Err(crate::client::inventory::unavailable(
                            "recipe placement invalid window",
                        ));
                    }
                    bytes = &bytes[1..];
                    let name = crate::protocol::get_string(&mut bytes)?;
                    if name.is_empty() || bytes.len() != 1 || bytes[0] > 1 {
                        return Err(crate::client::inventory::unavailable(
                            "recipe placement invalid identity/amount",
                        ));
                    }
                    Ok(())
                })();
                let admission = self
                    .can_begin(state, control, pending)
                    .await
                    .and_then(|()| {
                        if run_id == 0
                            || expected_revision != self.normal_revision
                            || valid_payload.is_err()
                        {
                            return Err(OperationAdmissionError::InvalidOperation);
                        }
                        self.normal_revision = self
                            .normal_revision
                            .checked_add(1)
                            .ok_or(OperationAdmissionError::InvalidOperation)?;
                        self.owner = Some(Owner::RecipePlacement {
                            run_id,
                            sent: false,
                        });
                        Ok(())
                    });
                let admitted = admission.is_ok();
                let result = match admission {
                    Err(e) => Err(crate::client::inventory::unavailable(format!(
                        "recipe placement admission: {e:?}"
                    ))),
                    Ok(()) => {
                        let mut writer = writer.lock().await;
                        let compression = writer.compression;
                        let result = crate::protocol::write_packet(
                            &mut writer.inner,
                            compression,
                            0x19,
                            &payload,
                        )
                        .await
                        .map_err(crate::Error::from);
                        if result.is_ok() {
                            self.owner = Some(Owner::RecipePlacement { run_id, sent: true });
                        }
                        result
                    }
                };
                let failed = admitted && result.is_err();
                let _ = reply.send(result);
                return failed;
            }
            MotionCommand::FinishRecipePlacement { run_id, reply } => {
                let result = admit_lifecycle(state, OperationClass::Normal).and_then(|()| {
                    if self.owner != Some(Owner::RecipePlacement { run_id, sent: true }) {
                        return Err(OperationAdmissionError::InvalidOperation);
                    }
                    self.owner = None;
                    Ok(())
                });
                let _ = reply.send(result);
            }
            MotionCommand::OpenContainer {
                run_id,
                expected_revision,
                target,
                face,
                cursor,
                reply,
            } => {
                let admission = self
                    .can_begin(state, control, pending)
                    .await
                    .and_then(|()| {
                        if run_id == 0
                            || expected_revision != self.normal_revision
                            || face > 5
                            || !(0..=255).contains(&target.y)
                            || target.x.abs_diff(0) > 30_000_000
                            || target.z.abs_diff(0) > 30_000_000
                            || cursor
                                .iter()
                                .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
                        {
                            return Err(OperationAdmissionError::InvalidOperation);
                        }
                        self.normal_revision = self
                            .normal_revision
                            .checked_add(1)
                            .ok_or(OperationAdmissionError::InvalidOperation)?;
                        self.owner = Some(Owner::ContainerOpen {
                            run_id,
                            sent: false,
                        });
                        Ok(())
                    });
                let admitted = admission.is_ok();
                let result = match admission {
                    Err(e) => Err(crate::client::inventory::unavailable(format!(
                        "storage open admission: {e:?}"
                    ))),
                    Ok(()) => {
                        let mut payload = vec![0];
                        payload.extend(target.packed().to_be_bytes());
                        payload.push(face);
                        for v in cursor {
                            payload.extend(v.to_be_bytes());
                        }
                        payload.push(0);
                        let mut writer = writer.lock().await;
                        let compression = writer.compression;
                        let result = crate::protocol::write_packet(
                            &mut writer.inner,
                            compression,
                            0x2d,
                            &payload,
                        )
                        .await
                        .map_err(crate::Error::from);
                        if result.is_ok() {
                            self.owner = Some(Owner::ContainerOpen { run_id, sent: true });
                        }
                        result
                    }
                };
                let failed = admitted && result.is_err();
                let _ = reply.send(result);
                return failed;
            }
            MotionCommand::FinishContainerOpen { run_id, reply } => {
                let result = admit_lifecycle(state, OperationClass::Normal).and_then(|()| {
                    if self.owner != Some(Owner::ContainerOpen { run_id, sent: true }) {
                        return Err(OperationAdmissionError::InvalidOperation);
                    }
                    self.owner = None;
                    Ok(())
                });
                let _ = reply.send(result);
            }
            #[cfg(test)]
            MotionCommand::CloseContainer {
                expected_revision,
                window,
                reply,
            } => {
                let admission = self
                    .can_begin(state, control, pending)
                    .await
                    .and_then(|()| {
                        if window <= 0 || expected_revision != self.normal_revision {
                            return Err(OperationAdmissionError::InvalidOperation);
                        }
                        self.normal_revision = self
                            .normal_revision
                            .checked_add(1)
                            .ok_or(OperationAdmissionError::InvalidOperation)?;
                        Ok(())
                    });
                let result = match admission {
                    Err(error) => Err(crate::client::inventory::unavailable(format!(
                        "close admission: {error:?}"
                    ))),
                    Ok(()) => {
                        let mut writer = writer.lock().await;
                        let compression = writer.compression;
                        crate::protocol::write_packet(
                            &mut writer.inner,
                            compression,
                            0x0a,
                            &[window as u8],
                        )
                        .await
                        .map_err(crate::Error::from)
                    }
                };
                let failed_write = admission.is_ok() && result.is_err();
                let _ = reply.send(result);
                return failed_write;
            }
            MotionCommand::Revision { reply } => {
                let result = self
                    .can_begin(state, control, pending)
                    .await
                    .map(|()| self.normal_revision);
                let _ = reply.send(result);
            }
            MotionCommand::Begin {
                run_id,
                expected_revision,
                reply,
            } => {
                let result = self
                    .can_begin(state, control, pending)
                    .await
                    .and_then(|()| {
                        if run_id == 0 || expected_revision != self.normal_revision {
                            return Err(OperationAdmissionError::InvalidOperation);
                        }
                        self.owner = Some(Owner::Motion(run_id));
                        Ok(())
                    });
                let _ = reply.send(result);
            }
            MotionCommand::Position {
                run_id,
                payload,
                reply,
            } => {
                let admission = admit_lifecycle(state, OperationClass::Normal).and_then(|()| {
                    if self.owner != Some(Owner::Motion(run_id)) || payload.len() != 33 {
                        return Err(OperationAdmissionError::InvalidOperation);
                    }
                    Ok(())
                });
                let result = match admission {
                    Err(error) => Err(crate::Error::new(
                        crate::ErrorKind::State,
                        anyhow::anyhow!("bounded movement rejected: {error:?}"),
                    )),
                    Ok(()) => {
                        let mut writer = writer.lock().await;
                        let compression = writer.compression;
                        crate::protocol::write_packet(
                            &mut writer.inner,
                            compression,
                            0x13,
                            &payload,
                        )
                        .await
                        .map_err(crate::Error::from)
                    }
                };
                let failed_write = admission.is_ok() && result.is_err();
                let _ = reply.send(result);
                return failed_write;
            }
            MotionCommand::MiningBegin {
                run_id,
                expected_revision,
                target,
                face,
                reply,
            } => {
                let result = self
                    .can_begin(state, control, pending)
                    .await
                    .and_then(|()| {
                        if run_id == 0
                            || expected_revision != self.normal_revision
                            || face > 5
                            || !(0..=255).contains(&target.y)
                            || target.x.abs_diff(0) > 30_000_000
                            || target.z.abs_diff(0) > 30_000_000
                        {
                            return Err(OperationAdmissionError::InvalidOperation);
                        }
                        self.owner = Some(Owner::Mining {
                            run_id,
                            target,
                            face,
                            started: false,
                            finished: false,
                            aborted: false,
                        });
                        Ok(())
                    });
                let _ = reply.send(result);
            }
            MotionCommand::MiningDig {
                run_id,
                action,
                reply,
            } => {
                let payload = admit_lifecycle(state, OperationClass::Normal).and_then(|()| {
                    let Some(Owner::Mining {
                        run_id: id,
                        target,
                        face,
                        started,
                        finished,
                        aborted,
                    }) = self.owner.as_mut()
                    else {
                        return Err(OperationAdmissionError::InvalidOperation);
                    };
                    if *id != run_id {
                        return Err(OperationAdmissionError::InvalidOperation);
                    }
                    use crate::client::survival::MiningAction::*;
                    match action {
                        Start if !*started && !*finished && !*aborted => *started = true,
                        Finish if *started && !*finished && !*aborted => *finished = true,
                        Abort if *started && !*aborted => *aborted = true,
                        _ => return Err(OperationAdmissionError::InvalidOperation),
                    }
                    // Actor records the stage BEFORE writer acquisition. Waiter cancellation
                    // cannot send it twice or release ownership. Legacy has no interaction seq.
                    let mut payload = vec![action as u8];
                    payload.extend(target.packed().to_be_bytes());
                    payload.push(*face);
                    Ok(payload)
                });
                let admitted = payload.is_ok();
                let result = match payload {
                    Err(e) => Err(crate::Error::new(
                        crate::ErrorKind::State,
                        anyhow::anyhow!("bounded mining rejected: {e:?}"),
                    )),
                    Ok(payload) => {
                        let mut writer = writer.lock().await;
                        let compression = writer.compression;
                        crate::protocol::write_packet(
                            &mut writer.inner,
                            compression,
                            0x1b,
                            &payload,
                        )
                        .await
                        .map_err(crate::Error::from)
                    }
                };
                let failed_write = admitted && result.is_err();
                let _ = reply.send(result);
                return failed_write;
            }
            MotionCommand::Place {
                run_id,
                expected_revision,
                support,
                face,
                cursor,
                reply,
            } => {
                let admission = self
                    .can_begin(state, control, pending)
                    .await
                    .and_then(|()| {
                        if run_id == 0
                            || expected_revision != self.normal_revision
                            || face > 5
                            || !(0..=255).contains(&support.y)
                            || support.x.abs_diff(0) > 30_000_000
                            || support.z.abs_diff(0) > 30_000_000
                            || cursor
                                .iter()
                                .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
                        {
                            return Err(OperationAdmissionError::InvalidOperation);
                        }
                        // Retain ownership before writer acquisition. No replay or alias release.
                        self.owner = Some(Owner::Placement(run_id));
                        Ok(())
                    });
                let admitted = admission.is_ok();
                let result = match admission {
                    Err(e) => Err(crate::Error::new(
                        crate::ErrorKind::State,
                        anyhow::anyhow!("bounded placement rejected: {e:?}"),
                    )),
                    Ok(()) => {
                        let mut payload = vec![0];
                        payload.extend(support.packed().to_be_bytes());
                        payload.push(face);
                        for v in cursor {
                            payload.extend(v.to_be_bytes());
                        }
                        payload.push(0);
                        let mut writer = writer.lock().await;
                        let compression = writer.compression;
                        crate::protocol::write_packet(
                            &mut writer.inner,
                            compression,
                            0x2d,
                            &payload,
                        )
                        .await
                        .map_err(crate::Error::from)
                    }
                };
                let failed = admitted && result.is_err();
                let _ = reply.send(result);
                return failed;
            }
            MotionCommand::InventorySwap {
                run_id,
                main_slot,
                hotbar,
                comparison,
                reply,
            } => {
                let payload = admit_lifecycle(state, OperationClass::Normal).and_then(|()| {
                    let Some(Owner::InventorySwap {
                        run_id: id,
                        action,
                        window,
                        sent,
                    }) = self.owner.as_mut()
                    else {
                        return Err(OperationAdmissionError::InvalidOperation);
                    };
                    if *id != run_id
                        || *sent
                        || (*window == 0 && !(9..=35).contains(&main_slot))
                        || main_slot >= 4096
                        || hotbar > 8
                        || comparison.item_id < 0
                        || comparison.count <= 0
                        || comparison.nbt.as_ref().is_some_and(|bytes| {
                            crate::client::nbt::decode(bytes, crate::MinecraftVersion::Java1_16_1)
                                .is_err()
                        })
                    {
                        return Err(OperationAdmissionError::InvalidOperation);
                    }
                    *sent = true;
                    let mut payload = vec![*window as u8];
                    payload.extend((main_slot as i16).to_be_bytes());
                    payload.push(hotbar);
                    payload.extend(action.to_be_bytes());
                    payload.push(2);
                    // Native applies SWAP before comparing its empty returned
                    // stack. A received nonempty predecessor requests full
                    // native resync, rather than inventing predicted receipts.
                    crate::versions::java_1_16_1::inventory::write_slot(
                        &mut payload,
                        Some(&comparison),
                    );
                    Ok(payload)
                });
                let admitted = payload.is_ok();
                let result = match payload {
                    Err(e) => Err(crate::client::inventory::unavailable(format!(
                        "inventory swap actor rejected: {e:?}"
                    ))),
                    Ok(payload) => {
                        let mut writer = writer.lock().await;
                        let compression = writer.compression;
                        crate::protocol::write_packet(
                            &mut writer.inner,
                            compression,
                            0x09,
                            &payload,
                        )
                        .await
                        .map_err(crate::Error::from)
                    }
                };
                let failed = admitted && result.is_err();
                let _ = reply.send(result);
                return failed;
            }
            MotionCommand::InventoryClick {
                run_id,
                slot,
                button,
                comparison,
                reply,
            } => {
                let payload = admit_lifecycle(state, OperationClass::Normal).and_then(|()| {
                    let Some(Owner::InventoryClick {
                        result_take,
                        run_id: id,
                        action,
                        window,
                        sent,
                    }) = self.owner.as_mut()
                    else {
                        return Err(OperationAdmissionError::InvalidOperation);
                    };
                    if *id != run_id
                        || *sent
                        || if *result_take {
                            slot != 0 || button != 0
                        } else {
                            *window == 0 && !(1..=4).contains(&slot) && !(9..=44).contains(&slot)
                        }
                        || slot >= 4096
                        || button > 1
                        || comparison.as_ref().is_some_and(|s| {
                            !crate::client::inventory::transfer_policy::legacy_comparison_supported(
                                s,
                            )
                        })
                    {
                        return Err(OperationAdmissionError::InvalidOperation);
                    }
                    *sent = true;
                    let mut payload = vec![*window as u8];
                    payload.extend((slot as i16).to_be_bytes());
                    payload.push(button);
                    payload.extend(action.to_be_bytes());
                    payload.push(0); // Original PICKUP, never SWAP/creative creation.
                    crate::versions::java_1_16_1::inventory::write_slot(
                        &mut payload,
                        comparison.as_ref(),
                    );
                    Ok(payload)
                });
                let admitted = payload.is_ok();
                let result = match payload {
                    Err(e) => Err(crate::client::inventory::unavailable(format!(
                        "inventory click actor rejected: {e:?}"
                    ))),
                    Ok(payload) => {
                        let mut writer = writer.lock().await;
                        let compression = writer.compression;
                        crate::protocol::write_packet(
                            &mut writer.inner,
                            compression,
                            0x09,
                            &payload,
                        )
                        .await
                        .map_err(crate::Error::from)
                    }
                };
                let failed = admitted && result.is_err();
                let _ = reply.send(result);
                return failed;
            }
            MotionCommand::FinishInventoryClick { run_id, reply } => {
                let result = admit_lifecycle(state, OperationClass::Normal).and_then(|()| {
                    if !matches!(self.owner, Some(Owner::InventoryClick { run_id: id, sent: true, .. }) if id == run_id) {
                        return Err(OperationAdmissionError::InvalidOperation);
                    }
                    self.owner = None; Ok(())
                });
                let _ = reply.send(result);
            }
            MotionCommand::InventoryTransfer {
                run_id,
                slot,
                button,
                comparison,
                reply,
            } => {
                let payload = admit_lifecycle(state, OperationClass::Normal).and_then(|()| {
                    let Some(Owner::InventoryTransfer {
                        run_id: id,
                        action,
                        window,
                        sent,
                    }) = self.owner.as_mut()
                    else {
                        return Err(OperationAdmissionError::InvalidOperation);
                    };
                    if *id != run_id
                        || *sent
                        || (*window == 0 && !(5..=45).contains(&slot))
                        || slot >= 4096
                        || button != 0
                        || comparison.as_ref().is_some_and(|s| {
                            !crate::client::inventory::transfer_policy::legacy_comparison_supported(
                                s,
                            )
                        })
                    {
                        return Err(OperationAdmissionError::InvalidOperation);
                    }
                    *sent = true;
                    let mut payload = vec![*window as u8];
                    payload.extend((slot as i16).to_be_bytes());
                    payload.push(button);
                    payload.extend(action.to_be_bytes());
                    payload.push(1); // Original QUICK_MOVE, one frame including native internal rounds.
                    crate::versions::java_1_16_1::inventory::write_slot(
                        &mut payload,
                        comparison.as_ref(),
                    );
                    Ok(payload)
                });
                let admitted = payload.is_ok();
                let result = match payload {
                    Err(e) => Err(crate::client::inventory::unavailable(format!(
                        "inventory click actor rejected: {e:?}"
                    ))),
                    Ok(payload) => {
                        let mut writer = writer.lock().await;
                        let compression = writer.compression;
                        crate::protocol::write_packet(
                            &mut writer.inner,
                            compression,
                            0x09,
                            &payload,
                        )
                        .await
                        .map_err(crate::Error::from)
                    }
                };
                let failed = admitted && result.is_err();
                let _ = reply.send(result);
                return failed;
            }
            MotionCommand::FinishInventoryTransfer { run_id, reply } => {
                let result = admit_lifecycle(state, OperationClass::Normal).and_then(|()| {
                    if !matches!(self.owner, Some(Owner::InventoryTransfer { run_id: id, sent: true, .. }) if id == run_id) {
                        return Err(OperationAdmissionError::InvalidOperation);
                    }
                    self.owner = None; Ok(())
                });
                let _ = reply.send(result);
            }
            MotionCommand::FinishInventorySwap { run_id, reply } => {
                let result=admit_lifecycle(state,OperationClass::Normal).and_then(|()|{
                    if !matches!(self.owner,Some(Owner::InventorySwap{run_id:id,sent:true,..}) if id==run_id){return Err(OperationAdmissionError::InvalidOperation)}
                    self.owner=None;Ok(())
                });
                let _ = reply.send(result);
            }
            MotionCommand::FinishPlacement { run_id, reply } => {
                let result = admit_lifecycle(state, OperationClass::Normal).and_then(|()| {
                    if self.owner != Some(Owner::Placement(run_id)) {
                        return Err(OperationAdmissionError::InvalidOperation);
                    }
                    self.owner = None;
                    Ok(())
                });
                let _ = reply.send(result);
            }
            MotionCommand::Finish { run_id, reply } => {
                let result = admit_lifecycle(state, OperationClass::Normal).and_then(|()| {
                    if self.owner != Some(Owner::Motion(run_id)) {
                        return Err(OperationAdmissionError::InvalidOperation);
                    }
                    self.owner = None;
                    Ok(())
                });
                let _ = reply.send(result);
            }
        }
        false
    }
    fn owner_error(&self) -> OperationAdmissionError {
        match self.owner {
            Some(Owner::CursorClose { .. }) => {
                OperationAdmissionError::BoundedContainerCloseInProgress
            }
            Some(Owner::ContainerOpen { .. }) => {
                OperationAdmissionError::BoundedContainerOpenInProgress
            }
            Some(Owner::RecipePlacement { .. }) => {
                OperationAdmissionError::BoundedRecipePlacementInProgress
            }
            Some(Owner::InventoryTransfer { .. }) => {
                OperationAdmissionError::BoundedInventoryTransferInProgress
            }
            Some(Owner::InventoryClick { .. }) => {
                OperationAdmissionError::BoundedInventoryClickInProgress
            }
            Some(Owner::InventorySwap { .. }) => {
                OperationAdmissionError::BoundedInventorySwapInProgress
            }
            Some(Owner::Placement(_)) => OperationAdmissionError::BoundedPlacementInProgress,
            Some(Owner::Mining { .. }) => OperationAdmissionError::BoundedMiningInProgress,
            _ => OperationAdmissionError::BoundedMotionInProgress,
        }
    }
    async fn can_begin(
        &self,
        state: ConnectionState,
        control: &Arc<RwLock<crate::snapshot::Versioned<crate::ControlState>>>,
        pending: bool,
    ) -> Admission<()> {
        admit_lifecycle(state, OperationClass::Normal)?;
        if self.owner.is_some() {
            return Err(self.owner_error());
        }
        if pending || **control.read().await != crate::ControlState::default() {
            return Err(OperationAdmissionError::TransactionInProgress);
        }
        Ok(())
    }
}
impl ConnectionActor {
    pub(crate) async fn bounded_recipe_placement(
        &self,
        run_id: u64,
        expected_revision: u64,
        payload: Vec<u8>,
    ) -> crate::Result<()> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::Motion(MotionCommand::RecipePlacement {
                run_id,
                expected_revision,
                payload,
                reply,
            }))
            .await
            .map_err(|_| {
                crate::client::inventory::unavailable("recipe placement actor unavailable")
            })?;
        result.await.map_err(|_| {
            crate::client::inventory::unavailable("recipe placement actor result unavailable")
        })?
    }
    pub(crate) async fn finish_recipe_placement(&self, run_id: u64) -> Admission<()> {
        self.motion_admission(|reply| MotionCommand::FinishRecipePlacement { run_id, reply })
            .await
    }
    pub(crate) async fn bounded_container_open(
        &self,
        run_id: u64,
        expected_revision: u64,
        target: crate::BlockPos,
        face: u8,
        cursor: [f32; 3],
    ) -> crate::Result<()> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::Motion(MotionCommand::OpenContainer {
                run_id,
                expected_revision,
                target,
                face,
                cursor,
                reply,
            }))
            .await
            .map_err(|_| crate::client::inventory::unavailable("storage open actor unavailable"))?;
        result.await.map_err(|_| {
            crate::client::inventory::unavailable("storage open actor result unavailable")
        })?
    }
    pub(crate) async fn finish_container_open(&self, run_id: u64) -> Admission<()> {
        self.motion_admission(|reply| MotionCommand::FinishContainerOpen { run_id, reply })
            .await
    }
    #[cfg(test)]
    pub(crate) async fn bounded_container_close(
        &self,
        expected_revision: u64,
        window: i8,
    ) -> crate::Result<()> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::Motion(MotionCommand::CloseContainer {
                expected_revision,
                window,
                reply,
            }))
            .await
            .map_err(|_| crate::client::inventory::unavailable("close actor unavailable"))?;
        result
            .await
            .map_err(|_| crate::client::inventory::unavailable("close actor result unavailable"))?
    }
    #[cfg(test)]
    pub(crate) async fn begin_inventory_swap(
        &self,
        run_id: u64,
        expected_revision: u64,
    ) -> Admission<i16> {
        self.begin_window_swap(run_id, expected_revision, 0).await
    }
    pub(crate) async fn begin_window_swap(
        &self,
        run_id: u64,
        expected_revision: u64,
        window: i8,
    ) -> Admission<i16> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::BeginInventorySwap {
                run_id,
                expected_revision,
                window,
                reply,
            })
            .await
            .map_err(|_| self.terminal_admission_error())?;
        result.await.map_err(|_| self.terminal_admission_error())?
    }
    pub(crate) async fn bounded_inventory_swap(
        &self,
        run_id: u64,
        main_slot: u16,
        hotbar: u8,
        comparison: crate::versions::java_1_16_1::ItemStack,
    ) -> crate::Result<()> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::Motion(MotionCommand::InventorySwap {
                run_id,
                main_slot,
                hotbar,
                comparison,
                reply,
            }))
            .await
            .map_err(|_| crate::client::inventory::unavailable("inventory actor unavailable"))?;
        result.await.map_err(|_| {
            crate::client::inventory::unavailable("inventory actor result unavailable")
        })?
    }
    pub(crate) async fn finish_inventory_swap(&self, run_id: u64) -> Admission<()> {
        self.motion_admission(|reply| MotionCommand::FinishInventorySwap { run_id, reply })
            .await
    }
    pub(crate) async fn begin_window_click(
        &self,
        run_id: u64,
        expected_revision: u64,
        window: i8,
    ) -> Admission<i16> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::BeginInventoryClick {
                result_take: false,
                run_id,
                expected_revision,
                window,
                reply,
            })
            .await
            .map_err(|_| self.terminal_admission_error())?;
        result.await.map_err(|_| self.terminal_admission_error())?
    }
    pub(crate) async fn begin_crafting_result_take(
        &self,
        run_id: u64,
        expected_revision: u64,
        window: i8,
    ) -> Admission<i16> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::BeginInventoryClick {
                result_take: true,
                run_id,
                expected_revision,
                window,
                reply,
            })
            .await
            .map_err(|_| self.terminal_admission_error())?;
        result.await.map_err(|_| self.terminal_admission_error())?
    }
    pub(crate) async fn bounded_inventory_click(
        &self,
        run_id: u64,
        slot: u16,
        button: u8,
        comparison: Option<crate::versions::java_1_16_1::ItemStack>,
    ) -> crate::Result<()> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::Motion(MotionCommand::InventoryClick {
                run_id,
                slot,
                button,
                comparison,
                reply,
            }))
            .await
            .map_err(|_| {
                crate::client::inventory::unavailable("inventory click actor unavailable")
            })?;
        result.await.map_err(|_| {
            crate::client::inventory::unavailable("inventory click actor result unavailable")
        })?
    }
    pub(crate) async fn finish_inventory_click(&self, run_id: u64) -> Admission<()> {
        self.motion_admission(|reply| MotionCommand::FinishInventoryClick { run_id, reply })
            .await
    }
    pub(crate) async fn begin_window_transfer(
        &self,
        run_id: u64,
        expected_revision: u64,
        window: i8,
    ) -> Admission<i16> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::BeginInventoryTransfer {
                run_id,
                expected_revision,
                window,
                reply,
            })
            .await
            .map_err(|_| self.terminal_admission_error())?;
        result.await.map_err(|_| self.terminal_admission_error())?
    }
    pub(crate) async fn bounded_inventory_transfer(
        &self,
        run_id: u64,
        slot: u16,
        button: u8,
        comparison: Option<crate::versions::java_1_16_1::ItemStack>,
    ) -> crate::Result<()> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::Motion(MotionCommand::InventoryTransfer {
                run_id,
                slot,
                button,
                comparison,
                reply,
            }))
            .await
            .map_err(|_| {
                crate::client::inventory::unavailable("inventory click actor unavailable")
            })?;
        result.await.map_err(|_| {
            crate::client::inventory::unavailable("inventory click actor result unavailable")
        })?
    }
    pub(crate) async fn finish_inventory_transfer(&self, run_id: u64) -> Admission<()> {
        self.motion_admission(|reply| MotionCommand::FinishInventoryTransfer { run_id, reply })
            .await
    }
    async fn motion_admission<T>(
        &self,
        make: impl FnOnce(oneshot::Sender<Admission<T>>) -> MotionCommand,
    ) -> Admission<T> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::Motion(make(reply)))
            .await
            .map_err(|_| self.terminal_admission_error())?;
        result.await.map_err(|_| self.terminal_admission_error())?
    }
    pub(crate) async fn motion_admission_revision(&self) -> Admission<u64> {
        self.motion_admission(|reply| MotionCommand::Revision { reply })
            .await
    }
    pub(crate) async fn bounded_placement(
        &self,
        run_id: u64,
        expected_revision: u64,
        support: crate::BlockPos,
        face: u8,
        cursor: [f32; 3],
    ) -> crate::Result<()> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::Motion(MotionCommand::Place {
                run_id,
                expected_revision,
                support,
                face,
                cursor,
                reply,
            }))
            .await
            .map_err(|_| {
                crate::client::survival::placement::unavailable("placement actor unavailable")
            })?;
        result.await.map_err(|_| {
            crate::client::survival::placement::unavailable("placement actor result missing")
        })?
    }
    pub(crate) async fn finish_bounded_placement(&self, run_id: u64) -> Admission<()> {
        self.motion_admission(|reply| MotionCommand::FinishPlacement { run_id, reply })
            .await
    }
    pub(crate) async fn begin_bounded_motion(
        &self,
        run_id: u64,
        expected_revision: u64,
    ) -> Admission<()> {
        self.motion_admission(|reply| MotionCommand::Begin {
            run_id,
            expected_revision,
            reply,
        })
        .await
    }
    pub(crate) async fn begin_bounded_mining(
        &self,
        run_id: u64,
        expected_revision: u64,
        target: crate::BlockPos,
        face: u8,
    ) -> Admission<()> {
        self.motion_admission(|reply| MotionCommand::MiningBegin {
            run_id,
            expected_revision,
            target,
            face,
            reply,
        })
        .await
    }
    pub(crate) async fn bounded_mining(
        &self,
        run_id: u64,
        action: crate::client::survival::MiningAction,
    ) -> crate::Result<()> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::Motion(MotionCommand::MiningDig {
                run_id,
                action,
                reply,
            }))
            .await
            .map_err(|_| {
                crate::client::survival::mining::unavailable("bounded mining actor unavailable")
            })?;
        result.await.map_err(|_| {
            crate::client::survival::mining::unavailable("bounded mining actor result missing")
        })?
    }
    pub(crate) async fn finish_bounded_motion(&self, run_id: u64) -> Admission<()> {
        self.motion_admission(|reply| MotionCommand::Finish { run_id, reply })
            .await
    }
    pub(crate) async fn bounded_position(
        &self,
        run_id: u64,
        payload: Vec<u8>,
    ) -> crate::Result<()> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::Motion(MotionCommand::Position {
                run_id,
                payload,
                reply,
            }))
            .await
            .map_err(|_| {
                crate::Error::new(
                    crate::ErrorKind::State,
                    anyhow::anyhow!("bounded motion actor unavailable"),
                )
            })?;
        result.await.map_err(|_| {
            crate::Error::new(
                crate::ErrorKind::State,
                anyhow::anyhow!("bounded motion actor result missing"),
            )
        })?
    }
}

impl ConnectionActor {
    pub(crate) async fn begin_cursor_close(
        &self,
        run_id: u64,
        revision: u64,
        window: i8,
        steps: u16,
    ) -> Admission<()> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::BeginCursorClose {
                identity: (run_id, window, steps),
                expected_revision: revision,
                reply,
            })
            .await
            .map_err(|_| self.terminal_admission_error())?;
        result.await.map_err(|_| self.terminal_admission_error())?
    }
    pub(crate) async fn reserve_cursor_return(&self, run_id: u64, step: u16) -> Admission<i16> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::ReserveCursorReturn {
                identity: (run_id, step),
                reply,
            })
            .await
            .map_err(|_| self.terminal_admission_error())?;
        result.await.map_err(|_| self.terminal_admission_error())?
    }
    pub(crate) async fn bounded_cursor_return(
        &self,
        run_id: u64,
        step: u16,
        slot: u16,
        comparison: Option<crate::versions::java_1_16_1::ItemStack>,
    ) -> crate::Result<()> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::Motion(MotionCommand::CursorReturn {
                run_id,
                step,
                slot,
                comparison,
                reply,
            }))
            .await
            .map_err(|_| {
                crate::client::inventory::unavailable("cursor return actor unavailable")
            })?;
        result.await.map_err(|_| {
            crate::client::inventory::unavailable("cursor return result unavailable")
        })?
    }
    pub(crate) async fn finish_cursor_return(&self, run_id: u64, step: u16) -> Admission<()> {
        self.motion_admission(|reply| MotionCommand::FinishCursorReturn {
            run_id,
            step,
            reply,
        })
        .await
    }
    pub(crate) async fn bounded_cursor_close(&self, run_id: u64) -> crate::Result<()> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::Motion(MotionCommand::CursorClose {
                run_id,
                reply,
            }))
            .await
            .map_err(|_| crate::client::inventory::unavailable("close actor unavailable"))?;
        result
            .await
            .map_err(|_| crate::client::inventory::unavailable("close result unavailable"))?
    }
    pub(crate) async fn finish_cursor_close(&self, run_id: u64) -> Admission<()> {
        self.motion_admission(|reply| MotionCommand::FinishCursorClose { run_id, reply })
            .await
    }
}
