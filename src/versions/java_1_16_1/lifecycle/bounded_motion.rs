//! Actor-owned exclusive normal-dispatch gate for finite common movement.
use super::*;
type Admission<T> = Result<T, OperationAdmissionError>;

pub(super) enum MotionCommand {
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
    Finish {
        run_id: u64,
        reply: oneshot::Sender<Admission<()>>,
    },
}
#[derive(Default)]
pub(super) struct MotionGate {
    owner: Option<u64>,
    normal_revision: u64,
}
impl MotionGate {
    pub(super) fn normal_admission(&mut self, class: OperationClass) -> Admission<()> {
        if class == OperationClass::Normal {
            if self.owner.is_some() {
                return Err(OperationAdmissionError::BoundedMotionInProgress);
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
                        self.owner = Some(run_id);
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
                    if self.owner != Some(run_id) || payload.len() != 33 {
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
            MotionCommand::Finish { run_id, reply } => {
                let result = admit_lifecycle(state, OperationClass::Normal).and_then(|()| {
                    if self.owner != Some(run_id) {
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
    async fn can_begin(
        &self,
        state: ConnectionState,
        control: &Arc<RwLock<crate::snapshot::Versioned<crate::ControlState>>>,
        pending: bool,
    ) -> Admission<()> {
        admit_lifecycle(state, OperationClass::Normal)?;
        if self.owner.is_some() {
            return Err(OperationAdmissionError::BoundedMotionInProgress);
        }
        if pending || **control.read().await != crate::ControlState::default() {
            return Err(OperationAdmissionError::TransactionInProgress);
        }
        Ok(())
    }
}
impl ConnectionActor {
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
