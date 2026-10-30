//! The host that owns the model: it loads on first use, unloads when idle, and keeps every
//! caller inside its deadline even when the engine does not return.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use wf_pipeline::llm_host::{KeepLoaded, LlmHost};
use wf_pipeline::{Llm, LlmPrompt, PipelineError};

/// Counts loads and live engines so tests can see the model come and go.
#[derive(Clone, Default)]
struct Probe {
    loads: Arc<AtomicUsize>,
    alive: Arc<AtomicUsize>,
    generations: Arc<AtomicUsize>,
}

impl Probe {
    fn loads(&self) -> usize {
        self.loads.load(Ordering::SeqCst)
    }

    fn alive(&self) -> usize {
        self.alive.load(Ordering::SeqCst)
    }

    fn generations(&self) -> usize {
        self.generations.load(Ordering::SeqCst)
    }

    fn host(&self, keep: KeepLoaded) -> LlmHost {
        let probe = self.clone();
        LlmHost::spawn(
            move || {
                probe.loads.fetch_add(1, Ordering::SeqCst);
                probe.alive.fetch_add(1, Ordering::SeqCst);
                Ok(Box::new(FakeEngine {
                    probe: probe.clone(),
                    hold: Duration::ZERO,
                }) as Box<dyn Llm>)
            },
            keep,
        )
    }
}

struct FakeEngine {
    probe: Probe,
    /// How long a generation takes, ignoring any deadline.
    hold: Duration,
}

impl Llm for FakeEngine {
    fn generate(
        &mut self,
        prompt: &LlmPrompt,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<String, PipelineError> {
        self.probe.generations.fetch_add(1, Ordering::SeqCst);
        thread::sleep(self.hold);
        for word in prompt.user.split_inclusive(' ') {
            on_token(word);
        }
        Ok(prompt.user.clone())
    }
}

impl Drop for FakeEngine {
    fn drop(&mut self) {
        self.probe.alive.fetch_sub(1, Ordering::SeqCst);
    }
}

fn prompt(user: &str) -> LlmPrompt {
    LlmPrompt {
        user: user.to_owned(),
        ..LlmPrompt::default()
    }
}

fn eventually(mut condition: impl FnMut() -> bool) -> bool {
    let give_up = Instant::now() + Duration::from_secs(5);
    while Instant::now() < give_up {
        if condition() {
            return true;
        }
        thread::sleep(Duration::from_millis(2));
    }
    false
}

#[test]
fn spawning_a_host_does_not_load_the_model() {
    let probe = Probe::default();

    let _host = probe.host(KeepLoaded::Always);

    thread::sleep(Duration::from_millis(20));
    assert_eq!(probe.loads(), 0);
}

#[test]
fn the_first_generation_loads_the_model_and_later_ones_reuse_it() {
    let probe = Probe::default();
    let mut host = probe.host(KeepLoaded::Always);

    let first = host.generate(&prompt("one"), &mut |_| {}).unwrap();
    let second = host.generate(&prompt("two"), &mut |_| {}).unwrap();

    assert_eq!((first.as_str(), second.as_str()), ("one", "two"));
    assert_eq!(probe.loads(), 1);
}

#[test]
fn tokens_stream_to_the_caller_in_order() {
    let probe = Probe::default();
    let mut host = probe.host(KeepLoaded::Always);
    let mut tokens = Vec::new();

    host.generate(&prompt("ship it now"), &mut |t| tokens.push(t.to_owned()))
        .unwrap();

    assert_eq!(tokens, ["ship ", "it ", "now"]);
}

#[test]
fn idle_time_unloads_the_model_and_the_next_generation_loads_it_again() {
    let probe = Probe::default();
    let mut host = probe.host(KeepLoaded::For(Duration::from_millis(30)));

    host.generate(&prompt("one"), &mut |_| {}).unwrap();
    assert!(eventually(|| probe.alive() == 0), "still loaded when idle");

    host.generate(&prompt("two"), &mut |_| {}).unwrap();
    assert_eq!(probe.loads(), 2);
}

#[test]
fn use_within_the_idle_window_keeps_the_model_loaded() {
    let probe = Probe::default();
    let mut host = probe.host(KeepLoaded::For(Duration::from_secs(60)));

    host.generate(&prompt("one"), &mut |_| {}).unwrap();
    thread::sleep(Duration::from_millis(30));
    host.generate(&prompt("two"), &mut |_| {}).unwrap();

    assert_eq!(probe.loads(), 1);
    assert_eq!(probe.alive(), 1);
}

#[test]
fn unload_after_use_drops_the_model_after_each_generation() {
    let probe = Probe::default();
    let mut host = probe.host(KeepLoaded::UnloadAfterUse);

    host.generate(&prompt("one"), &mut |_| {}).unwrap();
    assert!(eventually(|| probe.alive() == 0), "still loaded after use");

    host.generate(&prompt("two"), &mut |_| {}).unwrap();
    assert_eq!(probe.loads(), 2);
}

#[test]
fn dropping_the_last_handle_unloads_the_model() {
    let probe = Probe::default();
    let mut host = probe.host(KeepLoaded::Always);
    let second_handle = host.clone();

    host.generate(&prompt("one"), &mut |_| {}).unwrap();
    drop(host);
    thread::sleep(Duration::from_millis(20));
    assert_eq!(probe.alive(), 1, "the clone still uses it");

    drop(second_handle);
    assert!(
        eventually(|| probe.alive() == 0),
        "leaked after last handle"
    );
}

#[test]
fn clones_share_one_loaded_model() {
    let probe = Probe::default();
    let mut dictation = probe.host(KeepLoaded::Always);
    let mut meetings = dictation.clone();

    dictation.generate(&prompt("one"), &mut |_| {}).unwrap();
    meetings.generate(&prompt("two"), &mut |_| {}).unwrap();

    assert_eq!(probe.loads(), 1);
}

#[test]
fn a_failed_load_is_reported_and_retried_on_the_next_generation() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let probe = Probe::default();
    let counted = attempts.clone();
    let mut host = LlmHost::spawn(
        move || {
            if counted.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err(PipelineError::ModelMissing("qwen".into()));
            }
            Ok(Box::new(FakeEngine {
                probe: probe.clone(),
                hold: Duration::ZERO,
            }) as Box<dyn Llm>)
        },
        KeepLoaded::Always,
    );

    let first = host.generate(&prompt("one"), &mut |_| {});
    let second = host.generate(&prompt("two"), &mut |_| {});

    assert!(matches!(first, Err(PipelineError::ModelMissing(_))));
    assert_eq!(second.unwrap(), "two");
}

