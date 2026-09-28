//! Printer redirection for Linux and macOS: the remote's print jobs land in
//! the local print system.
//!
//! The channel announces one virtual printer to the server (MS-RDPEPC). When
//! the user prints to it, the server-side PostScript driver renders the job and
//! pushes the bytes down as a create / write... / close sequence on that
//! device. The job is spooled to a temporary file while it streams. On close,
//! a worker thread hands it to `lp` or saves it in the configured folder, so a
//! slow print system never stalls the channel.

use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ironrdp_pdu::PduResult;
use ironrdp_rdpdr::pdu::RdpdrPdu;
use ironrdp_rdpdr::pdu::efs::{
    DeviceCloseResponse, DeviceCreateResponse, DeviceIoRequest, DeviceIoResponse, DeviceWriteResponse, Information,
    NtStatus, PrinterIoRequest,
};
use ironrdp_svc::SvcMessage;
use tracing::{debug, info, warn};

/// Most print jobs the printer keeps open at once, counting abandoned jobs that
/// still wait for their close.
const MAX_OPEN_PRINT_JOBS: usize = 16;
/// Largest print job the printer accepts, in bytes.
const MAX_PRINT_JOB_BYTES: u64 = 128 * 1024 * 1024;
/// Closed jobs that may wait for the submission thread.
const SUBMISSION_QUEUE_CAPACITY: usize = 16;
/// How long `lp` may take to accept a job before it is killed.
const LP_TIMEOUT: Duration = Duration::from_secs(60);

/// Where a finished job goes.
#[derive(Debug, Clone)]
pub enum PrintTarget {
    /// The CUPS default destination (`lp` with no `-d`).
    DefaultPrinter,
    /// A named CUPS destination.
    Printer(String),
    /// A directory: each job becomes `RDP print <timestamp>.ps` inside it.
    Folder(PathBuf),
}

/// One job in flight: the server opened the printer and is writing.
#[derive(Debug)]
struct Job {
    spool: PathBuf,
    file: File,
    bytes: u64,
}

/// A closed job waiting for the submission thread.
#[derive(Debug)]
struct FinishedJob {
    spool: PathBuf,
    bytes: u64,
}

/// The thread that submits closed jobs, one at a time.
#[derive(Debug)]
struct Submitter {
    jobs: SyncSender<FinishedJob>,
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "tests join the thread; a session leaves it to finish the queue")
    )]
    thread: JoinHandle<()>,
}

/// Print-job state for one virtual printer.
#[derive(Debug)]
pub struct PrinterSpooler {
    target: PrintTarget,
    /// Private, atomically created 0700 directory, allocated on the first print job.
    spool_dir: Option<PathBuf>,
    next_file_id: u32,
    jobs: HashMap<u32, Job>,
    /// Handles whose job was abandoned; their close must not submit anything.
    abandoned: HashSet<u32>,
    /// Started on the first closed job.
    submitter: Option<Submitter>,
}

impl PrinterSpooler {
    pub fn new(target: PrintTarget) -> Self {
        Self {
            target,
            spool_dir: None,
            next_file_id: 1,
            jobs: HashMap::new(),
            abandoned: HashSet::new(),
            submitter: None,
        }
    }

    pub fn handle(&mut self, req: PrinterIoRequest) -> PduResult<Vec<SvcMessage>> {
        let response = match req {
            PrinterIoRequest::Create(create) => self.create(create.device_io_request),
            PrinterIoRequest::Write(write) => self.write(write.device_io_request, &write.write_data),
            PrinterIoRequest::Close(close) => self.close(close.device_io_request),
        };
        Ok(vec![SvcMessage::from(response)])
    }

    /// Abandons the job behind an oversized write so a later close discards it.
    pub fn reject_write(&mut self, req: DeviceIoRequest) -> PduResult<Vec<SvcMessage>> {
        if self.jobs.contains_key(&req.file_id) {
            warn!(
                file_id = req.file_id,
                "Print job abandoned: the server sent an oversized write"
            );
            self.abandon(req.file_id);
        }
        Ok(vec![SvcMessage::from(write_response(req, 0, NtStatus::UNSUCCESSFUL))])
    }

