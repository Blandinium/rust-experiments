
use actrs_macros::actor;
use actrs::{ActorRef, MsgCtx, Stage};
use proplists::List;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::time::{Duration, Instant};

// --- Configuration ---
const WORKER_COUNT: usize = 4;
const WORK_ITERATIONS: usize = 1000;
const DEFAULT_INPUT_PATH: &str = "input.log";
const DEFAULT_LEDGER_PATH: &str = "ledger.txt";

// --- Data Models ---

#[derive(Debug, Clone)]
pub struct LogRecord {
    pub timestamp: String,
    pub level: String,
    pub service: String,
    pub user: String,
    pub latency_ms: u64,
    pub status: u16,
    pub path: String,
}

#[derive(Debug, Clone)]
pub struct ProcessedRecord {
    pub line_id: u64,
    pub worker_id: usize,
    pub service: String,
    pub status: u16,
    pub latency_ms: u64,
    pub path: String,
    pub level: String,
    pub processing_time_us: u64,
    pub checksum: u64,
}

#[derive(Debug, Clone)]
pub struct InvalidRecord {
    pub line_id: u64,
    pub worker_id: usize,
    pub error: String,
    pub processing_time_us: u64,
}

// --- Messages ---

#[derive(Debug, Clone)]
pub struct StartReading;

#[derive(Debug, Clone)]
pub struct DispatchLine {
    pub line_id: u64,
    pub line: String,
}

#[derive(Debug, Clone)]
pub struct ReaderFinished {
    pub total_lines: u64,
}

#[derive(Debug, Clone)]
pub struct ProcessLine {
    pub line_id: u64,
    pub line: String,
}

#[derive(Debug, Clone)]
pub enum ProcessedLineResult {
    Valid(ProcessedRecord),
    Invalid(InvalidRecord),
}

#[derive(Debug, Clone)]
pub struct LedgerEntry {
    pub line_id: u64,
    pub worker_id: usize,
    pub processing_time_us: u64,
    pub success: bool,
}

#[derive(Debug, Clone)]
pub struct WorkCompletedAck {
    pub line_id: u64,
    pub worker_id: usize,
}

#[derive(Debug, Clone)]
pub struct Shutdown;

#[derive(Debug, Clone)]
pub struct RegisterWorker {
    pub worker_id: usize,
    pub worker_ref: ActorRef,
}

#[derive(Debug, Clone)]
pub struct FinalizeCollection;

#[derive(Debug, Clone)]
pub struct FinalizeLedger;

// --- Actors ---

// 1. ReaderActor
pub struct ReaderActor {
    _handle: Option<actrs::ActorHandle>,
}

#[derive(Debug, Clone)]
pub struct ReaderState {
    input_path: String,
    dispatcher: ActorRef,
    total_lines: u64,
    started: bool,
}

impl Default for ReaderState {
    fn default() -> Self {
        Self {
            input_path: String::new(),
            dispatcher: ActorRef(0),
            total_lines: 0,
            started: false,
        }
    }
}

#[actor(state = ReaderState, handle = _handle)]
impl ReaderActor {
    #[initialize]
    fn initialize(&self, state: ReaderState) -> ReaderState {
        state
    }

    #[message_consumer(StartReading)]
    fn handle_start(&self, _msg: StartReading, _ctx: MsgCtx, state: ReaderState) -> ::actrs::ActorResult<ReaderState> {
        if state.started {
            return ::actrs::ActorResult::Ok(state);
        }
        let state = ReaderState { started: true, ..state };

        let file = match File::open(&state.input_path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("ReaderActor: Failed to open file {}: {}", state.input_path, e);
                return ::actrs::ActorResult::Ok(state);
            }
        };

