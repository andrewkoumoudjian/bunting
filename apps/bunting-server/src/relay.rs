use crate::config::{RelayConfig, TlsConfig};
use simfix_wire::{Decoder, FixMessage, WireLimits};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[derive(Clone, Copy)]
enum Direction {
    ParticipantToWorker = 1,
    WorkerToParticipant = 2,
}

#[derive(Clone)]
struct RelayJournal {
    path: PathBuf,
    max_bytes: usize,
    lock: Arc<Mutex<()>>,
}

impl RelayJournal {
    fn append(&self, direction: Direction, bytes: &[u8]) -> Result<(), String> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "relay journal lock poisoned")?;
        let current = fs::metadata(&self.path)
            .map(|metadata| usize::try_from(metadata.len()).unwrap_or(usize::MAX))
            .unwrap_or(0);
        let record_size = bytes.len().saturating_add(9);
        if current.saturating_add(record_size) > self.max_bytes {
            OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&self.path)
                .and_then(|file| file.sync_all())
                .map_err(|error| format!("cannot rotate relay journal: {error}"))?;
        }
        if let Some(parent) = self
            .path
            .parent()
            .filter(|value| !value.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)
                .map_err(|error| format!("cannot create relay journal directory: {error}"))?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|error| format!("cannot open relay journal: {error}"))?;
        file.write_all(&[direction as u8])
            .and_then(|()| {
                file.write_all(&u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_be_bytes())
            })
            .and_then(|()| file.write_all(bytes))
            .and_then(|()| file.sync_data())
            .map_err(|error| format!("cannot persist relay journal: {error}"))
    }
}

pub fn run(config: &RelayConfig) -> Result<(), String> {
    let participant_listener = TcpListener::bind(&config.participant_bind)
        .map_err(|error| format!("cannot bind participant relay listener: {error}"))?;
    let worker_listener = TcpListener::bind(&config.worker_bind)
        .map_err(|error| format!("cannot bind Worker relay listener: {error}"))?;
    let journal = RelayJournal {
        path: PathBuf::from(&config.journal_path),
        max_bytes: config.max_journal_bytes,
        lock: Arc::new(Mutex::new(())),
    };
    loop {
        let (mut participant, _) = participant_listener
            .accept()
            .map_err(|error| format!("participant accept failed: {error}"))?;
        verify_terminated_peer(&participant, &config.tls)?;
        participant
            .set_read_timeout(Some(Duration::from_secs(30)))
            .map_err(|error| format!("participant timeout setup failed: {error}"))?;
        let participant_prefix = authenticate(
            &mut participant,
            config.max_message_bytes,
            &config.participant_sender_comp_id,
            &config.target_comp_id,
            Some((&config.participant_username, &config.participant_password)),
        )?;

        let (mut worker, _) = worker_listener
            .accept()
            .map_err(|error| format!("Worker accept failed: {error}"))?;
        worker
            .set_read_timeout(Some(Duration::from_secs(30)))
            .map_err(|error| format!("Worker timeout setup failed: {error}"))?;
        let worker_prefix = authenticate(
            &mut worker,
            config.max_message_bytes,
            &config.worker_sender_comp_id,
            &config.target_comp_id,
            None,
        )?;
        worker
            .write_all(&participant_prefix)
            .map_err(|error| format!("cannot relay participant Logon: {error}"))?;
        journal.append(Direction::ParticipantToWorker, &participant_prefix)?;
        participant
            .write_all(&worker_prefix)
            .map_err(|error| format!("cannot relay Worker Logon: {error}"))?;
        journal.append(Direction::WorkerToParticipant, &worker_prefix)?;
        relay_pair(
            participant,
            worker,
            config.max_message_bytes.min(config.max_pending_bytes),
            journal.clone(),
        )?;
    }
}

fn verify_terminated_peer(stream: &TcpStream, tls: &TlsConfig) -> Result<(), String> {
    let TlsConfig::Terminated { trusted_proxy, .. } = tls else {
        return Ok(());
    };
    let expected = trusted_proxy
        .parse::<std::net::IpAddr>()
        .map_err(|_| "invalid relay trusted proxy".to_owned())?;
    let actual = stream
        .peer_addr()
        .map_err(|error| format!("cannot inspect participant relay peer: {error}"))?
        .ip();
    if actual != expected {
        return Err(format!(
            "participant relay peer {actual} is not configured TLS terminator {expected}"
        ));
    }
    Ok(())
}