    /// Discards the open and abandoned jobs of the current RDPDR initialization sequence. Jobs
    /// that were already closed are still submitted.
    pub fn reset(&mut self) {
        for (_, job) in self.jobs.drain() {
            drop(job.file);
            let _ = std::fs::remove_file(job.spool);
        }
        self.abandoned.clear();
    }

    fn create(&mut self, request: DeviceIoRequest) -> RdpdrPdu {
        if self.jobs.len() + self.abandoned.len() >= MAX_OPEN_PRINT_JOBS {
            warn!(limit = MAX_OPEN_PRINT_JOBS, "Print job refused: too many jobs are open");
            return create_response(request, 0, NtStatus::UNSUCCESSFUL);
        }
        let file_id = self.next_file_id;
        self.next_file_id = self.next_file_id.wrapping_add(1).max(1);
        match self.open_spool(file_id) {
            Ok(job) => {
                debug!(file_id, spool = ?job.spool, "Print job opened");
                self.jobs.insert(file_id, job);
                create_response(request, file_id, NtStatus::SUCCESS)
            }
            Err(error) => {
                warn!(%error, "Could not open a spool file for a print job");
                create_response(request, 0, NtStatus::UNSUCCESSFUL)
            }
        }
    }

    fn write(&mut self, request: DeviceIoRequest, data: &[u8]) -> RdpdrPdu {
        let file_id = request.file_id;
        let Some(job) = self.jobs.get_mut(&file_id) else {
            return write_response(request, 0, NtStatus::UNSUCCESSFUL);
        };
        let total = job.bytes.saturating_add(u64::try_from(data.len()).unwrap_or(u64::MAX));
        if total > MAX_PRINT_JOB_BYTES {
            warn!(
                file_id,
                limit = MAX_PRINT_JOB_BYTES,
                "Print job abandoned: it exceeds the size limit"
            );
            self.abandon(file_id);
            return write_response(request, 0, NtStatus::UNSUCCESSFUL);
        }
        match job.file.write_all(data) {
            Ok(()) => {
                job.bytes = total;
                let length = u32::try_from(data.len()).unwrap_or(u32::MAX);
                write_response(request, length, NtStatus::SUCCESS)
            }
            Err(error) => {
                warn!(%error, file_id, "Print job abandoned: could not spool its data");
                self.abandon(file_id);
                write_response(request, 0, NtStatus::UNSUCCESSFUL)
            }
        }
    }

    fn close(&mut self, request: DeviceIoRequest) -> RdpdrPdu {
        let file_id = request.file_id;
        if !self.abandoned.remove(&file_id)
            && let Some(Job { spool, file, bytes }) = self.jobs.remove(&file_id)
        {
            drop(file);
            if bytes == 0 {
                debug!(?spool, "Empty print job discarded");
                let _ = std::fs::remove_file(spool);
            } else {
                self.queue_submission(FinishedJob { spool, bytes });
            }
        }
        close_response(request, NtStatus::SUCCESS)
    }

    /// Drops an open job's spool file so its close submits nothing, not even a partial document.
    fn abandon(&mut self, file_id: u32) {
        if let Some(job) = self.jobs.remove(&file_id) {
            drop(job.file);
            let _ = std::fs::remove_file(job.spool);
            self.abandoned.insert(file_id);
        }
    }

    fn queue_submission(&mut self, job: FinishedJob) {
        if self.submitter.is_none() {
            let Some(spool_dir) = self.spool_dir.clone() else {
                return;
            };
            match Submitter::spawn(self.target.clone(), spool_dir) {
                Ok(submitter) => self.submitter = Some(submitter),
                Err(error) => {
                    warn!(%error, "Could not start the print submission thread; the print job is discarded");
                    let _ = std::fs::remove_file(job.spool);
                    return;
                }
            }
        }
        let Some(submitter) = &self.submitter else {
            return;
        };
        if let Err(TrySendError::Full(job) | TrySendError::Disconnected(job)) = submitter.jobs.try_send(job) {
            warn!(spool = ?job.spool, "Could not queue the print job for submission; it is discarded");
            let _ = std::fs::remove_file(job.spool);
        }
    }