        let mut total_lines = state.total_lines;
        let reader = BufReader::new(file);
        for (index, line_result) in reader.lines().enumerate() {
            let line_id = (index + 1) as u64;
            match line_result {
                Ok(line) => {
                    total_lines += 1;
                    let _ = self.send::<_, DispatcherActor>(state.dispatcher, DispatchLine { line_id, line });
                }
                Err(e) => {
                    eprintln!("ReaderActor: Error reading line {}: {}", line_id, e);
                }
            }
        }

        let _ = self.send::<_, DispatcherActor>(state.dispatcher, ReaderFinished { total_lines });
        ::actrs::ActorResult::Stop
    }
}

// 2. DispatcherActor
pub struct DispatcherActor {
    _handle: Option<actrs::ActorHandle>,
}

#[derive(Debug, Clone)]
pub struct DispatcherState {
    workers: List<ActorRef>,
    collector: ActorRef,
    ledger: ActorRef,
    next_worker_idx: usize,
    total_received: u64,
    total_dispatched: u64,
    total_completed: u64,
    reader_done: bool,
    expected_total: u64,
}

impl Default for DispatcherState {
    fn default() -> Self {
        Self {
            workers: List::empty(),
            collector: ActorRef(0),
            ledger: ActorRef(0),
            next_worker_idx: 0,
            total_received: 0,
            total_dispatched: 0,
            total_completed: 0,
            reader_done: false,
            expected_total: 0,
        }
    }
}

#[actor(state = DispatcherState, handle = _handle)]
impl DispatcherActor {
    #[initialize]
    fn initialize(&self, state: DispatcherState) -> DispatcherState {
        state
    }

    #[message_consumer(RegisterWorker)]
    fn handle_register(&self, msg: RegisterWorker, _ctx: MsgCtx, state: DispatcherState) -> DispatcherState {
        DispatcherState {
            workers: state.workers.prepend(msg.worker_ref),
            ..state
        }
    }

    #[message_consumer(DispatchLine)]
    fn handle_dispatch(&self, msg: DispatchLine, _ctx: MsgCtx, state: DispatcherState) -> DispatcherState {
        let total_received = state.total_received + 1;
        
        let worker_count = state.workers.iter().count();
        if worker_count == 0 {
            return DispatcherState { total_received, ..state };
        }

        let worker_ref = state.workers.iter().nth(state.next_worker_idx).unwrap();
        let _ = self.send::<_, WorkerActor>(*worker_ref, ProcessLine { line_id: msg.line_id, line: msg.line });
        
        let total_dispatched = state.total_dispatched + 1;
        let next_worker_idx = (state.next_worker_idx + 1) % worker_count;
        
        DispatcherState {
            total_received,
            total_dispatched,
            next_worker_idx,
            ..state
        }
    }

    #[message_consumer(ReaderFinished)]
    fn handle_reader_finished(&self, msg: ReaderFinished, _ctx: MsgCtx, state: DispatcherState) -> ::actrs::ActorResult<DispatcherState> {
        let state = DispatcherState {
            reader_done: true,
            expected_total: msg.total_lines,
            ..state
        };
        if state.reader_done && state.total_completed == state.expected_total && state.total_dispatched == state.expected_total {
            for worker_ref in state.workers.iter() {
                let _ = self.send::<_, WorkerActor>(*worker_ref, Shutdown);
            }
            let _ = self.send::<_, CollectorActor>(state.collector, FinalizeCollection);
            let _ = self.send::<_, LedgerActor>(state.ledger, FinalizeLedger);
            ::actrs::ActorResult::Stop
        } else {
            ::actrs::ActorResult::Ok(state)
        }
    }