fn slow_host(probe: &Probe, hold: Duration) -> LlmHost {
    let probe = probe.clone();
    LlmHost::spawn(
        move || {
            Ok(Box::new(FakeEngine {
                probe: probe.clone(),
                hold,
            }) as Box<dyn Llm>)
        },
        KeepLoaded::Always,
    )
}

#[test]
fn a_caller_gets_its_timeout_even_if_the_engine_never_checks_the_deadline() {
    let probe = Probe::default();
    let mut host = slow_host(&probe, Duration::from_secs(2));
    let started = Instant::now();
    let deadline = started + Duration::from_millis(50);

    let result = host.generate(
        &LlmPrompt {
            deadline: Some(deadline),
            ..prompt("one")
        },
        &mut |_| {},
    );

    assert!(matches!(result, Err(PipelineError::Timeout)));
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn a_slow_model_load_times_out_the_caller_but_still_finishes_for_the_next_one() {
    let loaded = Arc::new(AtomicUsize::new(0));
    let done = loaded.clone();
    let mut host = LlmHost::spawn(
        move || {
            thread::sleep(Duration::from_millis(100));
            done.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(FakeEngine {
                probe: Probe::default(),
                hold: Duration::ZERO,
            }) as Box<dyn Llm>)
        },
        KeepLoaded::Always,
    );

    let first = host.generate(
        &LlmPrompt {
            deadline: Some(Instant::now() + Duration::from_millis(10)),
            ..prompt("one")
        },
        &mut |_| {},
    );
    assert!(matches!(first, Err(PipelineError::Timeout)));

    assert!(eventually(|| loaded.load(Ordering::SeqCst) == 1));
    assert_eq!(host.generate(&prompt("two"), &mut |_| {}).unwrap(), "two");
}

#[test]
fn work_whose_caller_already_gave_up_is_skipped() {
    let probe = Probe::default();
    let mut host = slow_host(&probe, Duration::from_millis(100));
    let results = Arc::new(Mutex::new(Vec::new()));

    let mut busy = host.clone();
    let blocker = thread::spawn(move || busy.generate(&prompt("slow"), &mut |_| {}));
    assert!(eventually(|| probe.generations() == 1));

    // Queued behind the slow job, and out of time before the worker reaches it.
    let result = host.generate(
        &LlmPrompt {
            deadline: Some(Instant::now() + Duration::from_millis(10)),
            ..prompt("late")
        },
        &mut |_| {},
    );
    results.lock().unwrap().push(result.is_err());
    blocker.join().unwrap().unwrap();

    // The queue moves on: this one is served, and the late job never reached the engine.
    assert_eq!(host.generate(&prompt("next"), &mut |_| {}).unwrap(), "next");
    assert_eq!(probe.generations(), 2);
    assert_eq!(*results.lock().unwrap(), [true]);
}

#[test]
fn hosts_can_move_to_a_worker_thread_as_boxed_engines() {
    let probe = Probe::default();
    let mut llm: Box<dyn Llm> = Box::new(probe.host(KeepLoaded::Always));

    let reply = thread::spawn(move || llm.generate(&prompt("one"), &mut |_| {}))
        .join()
        .unwrap();

    assert_eq!(reply.unwrap(), "one");
}