    fn open_spool(&mut self, file_id: u32) -> std::io::Result<Job> {
        if self.spool_dir.is_none() {
            // mkdtemp creates the directory exclusively with mode 0700; no shared
            // pathname can be substituted between creation and opening a job.
            let template = std::env::temp_dir().join("ironrdp-print-XXXXXX");
            self.spool_dir = Some(nix::unistd::mkdtemp(&template)?);
        }
        let dir = self
            .spool_dir
            .as_ref()
            .ok_or_else(|| std::io::Error::other("missing spool directory"))?;
        let spool = dir.join(format!("{file_id}.ps"));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&spool)?;
        Ok(Job { spool, file, bytes: 0 })
    }
}

impl Drop for PrinterSpooler {
    fn drop(&mut self) {
        // A disconnected session abandons its open jobs. Jobs it already closed are still
        // submitted, and the submission thread removes the spool directory once it is done.
        self.reset();
        if self.submitter.take().is_none()
            && let Some(dir) = self.spool_dir.take()
        {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

impl Submitter {
    fn spawn(target: PrintTarget, spool_dir: PathBuf) -> std::io::Result<Self> {
        let (jobs, queue) = sync_channel(SUBMISSION_QUEUE_CAPACITY);
        let thread = std::thread::Builder::new()
            .name("ironrdp-print-submit".to_owned())
            .spawn(move || submit_jobs(&target, &spool_dir, queue))?;
        Ok(Self { jobs, thread })
    }
}

/// Submits queued jobs until the spooler is gone, then removes the spool directory.
fn submit_jobs(target: &PrintTarget, spool_dir: &Path, queue: Receiver<FinishedJob>) {
    for job in queue {
        match target {
            PrintTarget::DefaultPrinter => print(&job, None),
            PrintTarget::Printer(name) => print(&job, Some(name)),
            PrintTarget::Folder(dir) => save(&job.spool, dir),
        }
        let _ = std::fs::remove_file(&job.spool);
    }
    let _ = std::fs::remove_dir_all(spool_dir);
}

/// Hands the job to `lp`, killing it if it has not accepted the job within [`LP_TIMEOUT`].
fn print(job: &FinishedJob, destination: Option<&str>) {
    let mut command = Command::new("lp");
    if let Some(name) = destination {
        command.arg("-d").arg(name);
    }
    command
        .arg("-t")
        .arg("RDP print job")
        .arg(&job.spool)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            warn!(%error, "lp is not available; the print job is discarded");
            return;
        }
    };
    let deadline = Instant::now() + LP_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                warn!(timeout = ?LP_TIMEOUT, "lp did not accept the print job in time; the job is discarded");
                return;
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                warn!(%error, "Could not wait for lp; the print job is discarded");
                return;
            }
        }
    };
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_string(&mut stdout);
    }
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    if status.success() {
        info!(bytes = job.bytes, "Print job handed to lp: {}", stdout.trim());
    } else {
        warn!("lp refused the print job ({}); the job is discarded", stderr.trim());
    }
}