    #[message_consumer(WorkCompletedAck)]
    fn handle_ack(&self, _msg: WorkCompletedAck, _ctx: MsgCtx, state: DispatcherState) -> ::actrs::ActorResult<DispatcherState> {
        let state = DispatcherState {
            total_completed: state.total_completed + 1,
            ..state
        };
        if state.reader_done && state.total_completed == state.expected_total && state.total_dispatched == state.expected_total {
            for worker_ref in state.workers.iter() {
                let _ = self.send::<_, WorkerActor>(*worker_ref, Shutdown);
            }
            let _ = self.send::<_, CollectorActor>(state.collector, FinalizeCollection);
            let _ = self.send::<_, LedgerActor>(state.ledger, FinalizeLedger);
            ::actrs::ActorResult::Stop
        } else {
            ::actrs::ActorResult::Ok(state)
        }
    }

}

// 3. WorkerActor
pub struct WorkerActor {
    _handle: Option<actrs::ActorHandle>,
}

#[derive(Debug, Clone)]
pub struct WorkerState {
    pub worker_id: usize,
    pub dispatcher: ActorRef,
    pub collector: ActorRef,
    pub ledger: ActorRef,
    pub work_iterations: usize,
    pub processed_count: u64,
}

impl Default for WorkerState {
    fn default() -> Self {
        Self {
            worker_id: 0,
            dispatcher: ActorRef(0),
            collector: ActorRef(0),
            ledger: ActorRef(0),
            work_iterations: 0,
            processed_count: 0,
        }
    }
}

#[actor(state = WorkerState, handle = _handle)]
impl WorkerActor {
    #[initialize]
    fn initialize(&self, state: WorkerState) -> WorkerState {
        // Automatically register with the dispatcher on initialization
        let self_ref = self._handle.as_ref().expect("handle not initialized").self_ref();
        let _ = self.send::<_, DispatcherActor>(state.dispatcher, RegisterWorker {
            worker_id: state.worker_id,
            worker_ref: self_ref,
        });
        state
    }

    #[message_consumer(ProcessLine)]
    fn handle_process(&self, msg: ProcessLine, ctx: MsgCtx, state: WorkerState) -> WorkerState {
        let start = Instant::now();
        let dispatcher = ctx.from.expect("Worker needs dispatcher ref for ack");
        
        // Synthetic CPU work
        let mut checksum: u64 = 0;
        for _ in 0..state.work_iterations {
            for byte in msg.line.as_bytes() {
                checksum = checksum.wrapping_add(*byte as u64).wrapping_mul(31);
            }
        }

        let result = self.parse_line(&msg.line, msg.line_id, checksum, start.elapsed().as_micros() as u64, state.worker_id);
        
        match result {
            ProcessedLineResult::Valid(r) => {
                let _ = self.send::<_, CollectorActor>(state.collector, ProcessedLineResult::Valid(r.clone()));
                let _ = self.send::<_, LedgerActor>(state.ledger, LedgerEntry {
                    line_id: msg.line_id,
                    worker_id: state.worker_id,
                    processing_time_us: r.processing_time_us,
                    success: true,
                });
            }
            ProcessedLineResult::Invalid(r) => {
                let _ = self.send::<_, CollectorActor>(state.collector, ProcessedLineResult::Invalid(r.clone()));
                let _ = self.send::<_, LedgerActor>(state.ledger, LedgerEntry {
                    line_id: msg.line_id,
                    worker_id: state.worker_id,
                    processing_time_us: r.processing_time_us,
                    success: false,
                });
            }
        }

        let _ = self.send::<_, DispatcherActor>(dispatcher, WorkCompletedAck { line_id: msg.line_id, worker_id: state.worker_id });

        WorkerState {
            processed_count: state.processed_count + 1,
            ..state
        }
    }

    #[message_consumer(Shutdown)]
    fn handle_shutdown(&self, _msg: Shutdown, _ctx: MsgCtx, _state: WorkerState) -> ::actrs::ActorResult<WorkerState> {
        ::actrs::ActorResult::Stop
    }