/// Runs until `stop` is called, like a process being killed from outside.
struct StoppableEngine {
    stopped: Arc<AtomicBool>,
    entered: Arc<AtomicBool>,
}

impl Llm for StoppableEngine {
    fn generate(
        &mut self,
        _prompt: &LlmPrompt,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<String, PipelineError> {
        self.entered.store(true, Ordering::SeqCst);
        on_token("first ");
        while !self.stopped.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(1));
        }
        Err(PipelineError::Engine("killed".into()))
    }
}

struct Stoppable {
    host: LlmHost,
    stopped: Arc<AtomicBool>,
    entered: Arc<AtomicBool>,
    loads: Arc<AtomicUsize>,
}

fn stoppable() -> Stoppable {
    let stopped = Arc::new(AtomicBool::new(false));
    let entered = Arc::new(AtomicBool::new(false));
    let loads = Arc::new(AtomicUsize::new(0));
    let (engine_stopped, engine_entered, counted) =
        (stopped.clone(), entered.clone(), loads.clone());
    let stop = stopped.clone();
    let host = LlmHost::spawn_stoppable(
        move || {
            counted.fetch_add(1, Ordering::SeqCst);
            engine_stopped.store(false, Ordering::SeqCst);
            Ok(Box::new(StoppableEngine {
                stopped: engine_stopped.clone(),
                entered: engine_entered.clone(),
            }) as Box<dyn Llm>)
        },
        KeepLoaded::Always,
        move || stop.store(true, Ordering::SeqCst),
    );
    Stoppable {
        host,
        stopped,
        entered,
        loads,
    }
}

#[test]
fn cancelling_a_running_generation_stops_the_engine_and_the_next_call_reloads() {
    let stoppable = stoppable();
    let cancel = Arc::new(AtomicBool::new(false));
    let trigger = cancel.clone();
    let entered = stoppable.entered.clone();
    thread::spawn(move || {
        while !entered.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(1));
        }
        trigger.store(true, Ordering::SeqCst);
    });

    let mut tokens = Vec::new();
    let result = stoppable
        .host
        .generate_until(&prompt("question"), &cancel, &mut |token| {
            tokens.push(token.to_owned())
        });

    assert!(
        matches!(result, Err(PipelineError::Cancelled)),
        "{result:?}"
    );
    assert_eq!(tokens, ["first "]);
    assert!(stoppable.stopped.load(Ordering::SeqCst));

    stoppable.entered.store(false, Ordering::SeqCst);
    let again = Arc::new(AtomicBool::new(false));
    let trigger = again.clone();
    let entered = stoppable.entered.clone();
    thread::spawn(move || {
        while !entered.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(1));
        }
        trigger.store(true, Ordering::SeqCst);
    });
    let result = stoppable
        .host
        .generate_until(&prompt("again"), &again, &mut |_| {});
    assert!(
        matches!(result, Err(PipelineError::Cancelled)),
        "{result:?}"
    );
    assert_eq!(stoppable.loads.load(Ordering::SeqCst), 2);
}

#[test]
fn cancelling_a_call_still_in_the_queue_leaves_the_running_one_alone() {
    let probe = Probe::default();
    let counted = probe.clone();
    let stopped = Arc::new(AtomicBool::new(false));
    let stop = stopped.clone();
    let host = LlmHost::spawn_stoppable(
        move || {
            counted.loads.fetch_add(1, Ordering::SeqCst);
            counted.alive.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(FakeEngine {
                probe: counted.clone(),
                hold: Duration::from_millis(150),
            }) as Box<dyn Llm>)
        },
        KeepLoaded::Always,
        move || stop.store(true, Ordering::SeqCst),
    );
    let mut first = host.clone();
    let running = thread::spawn(move || first.generate(&prompt("first"), &mut |_| {}));
    assert!(eventually(|| probe.generations() == 1));

    let cancel = Arc::new(AtomicBool::new(true));
    let queued = host.generate_until(&prompt("second"), &cancel, &mut |_| {});

    assert!(
        matches!(queued, Err(PipelineError::Cancelled)),
        "{queued:?}"
    );
    assert_eq!(running.join().unwrap().unwrap(), "first");
    assert!(
        !stopped.load(Ordering::SeqCst),
        "the running call was not ours"
    );
    thread::sleep(Duration::from_millis(50));
    assert_eq!(probe.generations(), 1, "the queued call never ran");
}

#[test]
fn a_call_nobody_cancels_finishes_normally() {
    let probe = Probe::default();
    let host = probe.host(KeepLoaded::Always);
    let cancel = AtomicBool::new(false);

    let result = host.generate_until(&prompt("hello there"), &cancel, &mut |_| {});

    assert_eq!(result.unwrap(), "hello there");
}
