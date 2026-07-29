use super::*;

const LEASE_JOURNAL_MAGIC: &[u8; 8] = b"DSKTQL01";
const LEASE_JOURNAL_VERSION: u16 = 1;
const LEASE_JOURNAL_HEADER_BYTES: usize = 8 + 2 + 2;
const LEASE_RECORD_HEADER_BYTES: usize = 4 + 32;
const MAX_LEASE_RECORD_BYTES: usize = 1024 * 1024;
const LEASE_BATCH_SCHEMA: &[u8] = b"dusklight-native-tactic-lease-batch/v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeTacticLeaseOutcome {
    Completed,
    Retryable,
    Cancelled,
    Failed,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticLeaseAccounting {
    pub journal_sha256: Digest,
    pub proposal_dispatches: u64,
    pub completed_leases: u64,
    pub retryable_leases: u64,
    pub cancelled_leases: u64,
    pub failed_leases: u64,
    pub unresolved_leases: u64,
}

impl NativeTacticLeaseAccounting {
    pub fn resolved_leases(&self) -> u64 {
        self.completed_leases
            .saturating_add(self.retryable_leases)
            .saturating_add(self.cancelled_leases)
            .saturating_add(self.failed_leases)
    }

    pub fn validate(&self) -> Result<(), NativeTacticRouteRunError> {
        let accounted = self
            .completed_leases
            .checked_add(self.retryable_leases)
            .and_then(|total| total.checked_add(self.cancelled_leases))
            .and_then(|total| total.checked_add(self.failed_leases))
            .and_then(|total| total.checked_add(self.unresolved_leases));
        if self.journal_sha256 == Digest::ZERO || accounted != Some(self.proposal_dispatches) {
            return Err(route_message("native tactic lease accounting is invalid"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct NativeTacticLeaseIdentity {
    expansion_sha256: Digest,
    lease_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", deny_unknown_fields)]
enum NativeTacticLeaseEvent {
    Issued {
        execution_plan_sha256: Digest,
        decision_index: u64,
        batch_sha256: Digest,
        leases: Vec<NativeTacticLeaseIdentity>,
    },
    Resolved {
        batch_sha256: Digest,
        outcome: NativeTacticLeaseOutcome,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct NativeTacticLeaseRecord {
    event_index: u64,
    event: NativeTacticLeaseEvent,
}

#[derive(Clone, Debug)]
struct IssuedLeaseBatch {
    decision_index: u64,
    leases: Vec<NativeTacticLeaseIdentity>,
    outcome: Option<NativeTacticLeaseOutcome>,
}

pub(super) struct NativeTacticLeaseLedger {
    path: PathBuf,
    records: Vec<NativeTacticLeaseRecord>,
    batches: BTreeMap<Digest, IssuedLeaseBatch>,
}

impl NativeTacticLeaseLedger {
    pub(super) fn open(seed_root: &Path) -> Result<Self, NativeTacticRouteRunError> {
        fs::create_dir_all(seed_root).map_err(route_error)?;
        let path = seed_root.join(NATIVE_TACTIC_LEASE_JOURNAL_FILE);
        ensure_journal(&path)?;
        let metadata = fs::symlink_metadata(&path).map_err(route_error)?;
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return Err(route_message(
                "native tactic lease journal is not a physical file",
            ));
        }
        let bytes = fs::read(&path).map_err(route_error)?;
        let decoded = decode_journal(&bytes)?;
        if decoded.valid_bytes != bytes.len() {
            OpenOptions::new()
                .write(true)
                .open(&path)
                .and_then(|file| file.set_len(decoded.valid_bytes as u64))
                .map_err(route_error)?;
        }
        let batches = validate_records(&decoded.records)?;
        Ok(Self {
            path,
            records: decoded.records,
            batches,
        })
    }

    pub(super) fn reconcile_unresolved(
        &mut self,
        completed_expansions_by_decision: &BTreeMap<u64, Vec<Digest>>,
    ) -> Result<u64, NativeTacticRouteRunError> {
        let unresolved = self
            .batches
            .iter()
            .filter_map(|(batch, state)| {
                state.outcome.is_none().then_some((
                    *batch,
                    state.decision_index,
                    state
                        .leases
                        .iter()
                        .map(|lease| lease.expansion_sha256)
                        .collect::<Vec<_>>(),
                ))
            })
            .collect::<Vec<_>>();
        let mut leases = 0_u64;
        for (batch, decision_index, expansions) in unresolved {
            let outcome =
                if completed_expansions_by_decision.get(&decision_index) == Some(&expansions) {
                    NativeTacticLeaseOutcome::Completed
                } else {
                    NativeTacticLeaseOutcome::Retryable
                };
            leases = leases.saturating_add(self.resolve(batch, outcome)?);
        }
        Ok(leases)
    }

    pub(super) fn issue(
        &mut self,
        execution_plan_sha256: Digest,
        decision_index: u64,
        leases: &[TacticExpansionLease],
    ) -> Result<Digest, NativeTacticRouteRunError> {
        let identities = leases
            .iter()
            .map(|lease| NativeTacticLeaseIdentity {
                expansion_sha256: lease.expansion_sha256,
                lease_sha256: lease.lease_sha256,
            })
            .collect::<Vec<_>>();
        self.issue_identities(execution_plan_sha256, decision_index, identities)
    }

    fn issue_identities(
        &mut self,
        execution_plan_sha256: Digest,
        decision_index: u64,
        identities: Vec<NativeTacticLeaseIdentity>,
    ) -> Result<Digest, NativeTacticRouteRunError> {
        if execution_plan_sha256 == Digest::ZERO || identities.is_empty() {
            return Err(route_message("native tactic lease issue is invalid"));
        }
        let unique_expansions = identities
            .iter()
            .map(|lease| lease.expansion_sha256)
            .collect::<BTreeSet<_>>();
        let unique_leases = identities
            .iter()
            .map(|lease| lease.lease_sha256)
            .collect::<BTreeSet<_>>();
        if identities.iter().any(|lease| {
            lease.expansion_sha256 == Digest::ZERO || lease.lease_sha256 == Digest::ZERO
        }) || unique_expansions.len() != identities.len()
            || unique_leases.len() != identities.len()
        {
            return Err(route_message(
                "native tactic lease issue contains invalid or duplicate identities",
            ));
        }
        let event_index = self.records.len() as u64;
        let batch_sha256 = lease_batch_sha256(
            execution_plan_sha256,
            decision_index,
            event_index,
            &identities,
        );
        if self.batches.contains_key(&batch_sha256) {
            return Err(route_message("native tactic lease batch is duplicated"));
        }
        let record = NativeTacticLeaseRecord {
            event_index,
            event: NativeTacticLeaseEvent::Issued {
                execution_plan_sha256,
                decision_index,
                batch_sha256,
                leases: identities.clone(),
            },
        };
        self.append(record)?;
        self.batches.insert(
            batch_sha256,
            IssuedLeaseBatch {
                decision_index,
                leases: identities,
                outcome: None,
            },
        );
        Ok(batch_sha256)
    }

    pub(super) fn resolve(
        &mut self,
        batch_sha256: Digest,
        outcome: NativeTacticLeaseOutcome,
    ) -> Result<u64, NativeTacticRouteRunError> {
        let state = self
            .batches
            .get(&batch_sha256)
            .ok_or_else(|| route_message("native tactic lease resolution has no issue"))?;
        if state.outcome.is_some() {
            return Err(route_message(
                "native tactic lease batch has more than one resolution",
            ));
        }
        let lease_count = state.leases.len() as u64;
        let record = NativeTacticLeaseRecord {
            event_index: self.records.len() as u64,
            event: NativeTacticLeaseEvent::Resolved {
                batch_sha256,
                outcome,
            },
        };
        self.append(record)?;
        self.batches
            .get_mut(&batch_sha256)
            .expect("validated batch remains present")
            .outcome = Some(outcome);
        Ok(lease_count)
    }

    pub(super) fn accounting(
        &self,
    ) -> Result<NativeTacticLeaseAccounting, NativeTacticRouteRunError> {
        let bytes = fs::read(&self.path).map_err(route_error)?;
        accounting_for_batches(&self.batches, Digest(Sha256::digest(bytes).into()))
    }

    pub(super) fn accounting_from_bytes(
        bytes: &[u8],
    ) -> Result<NativeTacticLeaseAccounting, NativeTacticRouteRunError> {
        let decoded = decode_journal(bytes)?;
        if decoded.valid_bytes != bytes.len() {
            return Err(route_message(
                "bundled native tactic lease journal has a truncated tail",
            ));
        }
        accounting_for_batches(
            &validate_records(&decoded.records)?,
            Digest(Sha256::digest(bytes).into()),
        )
    }

    fn append(&mut self, record: NativeTacticLeaseRecord) -> Result<(), NativeTacticRouteRunError> {
        if record.event_index != self.records.len() as u64 {
            return Err(route_message(
                "native tactic lease journal event index is detached",
            ));
        }
        let encoded = encode_record(&record)?;
        let mut file = OpenOptions::new()
            .append(true)
            .open(&self.path)
            .map_err(route_error)?;
        file.write_all(&encoded)
            .and_then(|_| file.sync_data())
            .map_err(route_error)?;
        self.records.push(record);
        Ok(())
    }
}

fn accounting_for_batches(
    batches: &BTreeMap<Digest, IssuedLeaseBatch>,
    journal_sha256: Digest,
) -> Result<NativeTacticLeaseAccounting, NativeTacticRouteRunError> {
    let mut accounting = NativeTacticLeaseAccounting {
        journal_sha256,
        ..Default::default()
    };
    for batch in batches.values() {
        let count = batch.leases.len() as u64;
        accounting.proposal_dispatches = accounting.proposal_dispatches.saturating_add(count);
        match batch.outcome {
            Some(NativeTacticLeaseOutcome::Completed) => {
                accounting.completed_leases = accounting.completed_leases.saturating_add(count);
            }
            Some(NativeTacticLeaseOutcome::Retryable) => {
                accounting.retryable_leases = accounting.retryable_leases.saturating_add(count);
            }
            Some(NativeTacticLeaseOutcome::Cancelled) => {
                accounting.cancelled_leases = accounting.cancelled_leases.saturating_add(count);
            }
            Some(NativeTacticLeaseOutcome::Failed) => {
                accounting.failed_leases = accounting.failed_leases.saturating_add(count);
            }
            None => {
                accounting.unresolved_leases = accounting.unresolved_leases.saturating_add(count);
            }
        }
    }
    accounting.validate()?;
    Ok(accounting)
}

fn lease_batch_sha256(
    execution_plan_sha256: Digest,
    decision_index: u64,
    event_index: u64,
    leases: &[NativeTacticLeaseIdentity],
) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(LEASE_BATCH_SCHEMA);
    hasher.update(execution_plan_sha256.0);
    hasher.update(decision_index.to_le_bytes());
    hasher.update(event_index.to_le_bytes());
    hasher.update((leases.len() as u64).to_le_bytes());
    for lease in leases {
        hasher.update(lease.expansion_sha256.0);
        hasher.update(lease.lease_sha256.0);
    }
    Digest(hasher.finalize().into())
}

fn validate_records(
    records: &[NativeTacticLeaseRecord],
) -> Result<BTreeMap<Digest, IssuedLeaseBatch>, NativeTacticRouteRunError> {
    let mut batches = BTreeMap::<Digest, IssuedLeaseBatch>::new();
    for (index, record) in records.iter().enumerate() {
        if record.event_index != index as u64 {
            return Err(route_message(
                "native tactic lease journal event sequence is detached",
            ));
        }
        match &record.event {
            NativeTacticLeaseEvent::Issued {
                execution_plan_sha256,
                decision_index,
                batch_sha256,
                leases,
            } => {
                let unique_expansions = leases
                    .iter()
                    .map(|lease| lease.expansion_sha256)
                    .collect::<BTreeSet<_>>();
                let unique_leases = leases
                    .iter()
                    .map(|lease| lease.lease_sha256)
                    .collect::<BTreeSet<_>>();
                if *execution_plan_sha256 == Digest::ZERO
                    || leases.is_empty()
                    || leases.iter().any(|lease| {
                        lease.expansion_sha256 == Digest::ZERO || lease.lease_sha256 == Digest::ZERO
                    })
                    || unique_expansions.len() != leases.len()
                    || unique_leases.len() != leases.len()
                    || *batch_sha256
                        != lease_batch_sha256(
                            *execution_plan_sha256,
                            *decision_index,
                            record.event_index,
                            leases,
                        )
                    || batches
                        .insert(
                            *batch_sha256,
                            IssuedLeaseBatch {
                                decision_index: *decision_index,
                                leases: leases.clone(),
                                outcome: None,
                            },
                        )
                        .is_some()
                {
                    return Err(route_message("native tactic lease issue record is invalid"));
                }
            }
            NativeTacticLeaseEvent::Resolved {
                batch_sha256,
                outcome,
            } => {
                let state = batches.get_mut(batch_sha256).ok_or_else(|| {
                    route_message("native tactic lease resolution precedes its issue")
                })?;
                if state.outcome.replace(*outcome).is_some() {
                    return Err(route_message(
                        "native tactic lease batch has duplicate resolutions",
                    ));
                }
            }
        }
    }
    Ok(batches)
}

fn ensure_journal(path: &Path) -> Result<(), NativeTacticRouteRunError> {
    if path.exists() {
        return Ok(());
    }
    let parent = path
        .parent()
        .ok_or_else(|| route_message("native tactic lease journal has no parent"))?;
    let partial = parent.join(format!(
        ".{NATIVE_TACTIC_LEASE_JOURNAL_FILE}.{}.partial",
        std::process::id()
    ));
    if partial.exists() {
        fs::remove_file(&partial).map_err(route_error)?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial)
        .map_err(route_error)?;
    file.write_all(LEASE_JOURNAL_MAGIC)
        .and_then(|_| file.write_all(&LEASE_JOURNAL_VERSION.to_le_bytes()))
        .and_then(|_| file.write_all(&0_u16.to_le_bytes()))
        .and_then(|_| file.sync_all())
        .map_err(route_error)?;
    drop(file);
    match fs::rename(&partial, path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            fs::remove_file(&partial).map_err(route_error)
        }
        Err(error) => Err(route_error(error)),
    }
}

struct DecodedLeaseJournal {
    records: Vec<NativeTacticLeaseRecord>,
    valid_bytes: usize,
}

fn encode_record(record: &NativeTacticLeaseRecord) -> Result<Vec<u8>, NativeTacticRouteRunError> {
    let payload = serde_cbor::to_vec(record).map_err(route_error)?;
    if payload.len() > MAX_LEASE_RECORD_BYTES {
        return Err(route_message(
            "native tactic lease journal record exceeds its bound",
        ));
    }
    let mut bytes = Vec::with_capacity(LEASE_RECORD_HEADER_BYTES + payload.len());
    bytes.extend_from_slice(
        &u32::try_from(payload.len())
            .map_err(route_error)?
            .to_le_bytes(),
    );
    bytes.extend_from_slice(&<[u8; 32]>::from(Sha256::digest(&payload)));
    bytes.extend_from_slice(&payload);
    Ok(bytes)
}

fn decode_journal(bytes: &[u8]) -> Result<DecodedLeaseJournal, NativeTacticRouteRunError> {
    if bytes.len() < LEASE_JOURNAL_HEADER_BYTES
        || &bytes[..8] != LEASE_JOURNAL_MAGIC
        || u16::from_le_bytes(bytes[8..10].try_into().expect("fixed slice"))
            != LEASE_JOURNAL_VERSION
        || u16::from_le_bytes(bytes[10..12].try_into().expect("fixed slice")) != 0
    {
        return Err(route_message(
            "native tactic lease journal header is invalid",
        ));
    }
    let mut records = Vec::new();
    let mut cursor = LEASE_JOURNAL_HEADER_BYTES;
    while cursor < bytes.len() {
        let remaining = bytes.len() - cursor;
        if remaining < LEASE_RECORD_HEADER_BYTES {
            break;
        }
        let payload_len =
            u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().expect("fixed slice")) as usize;
        if payload_len > MAX_LEASE_RECORD_BYTES {
            return Err(route_message(
                "native tactic lease journal record length is invalid",
            ));
        }
        let record_len = LEASE_RECORD_HEADER_BYTES
            .checked_add(payload_len)
            .ok_or_else(|| route_message("native tactic lease record length overflows"))?;
        if remaining < record_len {
            break;
        }
        let expected: [u8; 32] = bytes[cursor + 4..cursor + 36]
            .try_into()
            .expect("fixed slice");
        let payload = &bytes[cursor + LEASE_RECORD_HEADER_BYTES..cursor + record_len];
        let actual: [u8; 32] = Sha256::digest(payload).into();
        if expected != actual {
            return Err(route_message(
                "native tactic lease journal record digest is invalid",
            ));
        }
        let mut deserializer = serde_cbor::Deserializer::from_slice(payload);
        let record =
            NativeTacticLeaseRecord::deserialize(&mut deserializer).map_err(route_error)?;
        deserializer.end().map_err(route_error)?;
        records.push(record);
        cursor += record_len;
    }
    Ok(DecodedLeaseJournal {
        records,
        valid_bytes: cursor,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "dusklight-lease-journal-{label}-{}",
            std::process::id()
        ))
    }

    fn leases() -> Vec<NativeTacticLeaseIdentity> {
        vec![
            NativeTacticLeaseIdentity {
                expansion_sha256: Digest([1; 32]),
                lease_sha256: Digest([2; 32]),
            },
            NativeTacticLeaseIdentity {
                expansion_sha256: Digest([3; 32]),
                lease_sha256: Digest([4; 32]),
            },
        ]
    }

    #[test]
    fn ledger_accounts_for_dispatches_separately_from_outcomes() {
        let root = temp_root("accounting");
        let _ = fs::remove_dir_all(&root);
        let mut ledger = NativeTacticLeaseLedger::open(&root).unwrap();
        let batch = ledger
            .issue_identities(Digest([9; 32]), 7, leases())
            .unwrap();
        assert_eq!(ledger.accounting().unwrap().unresolved_leases, 2);
        ledger
            .resolve(batch, NativeTacticLeaseOutcome::Completed)
            .unwrap();
        let accounting = ledger.accounting().unwrap();
        assert_eq!(accounting.proposal_dispatches, 2);
        assert_eq!(accounting.completed_leases, 2);
        assert_eq!(accounting.resolved_leases(), 2);
        assert_eq!(accounting.unresolved_leases, 0);
        assert!(
            ledger
                .resolve(batch, NativeTacticLeaseOutcome::Failed)
                .is_err()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn truncated_tail_recovers_issue_and_reconciles_it_as_retryable() {
        let root = temp_root("truncated");
        let _ = fs::remove_dir_all(&root);
        let mut ledger = NativeTacticLeaseLedger::open(&root).unwrap();
        ledger
            .issue_identities(Digest([9; 32]), 7, leases())
            .unwrap();
        let path = root.join(NATIVE_TACTIC_LEASE_JOURNAL_FILE);
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(&[1, 2, 3])
            .unwrap();
        drop(ledger);

        let mut resumed = NativeTacticLeaseLedger::open(&root).unwrap();
        assert_eq!(resumed.reconcile_unresolved(&BTreeMap::new()).unwrap(), 2);
        let accounting = resumed.accounting().unwrap();
        assert_eq!(accounting.proposal_dispatches, 2);
        assert_eq!(accounting.retryable_leases, 2);
        assert_eq!(accounting.unresolved_leases, 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn every_terminal_outcome_is_counted_and_bundled_tails_fail_closed() {
        let root = temp_root("terminal-outcomes");
        let _ = fs::remove_dir_all(&root);
        let mut ledger = NativeTacticLeaseLedger::open(&root).unwrap();
        for (decision, outcome) in [
            (1, NativeTacticLeaseOutcome::Retryable),
            (2, NativeTacticLeaseOutcome::Cancelled),
            (3, NativeTacticLeaseOutcome::Failed),
        ] {
            let batch = ledger
                .issue_identities(Digest([9; 32]), decision, leases())
                .unwrap();
            ledger.resolve(batch, outcome).unwrap();
        }
        let accounting = ledger.accounting().unwrap();
        assert_eq!(accounting.proposal_dispatches, 6);
        assert_eq!(accounting.retryable_leases, 2);
        assert_eq!(accounting.cancelled_leases, 2);
        assert_eq!(accounting.failed_leases, 2);
        assert_eq!(accounting.unresolved_leases, 0);

        let path = root.join(NATIVE_TACTIC_LEASE_JOURNAL_FILE);
        let bytes = fs::read(path).unwrap();
        assert_eq!(
            NativeTacticLeaseLedger::accounting_from_bytes(&bytes).unwrap(),
            accounting
        );
        let mut truncated = bytes;
        truncated.push(1);
        assert!(NativeTacticLeaseLedger::accounting_from_bytes(&truncated).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn resume_recognizes_an_issued_batch_already_present_in_the_decision_journal() {
        let root = temp_root("durable-completion");
        let _ = fs::remove_dir_all(&root);
        let identities = leases();
        let expansions = identities
            .iter()
            .map(|lease| lease.expansion_sha256)
            .collect::<Vec<_>>();
        let mut ledger = NativeTacticLeaseLedger::open(&root).unwrap();
        ledger
            .issue_identities(Digest([9; 32]), 7, identities)
            .unwrap();
        drop(ledger);

        let mut completed = BTreeMap::new();
        completed.insert(7, expansions);
        let mut resumed = NativeTacticLeaseLedger::open(&root).unwrap();
        assert_eq!(resumed.reconcile_unresolved(&completed).unwrap(), 2);
        let accounting = resumed.accounting().unwrap();
        assert_eq!(accounting.completed_leases, 2);
        assert_eq!(accounting.retryable_leases, 0);
        fs::remove_dir_all(root).unwrap();
    }
}