    fn parse_line(&self, line: &str, line_id: u64, checksum: u64, proc_time_us: u64, worker_id: usize) -> ProcessedLineResult {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 2 {
            return ProcessedLineResult::Invalid(InvalidRecord {
                line_id,
                worker_id,
                error: "Line too short".to_string(),
                processing_time_us: proc_time_us,
            });
        }

        let _timestamp = parts[0].to_string();
        let level = parts[1].to_string();
        
        let mut service = None;
        let mut _user = None;
        let mut latency_ms = None;
        let mut status = None;
        let mut path = None;

        for part in &parts[2..] {
            if let Some((key, value)) = part.split_once('=') {
                match key {
                    "service" => service = Some(value.to_string()),
                    "user" => _user = Some(value.to_string()),
                    "latency" => latency_ms = value.parse().ok(),
                    "status" => status = value.parse().ok(),
                    "path" => path = Some(value.to_string()),
                    _ => {}
                }
            }
        }

        if let (Some(service), Some(_user), Some(latency_ms), Some(status), Some(path)) = 
            (service, _user, latency_ms, status, path) {
            ProcessedLineResult::Valid(ProcessedRecord {
                line_id,
                worker_id,
                service,
                status,
                latency_ms,
                path,
                level,
                processing_time_us: proc_time_us,
                checksum,
            })
        } else {
            ProcessedLineResult::Invalid(InvalidRecord {
                line_id,
                worker_id,
                error: "Missing required fields".to_string(),
                processing_time_us: proc_time_us,
            })
        }
    }
}

// 4. CollectorActor
pub struct CollectorActor {
    _handle: Option<actrs::ActorHandle>,
}

pub struct CollectorState {
    total_processed: u64,
    valid_count: u64,
    invalid_count: u64,
    service_counts: List<(String, u64)>,
    status_counts: List<(u16, u64)>,
    total_latency: u64,
}

#[actor(state = CollectorState, handle = _handle)]
impl CollectorActor {
    #[initialize]
    fn initialize(&self) -> CollectorState {
        CollectorState {
            total_processed: 0,
            valid_count: 0,
            invalid_count: 0,
            service_counts: List::empty(),
            status_counts: List::empty(),
            total_latency: 0,
        }
    }

    #[message_consumer(ProcessedLineResult)]
    fn handle_result(&self, msg: ProcessedLineResult, _ctx: MsgCtx, state: CollectorState) -> CollectorState {
        let total_processed = state.total_processed + 1;
        match msg {
            ProcessedLineResult::Valid(r) => {
                let valid_count = state.valid_count + 1;
                let total_latency = state.total_latency + r.latency_ms;
                
                let current_s_count = *state.service_counts.get_value(&r.service).unwrap_or(&0);
                let service_counts = state.service_counts.delete(&r.service).prepend((r.service, current_s_count + 1));

                let current_st_count = *state.status_counts.get_value(&r.status).unwrap_or(&0);
                let status_counts = state.status_counts.delete(&r.status).prepend((r.status, current_st_count + 1));

                CollectorState {
                    total_processed,
                    valid_count,
                    total_latency,
                    service_counts,
                    status_counts,
                    ..state
                }
            }
            ProcessedLineResult::Invalid(_) => {
                CollectorState {
                    total_processed,
                    invalid_count: state.invalid_count + 1,
                    ..state
                }
            }
        }
    }

    #[message_consumer(FinalizeCollection)]
    fn handle_finalize(&self, _msg: FinalizeCollection, _ctx: MsgCtx, state: CollectorState) -> ::actrs::ActorResult<CollectorState> {
        println!("\n--- Collector Summary ---");
        println!("Total Processed: {}", state.total_processed);
        println!("Valid:           {}", state.valid_count);
        println!("Invalid:         {}", state.invalid_count);
        
        if state.valid_count > 0 {
            println!("Average Latency: {:.2} ms", state.total_latency as f64 / state.valid_count as f64);
        }

        println!("\nService Counts:");
        let mut s_counts: Vec<_> = state.service_counts.iter().collect();
        s_counts.sort_by(|a, b| a.0.cmp(&b.0));
        for (service, count) in s_counts {
            println!("  {}: {}", service, count);
        }

        println!("\nStatus Counts:");
        let mut st_counts: Vec<_> = state.status_counts.iter().collect();
        st_counts.sort_by(|a, b| a.0.cmp(&b.0));
        for (status, count) in st_counts {
            println!("  {}: {}", status, count);
        }
        
        ::actrs::ActorResult::Stop
    }
}