/// Copies the spooled job into `dir` under a readable name.
fn save(spool: &Path, dir: &Path) {
    if let Err(error) = std::fs::create_dir_all(dir) {
        warn!(%error, ?dir, "Could not create the print output folder");
        return;
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // The destination may also be shared. Exclusive creation avoids overwriting
    // existing documents or following a symlink, including across filesystems.
    let mut n = 0u64;
    let (destination, mut output) = loop {
        let name = if n == 0 {
            format!("RDP print {stamp}.ps")
        } else {
            format!("RDP print {stamp} ({n}).ps")
        };
        let destination = dir.join(name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&destination)
        {
            Ok(file) => break (destination, file),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => n = n.saturating_add(1),
            Err(error) => {
                warn!(%error, ?destination, "Could not create the print output file");
                return;
            }
        }
    };
    let saved = File::open(spool)
        .and_then(|mut input| std::io::copy(&mut input, &mut output))
        .and_then(|_| output.flush());
    match saved {
        Ok(()) => info!(?destination, "Print job saved as a PostScript file"),
        Err(error) => {
            drop(output);
            let _ = std::fs::remove_file(&destination);
            warn!(%error, ?destination, "Could not save the print job");
        }
    }
}

/// Answers a printer request with `NOT_SUPPORTED`, in the response its major function expects.
pub(crate) fn unsupported_response(request: PrinterIoRequest) -> RdpdrPdu {
    match request {
        PrinterIoRequest::Create(create) => create_response(create.device_io_request, 0, NtStatus::NOT_SUPPORTED),
        PrinterIoRequest::Write(write) => write_response(write.device_io_request, 0, NtStatus::NOT_SUPPORTED),
        PrinterIoRequest::Close(close) => close_response(close.device_io_request, NtStatus::NOT_SUPPORTED),
    }
}

/// A failed create carries no file handle and no `FILE_OPENED`.
fn create_response(request: DeviceIoRequest, file_id: u32, status: NtStatus) -> RdpdrPdu {
    let opened = status == NtStatus::SUCCESS;
    RdpdrPdu::DeviceCreateResponse(DeviceCreateResponse {
        device_io_reply: DeviceIoResponse::new(request, status),
        file_id: if opened { file_id } else { 0 },
        information: if opened {
            Information::FILE_OPENED
        } else {
            Information::empty()
        },
    })
}

/// `length` is the number of bytes written, so a failed write reports 0 (MS-RDPEPC 3.2.5.1.12).
pub(crate) fn write_response(request: DeviceIoRequest, length: u32, status: NtStatus) -> RdpdrPdu {
    RdpdrPdu::DeviceWriteResponse(DeviceWriteResponse {
        device_io_reply: DeviceIoResponse::new(request, status),
        length,
    })
}

fn close_response(request: DeviceIoRequest, status: NtStatus) -> RdpdrPdu {
    RdpdrPdu::DeviceCloseResponse(DeviceCloseResponse {
        device_io_response: DeviceIoResponse::new(request, status),
    })
}

#[cfg(test)]
mod tests {
    use ironrdp_rdpdr::pdu::efs::{DeviceCloseRequest, DeviceWriteRequest, MajorFunction, MinorFunction};
    use std::os::unix::fs::{PermissionsExt as _, symlink};

    use super::*;

    fn io(file_id: u32, major: MajorFunction) -> DeviceIoRequest {
        DeviceIoRequest {
            device_id: 7,
            file_id,
            completion_id: 1,
            major_function: major,
            minor_function: MinorFunction::IRP_MN_QUERY_DIRECTORY,
        }
    }

    fn open_job(spooler: &mut PrinterSpooler) -> u32 {
        let RdpdrPdu::DeviceCreateResponse(response) = spooler.create(io(0, MajorFunction::Create)) else {
            panic!("expected a create response");
        };
        assert_eq!(response.information, Information::FILE_OPENED);
        response.file_id
    }

    fn close_job(spooler: &mut PrinterSpooler, file_id: u32) {
        spooler
            .handle(PrinterIoRequest::Close(DeviceCloseRequest::decode(io(
                file_id,
                MajorFunction::Close,
            ))))
            .expect("close");
    }

    /// Stops the submission thread once it has handled every queued job.
    fn wait_for_submissions(spooler: &mut PrinterSpooler) {
        if let Some(Submitter { jobs, thread }) = spooler.submitter.take() {
            drop(jobs);
            thread.join().expect("submission thread");
        }
    }

    #[test]
    fn a_job_streams_into_the_folder_target() {
        let dir = std::env::temp_dir().join(format!("ironrdp-print-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut spooler = PrinterSpooler::new(PrintTarget::Folder(dir.clone()));

        let file_id = open_job(&mut spooler);
        for chunk in [&b"%!PS-Adobe-3.0\n"[..], b"showpage\n"] {
            spooler
                .handle(PrinterIoRequest::Write(DeviceWriteRequest {
                    device_io_request: io(file_id, MajorFunction::Write),
                    offset: 0,
                    write_data: chunk.to_vec(),
                }))
                .expect("write");
        }
        close_job(&mut spooler, file_id);
        wait_for_submissions(&mut spooler);

        let saved: Vec<_> = std::fs::read_dir(&dir).expect("dir").flatten().collect();
        assert_eq!(saved.len(), 1, "one job file");
        assert_eq!(
            saved[0].metadata().expect("saved job").permissions().mode() & 0o777,
            0o600
        );
        let content = std::fs::read_to_string(saved[0].path()).expect("content");
        assert_eq!(content, "%!PS-Adobe-3.0\nshowpage\n");
        assert!(spooler.jobs.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_rejected_write_poisons_the_job_so_close_discards_it() {
        let dir = std::env::temp_dir().join(format!("ironrdp-print-test-poison-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut spooler = PrinterSpooler::new(PrintTarget::Folder(dir.clone()));
        let file_id = open_job(&mut spooler);
        spooler.reject_write(io(file_id, MajorFunction::Write)).expect("reject");
        close_job(&mut spooler, file_id);
        wait_for_submissions(&mut spooler);
        assert!(
            !dir.exists() || std::fs::read_dir(&dir).expect("dir").next().is_none(),
            "nothing saved"
        );
    }

    #[test]
    fn a_job_over_the_size_limit_is_abandoned() {
        let mut spooler = PrinterSpooler::new(PrintTarget::DefaultPrinter);
        let file_id = open_job(&mut spooler);
        let spool = spooler.jobs[&file_id].spool.clone();
        spooler.jobs.get_mut(&file_id).expect("open job").bytes = MAX_PRINT_JOB_BYTES;

        let RdpdrPdu::DeviceWriteResponse(response) = spooler.write(io(file_id, MajorFunction::Write), b"x") else {
            panic!("expected a write response");
        };
        assert_eq!(response.length, 0, "a failed write reports no bytes written");
        assert!(!spool.exists(), "the abandoned job's spool file is removed");
        assert!(spooler.abandoned.contains(&file_id));
    }

    #[test]
    fn failed_requests_carry_no_handle_or_length() {
        let mut spooler = PrinterSpooler::new(PrintTarget::DefaultPrinter);
        for _ in 0..MAX_OPEN_PRINT_JOBS {
            open_job(&mut spooler);
        }
        let RdpdrPdu::DeviceCreateResponse(response) = spooler.create(io(0, MajorFunction::Create)) else {
            panic!("expected a create response");
        };
        assert_eq!(response.file_id, 0);
        assert_eq!(response.information, Information::empty());

        let RdpdrPdu::DeviceWriteResponse(response) = spooler.write(io(u32::MAX, MajorFunction::Write), b"data") else {
            panic!("expected a write response");
        };
        assert_eq!(response.length, 0);
    }

    #[test]
    fn unsupported_requests_are_answered_with_their_own_response_type() {
        let write = unsupported_response(PrinterIoRequest::Write(DeviceWriteRequest {
            device_io_request: io(1, MajorFunction::Write),
            offset: 0,
            write_data: b"data".to_vec(),
        }));
        let RdpdrPdu::DeviceWriteResponse(response) = write else {
            panic!("expected a write response");
        };
        assert_eq!(response.length, 0);
    }

    #[test]
    fn reset_discards_open_jobs() {
        let mut spooler = PrinterSpooler::new(PrintTarget::DefaultPrinter);
        let open = open_job(&mut spooler);
        let spool = spooler.jobs[&open].spool.clone();
        let abandoned = open_job(&mut spooler);
        spooler
            .reject_write(io(abandoned, MajorFunction::Write))
            .expect("reject");

        spooler.reset();
        assert!(spooler.jobs.is_empty());
        assert!(spooler.abandoned.is_empty());
        assert!(!spool.exists(), "the open job's spool file is removed");
    }

    #[test]
    fn spool_files_are_private_and_removed_when_the_session_ends() {
        let mut spooler = PrinterSpooler::new(PrintTarget::DefaultPrinter);
        let mut job = spooler.open_spool(1).expect("private job");
        let dir = job.spool.parent().expect("spool directory").to_path_buf();
        assert_eq!(
            std::fs::metadata(&dir).expect("directory").permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(job.file.metadata().expect("job").permissions().mode() & 0o777, 0o600);
        job.file.write_all(b"private document").expect("write");
        spooler.jobs.insert(1, job);
        drop(spooler);
        assert!(!dir.exists(), "disconnection removes unfinished documents");
    }

    #[test]
    fn an_existing_spool_symlink_is_never_followed() {
        let mut spooler = PrinterSpooler::new(PrintTarget::DefaultPrinter);
        let job = spooler.open_spool(1).expect("allocate directory");
        let dir = job.spool.parent().expect("directory");
        let target = dir.join("existing-document");
        std::fs::write(&target, b"keep this").expect("target");
        symlink(&target, dir.join("2.ps")).expect("symlink");
        assert_eq!(
            spooler.open_spool(2).expect_err("collision must fail").kind(),
            std::io::ErrorKind::AlreadyExists
        );
        assert_eq!(std::fs::read(&target).expect("unchanged target"), b"keep this");
    }
}