fn authenticate(
    stream: &mut TcpStream,
    max_message_bytes: usize,
    sender: &str,
    target: &str,
    credentials: Option<(&str, &str)>,
) -> Result<Vec<u8>, String> {
    let mut decoder = Decoder::try_new(WireLimits {
        max_message_bytes,
        ..WireLimits::default()
    })
    .map_err(|error| format!("cannot load FIX dictionaries: {error:?}"))?;
    let mut collected = Vec::new();
    let mut chunk = vec![0; max_message_bytes.min(8_192)];
    loop {
        let count = stream
            .read(&mut chunk)
            .map_err(|error| format!("cannot read relay Logon: {error}"))?;
        if count == 0 {
            return Err("peer disconnected before FIX Logon".to_owned());
        }
        collected.extend_from_slice(&chunk[..count]);
        if collected.len() > max_message_bytes {
            return Err("relay Logon exceeds max_message_bytes".to_owned());
        }
        let messages = decoder
            .push(&chunk[..count])
            .map_err(|error| format!("invalid relay FIX framing: {error:?}"))?;
        if let Some(logon) = messages.first() {
            validate_logon(logon, sender, target, credentials)?;
            return Ok(collected);
        }
    }
}

fn validate_logon(
    message: &FixMessage,
    sender: &str,
    target: &str,
    credentials: Option<(&str, &str)>,
) -> Result<(), String> {
    if message.msg_type != "A"
        || message.value(49) != Some(sender)
        || message.value(56) != Some(target)
        || message.value(1137) != Some(simfix_wire::FIX_50_SP2_APPL_VER_ID)
        || message.value(10000) != Some(bunting_api_contract::FIX_COMPETITION_PROFILE_VERSION)
    {
        return Err("relay peer FIX identity does not match configured binding".to_owned());
    }
    if let Some((username, password)) = credentials
        && (!constant_time_eq(message.value(553).unwrap_or_default(), username)
            || !constant_time_eq(message.value(554).unwrap_or_default(), password))
    {
        return Err("relay participant credentials rejected".to_owned());
    }
    Ok(())
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    let mut difference = left.len() ^ right.len();
    let limit = left.len().max(right.len()).min(256);
    for index in 0..limit {
        difference |= usize::from(
            left.as_bytes().get(index).copied().unwrap_or_default()
                ^ right.as_bytes().get(index).copied().unwrap_or_default(),
        );
    }
    difference == 0 && left.len() <= 256 && right.len() <= 256
}

fn relay_pair(
    participant: TcpStream,
    worker: TcpStream,
    buffer_bytes: usize,
    journal: RelayJournal,
) -> Result<(), String> {
    let participant_read = participant
        .try_clone()
        .map_err(|error| format!("cannot clone participant socket: {error}"))?;
    let worker_read = worker
        .try_clone()
        .map_err(|error| format!("cannot clone Worker socket: {error}"))?;
    let first_journal = journal.clone();
    let first = thread::spawn(move || {
        copy_bounded(
            participant_read,
            worker,
            buffer_bytes,
            Direction::ParticipantToWorker,
            &first_journal,
        )
    });
    let second = thread::spawn(move || {
        copy_bounded(
            worker_read,
            participant,
            buffer_bytes,
            Direction::WorkerToParticipant,
            &journal,
        )
    });
    first
        .join()
        .map_err(|_| "participant relay thread panicked".to_owned())??;
    second
        .join()
        .map_err(|_| "Worker relay thread panicked".to_owned())??;
    Ok(())
}

fn copy_bounded(
    mut input: TcpStream,
    mut output: TcpStream,
    buffer_bytes: usize,
    direction: Direction,
    journal: &RelayJournal,
) -> Result<(), String> {
    let mut buffer = vec![0; buffer_bytes.min(65_536)];
    loop {
        let count = input
            .read(&mut buffer)
            .map_err(|error| format!("relay read failed: {error}"))?;
        if count == 0 {
            return Ok(());
        }
        journal.append(direction, &buffer[..count])?;
        output
            .write_all(&buffer[..count])
            .map_err(|error| format!("relay write failed: {error}"))?;
    }
}

#[allow(dead_code)]
fn _journal_is_native(path: &Path) -> bool {
    path.is_absolute() || path.parent().is_some()
}