// 5. LedgerActor
pub struct LedgerActor {
    _handle: Option<actrs::ActorHandle>,
}

#[derive(Clone, Debug)]
pub struct WorkerLedgerStats {
    lines_handled: u64,
    valid_count: u64,
    invalid_count: u64,
    total_proc_time: u64,
    min_proc_time: u64,
    max_proc_time: u64,
}

#[derive(Debug, Clone)]
pub struct LedgerState {
    output_path: String,
    input_path: String,
    start_time: Instant,
    worker_stats: List<(usize, WorkerLedgerStats)>,
    total_events: u64,
}

impl Default for LedgerState {
    fn default() -> Self {
        Self {
            output_path: String::new(),
            input_path: String::new(),
            start_time: Instant::now(),
            worker_stats: List::empty(),
            total_events: 0,
        }
    }
}

#[actor(state = LedgerState, handle = _handle)]
impl LedgerActor {
    #[initialize]
    fn initialize(&self, state: LedgerState) -> LedgerState {
        state
    }

    #[message_consumer(LedgerEntry)]
    fn handle_entry(&self, msg: LedgerEntry, _ctx: MsgCtx, state: LedgerState) -> LedgerState {
        let total_events = state.total_events + 1;
        let stats_before = state.worker_stats.get_value(&msg.worker_id).cloned().unwrap_or(WorkerLedgerStats {
            lines_handled: 0,
            valid_count: 0,
            invalid_count: 0,
            total_proc_time: 0,
            min_proc_time: u64::MAX,
            max_proc_time: 0,
        });

        let stats = WorkerLedgerStats {
            lines_handled: stats_before.lines_handled + 1,
            valid_count: stats_before.valid_count + if msg.success { 1 } else { 0 },
            invalid_count: stats_before.invalid_count + if msg.success { 0 } else { 1 },
            total_proc_time: stats_before.total_proc_time + msg.processing_time_us,
            min_proc_time: stats_before.min_proc_time.min(msg.processing_time_us),
            max_proc_time: stats_before.max_proc_time.max(msg.processing_time_us),
        };

        let worker_stats = state.worker_stats.delete(&msg.worker_id).prepend((msg.worker_id, stats));
        LedgerState {
            total_events,
            worker_stats,
            ..state
        }
    }

    #[message_consumer(FinalizeLedger)]
    fn handle_finalize(&self, _msg: FinalizeLedger, _ctx: MsgCtx, state: LedgerState) -> ::actrs::ActorResult<LedgerState> {
        let elapsed = state.start_time.elapsed();
        let mut file = match File::create(&state.output_path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("LedgerActor: Failed to create ledger file {}: {}", state.output_path, e);
                return ::actrs::ActorResult::Ok(state);
            }
        };

        let mut write = |s: String| { let _ = writeln!(file, "{}", s); };

        write(format!("Log Processing Ledger"));
        write(format!("====================="));
        write(format!("Input file: {}", state.input_path));
        write(format!("Overall elapsed wall time: {:?}", elapsed));
        write(format!("Total entries processed: {}", state.total_events));
        write(format!("Total workers: {}", state.worker_stats.iter().count()));
        write(format!(""));

        let mut sorted_stats: Vec<_> = state.worker_stats.iter().collect();
        sorted_stats.sort_by_key(|(id, _)| *id);

        for (id, stats) in sorted_stats {
            write(format!("Worker ID: {}", id));
            write(format!("  Lines handled:         {}", stats.lines_handled));
            write(format!("  Valid lines:           {}", stats.valid_count));
            write(format!("  Invalid lines:         {}", stats.invalid_count));
            write(format!("  Total processing time: {} us", stats.total_proc_time));
            if stats.lines_handled > 0 {
                write(format!("  Average processing time: {:.2} us", stats.total_proc_time as f64 / stats.lines_handled as f64));
            }
            write(format!("  Min processing time:   {} us", stats.min_proc_time));
            write(format!("  Max processing time:   {} us", stats.max_proc_time));
            write(format!(""));
        }

        println!("\nLedger written to: {}", state.output_path);
        ::actrs::ActorResult::Stop
    }
}

// --- Helper functions ---

fn generate_sample_log(path: &str, count: usize) {
    let mut file = File::create(path).expect("Failed to create sample log file");
    let services = vec!["auth", "billing", "inventory", "shipping", "frontend"];
    let users = vec!["alice", "bob", "charlie", "delta", "echo"];
    let levels = vec!["INFO", "WARN", "ERROR"];
    let paths = vec!["/login", "/checkout", "/items", "/profile", "/api/v1/status"];

    for i in 0..count {
        let timestamp = format!("2026-03-28T12:00:{:02}Z", i % 60);
        let level = levels[i % levels.len()];
        let service = services[i % services.len()];
        let user = users[i % users.len()];
        let path_str = paths[i % paths.len()];
        let latency = 10 + (i % 100);
        let status = if i % 10 == 0 { 500 } else if i % 7 == 0 { 404 } else { 200 };

        if i % 50 == 0 {
            // Malformed line
            writeln!(file, "MALFORMED LINE WITHOUT PROPER FORMATTING").unwrap();
        } else {
            writeln!(file, "{} {} service={} user={} latency={} status={} path={}", 
                timestamp, level, service, user, latency, status, path_str).unwrap();
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let input_path = args.get(1).map(|s| s.as_str()).unwrap_or(DEFAULT_INPUT_PATH).to_string();
    let output_path = args.get(2).map(|s| s.as_str()).unwrap_or(DEFAULT_LEDGER_PATH).to_string();

    if !std::path::Path::new(&input_path).exists() {
        println!("Input file not found, generating sample log: {}", input_path);
        generate_sample_log(&input_path, 1000);
    }

    let stage = Stage::new(8, 10, Duration::from_millis(5));

    let collector = stage.add_actor(CollectorActor { _handle: None }, ());
    let ledger = stage.add_actor(LedgerActor { _handle: None }, LedgerState {
        output_path: output_path.clone(),
        input_path: input_path.clone(),
        start_time: Instant::now(),
        worker_stats: List::empty(),
        total_events: 0,
    });

    let dispatcher = stage.add_actor(DispatcherActor { _handle: None }, DispatcherState {
        workers: List::empty(),
        collector,
        ledger,
        next_worker_idx: 0,
        total_received: 0,
        total_dispatched: 0,
        total_completed: 0,
        reader_done: false,
        expected_total: 0,
    });

    for i in 0..WORKER_COUNT {
        stage.add_actor(WorkerActor { _handle: None }, WorkerState {
            worker_id: i,
            dispatcher,
            collector,
            ledger,
            work_iterations: WORK_ITERATIONS,
            processed_count: 0,
        });
    }

    let reader = stage.add_actor(ReaderActor { _handle: None }, ReaderState {
        input_path: input_path.clone(),
        dispatcher,
        total_lines: 0,
        started: false,
    });

    println!("Starting demo: reading from {}, writing ledger to {}", input_path, output_path);
    
    let start_msg = StartReading;
    stage.send(reader, None, Box::new(ReaderActorMessage::from(start_msg)));

    // Wait for all actors to finish
    stage.wait_for_completion();
    
    stage.shutdown();
    
    println!("Demo complete, all actors finished and stage shutdown.");
}
