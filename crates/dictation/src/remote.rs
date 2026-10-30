//! Engines in child processes (ADR-0005). Native engines don't give their memory back when dropped,
//! so an engine lives in its own short-lived process and unloading it ends that process. The same
//! wall keeps a crash in an engine, such as a read from an unplugged drive, away from the app.
//! There are four kinds of host: VAD, ASR, LLM and diarization.
//!
//! [`serve`] is the child's side, and [`ChildEngines`] the app's. A message is a JSON header and
//! raw samples, each with a length in front. One call runs at a time.

use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use wf_core::audio::AudioClip;
use wf_pipeline::{
    Asr, AsrOptions, Diarizer, Llm, LlmPrompt, PipelineError, SpeakerTurn, SpeechSegment,
    Transcript, Vad,
};

use crate::engines::Engines;
use crate::plan::{AsrModel, DiarizerModel, LlmModel};

/// The argument that turns the app's binary into an engine host.
pub const ENGINE_HOST_ARG: &str = "--engine-host";

/// What a host should load.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EngineSpec {
    Silero(PathBuf),
    Whisper(PathBuf),
    Sherpa(PathBuf),
    Llama {
        file: PathBuf,
        in_ram: bool,
    },
    Diarizer {
        segmentation: PathBuf,
        embedding: PathBuf,
    },
}

#[derive(Debug, Serialize, Deserialize)]
enum Call {
    Load(EngineSpec),
    /// The samples follow the header.
    Accept,
    Finish,
    /// The samples follow the header.
    Transcribe {
        language: Option<String>,
        bias_words: Vec<String>,
    },
    Generate {
        system: String,
        user: String,
        examples: Vec<(String, String)>,
        max_tokens: u32,
        /// How long from now the caller will wait.
        timeout_ms: Option<u64>,
    },
    /// The samples follow the header.
    Diarize,
}

#[derive(Debug, Serialize, Deserialize)]
enum Reply {
    Done,
    Segments {
        segments: Vec<(usize, usize)>,
        speaking: bool,
    },
    Transcript {
        text: String,
        language: Option<String>,
    },
    Token(String),
    Completion(String),
    /// Start and end in milliseconds, and the engine's number for the voice.
    Turns(Vec<(u64, u64, u32)>),
    Failed(Failure),
}

#[derive(Debug, Serialize, Deserialize)]
enum Failure {
    ModelMissing(String),
    Timeout,
    Engine(String),
}

impl From<PipelineError> for Failure {
    fn from(error: PipelineError) -> Self {
        match error {
            PipelineError::ModelMissing(model) => Self::ModelMissing(model),
            PipelineError::Timeout => Self::Timeout,
            PipelineError::Cancelled => Self::Engine("cancelled".to_owned()),
            PipelineError::Engine(message) => Self::Engine(message),
        }
    }
}

impl From<Failure> for PipelineError {
    fn from(failure: Failure) -> Self {
        match failure {
            Failure::ModelMissing(model) => Self::ModelMissing(model),
            Failure::Timeout => Self::Timeout,
            Failure::Engine(message) => Self::Engine(message),
        }
    }
}

fn write_message<T: Serialize>(
    output: &mut impl Write,
    message: &T,
    samples: &[f32],
) -> io::Result<()> {
    let header = serde_json::to_vec(message)?;
    output.write_all(&(header.len() as u32).to_le_bytes())?;
    output.write_all(&(samples.len() as u32).to_le_bytes())?;
    output.write_all(&header)?;
    for sample in samples {
        output.write_all(&sample.to_le_bytes())?;
    }
    output.flush()
}

/// `None` at the end of the stream.
fn read_message<T: DeserializeOwned>(input: &mut impl Read) -> io::Result<Option<(T, Vec<f32>)>> {
    let mut lengths = [0; 8];
    if let Err(error) = input.read_exact(&mut lengths) {
        return match error.kind() {
            io::ErrorKind::UnexpectedEof => Ok(None),
            _ => Err(error),
        };
    }
    let (header, samples) = lengths.split_at(4);
    let header = u32::from_le_bytes(header.try_into().expect("four bytes")) as usize;
    let samples = u32::from_le_bytes(samples.try_into().expect("four bytes")) as usize;

    let mut bytes = vec![0; header];
    input.read_exact(&mut bytes)?;
    let message = serde_json::from_slice(&bytes)?;

    let mut bytes = vec![0; samples * size_of::<f32>()];
    input.read_exact(&mut bytes)?;
    let samples = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|&sample| f32::from_le_bytes(sample))
        .collect();
    Ok(Some((message, samples)))
}

/// One loaded engine, on the child's side.
pub enum Hosted {
    Vad(Box<dyn Vad>),
    Asr(Box<dyn Asr>),
    Llm(Box<dyn Llm>),
    Diarizer(Box<dyn Diarizer>),
}

pub type LoadEngine = dyn FnMut(&EngineSpec) -> Result<Hosted, PipelineError>;

/// The child's loop: answers calls until the app closes the stream.
pub fn serve(input: impl Read, output: impl Write, load: &mut LoadEngine) -> io::Result<()> {
    let mut input = BufReader::new(input);
    let mut output = BufWriter::new(output);
    let mut engine = None;
    while let Some((call, samples)) = read_message::<Call>(&mut input)? {
        let reply = answer(call, samples, &mut engine, load, &mut output)?;
        write_message(&mut output, &reply, &[])?;
    }
    Ok(())
}

fn answer(
    call: Call,
    samples: Vec<f32>,
    engine: &mut Option<Hosted>,
    load: &mut LoadEngine,
    output: &mut impl Write,
) -> io::Result<Reply> {
    let failed = |error: PipelineError| Reply::Failed(error.into());
    Ok(match (call, engine.as_mut()) {
        (Call::Load(spec), _) => match load(&spec) {
            Ok(loaded) => {
                *engine = Some(loaded);
                Reply::Done
            }
            Err(error) => failed(error),
        },
        (Call::Accept, Some(Hosted::Vad(vad))) => match vad.accept(&samples) {
            Ok(segments) => segments_reply(segments, vad.is_speaking()),
            Err(error) => failed(error),
        },
        (Call::Finish, Some(Hosted::Vad(vad))) => match vad.finish() {
            Ok(segments) => segments_reply(segments, false),
            Err(error) => failed(error),
        },
        (
            Call::Transcribe {
                language,
                bias_words,
            },
            Some(Hosted::Asr(asr)),
        ) => {
            let options = AsrOptions {
                language,
                bias_words,
            };
            match asr.transcribe(&AudioClip::new(samples), &options) {
                Ok(Transcript { text, language }) => Reply::Transcript { text, language },
                Err(error) => failed(error),
            }
        }
        (
            Call::Generate {
                system,
                user,
                examples,
                max_tokens,
                timeout_ms,
            },
            Some(Hosted::Llm(llm)),
        ) => {
            let prompt = LlmPrompt {
                system,
                user,
                examples,
                max_tokens,
                deadline: timeout_ms.map(|ms| Instant::now() + Duration::from_millis(ms)),
            };
            let mut stream_failed = None;
            let result = llm.generate(&prompt, &mut |token| {
                let token = Reply::Token(token.to_owned());
                if let Err(error) = write_message(output, &token, &[]) {
                    stream_failed = Some(error);
                }
            });
            if let Some(error) = stream_failed {
                return Err(error);
            }
            match result {
                Ok(text) => Reply::Completion(text),
                Err(error) => failed(error),
            }
        }
        (Call::Diarize, Some(Hosted::Diarizer(diarizer))) => {
            match diarizer.diarize(&AudioClip::new(samples)) {
                Ok(turns) => Reply::Turns(
                    turns
                        .iter()
                        .map(|turn| {
                            let (start, end) = (turn.start.as_millis(), turn.end.as_millis());
                            (start as u64, end as u64, turn.speaker)
                        })
                        .collect(),
                ),
                Err(error) => failed(error),
            }
        }
        _ => failed(PipelineError::Engine("no such engine is loaded".into())),
    })
}

fn segments_reply(segments: Vec<SpeechSegment>, speaking: bool) -> Reply {
    Reply::Segments {
        segments: segments.iter().map(|s| (s.start, s.end)).collect(),
        speaking,
    }
}

/// A running engine host. Unloading is ending the process, so dropping this kills it.
struct Process {
    child: Mutex<Child>,
    /// A call is in flight, so an interrupt has something to stop.
    busy: AtomicBool,
}

impl Process {
    fn kill(&self) {
        let _ = self.lock().kill();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Child> {
        self.child.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let mut child = self.lock();
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// The app's end of one host.
struct Connection {
    process: Arc<Process>,
    input: BufWriter<ChildStdin>,
    output: BufReader<ChildStdout>,
    name: &'static str,
}

impl Connection {
    fn stopped(&self) -> PipelineError {
        PipelineError::Engine(format!("the {} engine stopped", self.name))
    }

    fn send(&mut self, call: &Call, samples: &[f32]) -> Result<(), PipelineError> {
        self.process.busy.store(true, Ordering::SeqCst);
        write_message(&mut self.input, call, samples).map_err(|_| self.stopped())
    }

    fn receive(&mut self) -> Result<Reply, PipelineError> {
        match read_message::<Reply>(&mut self.output) {
            Ok(Some((Reply::Failed(failure), _))) => Err(failure.into()),
            Ok(Some((reply, _))) => Ok(reply),
            Ok(None) | Err(_) => Err(self.stopped()),
        }
    }

    fn call(&mut self, call: &Call, samples: &[f32]) -> Result<Reply, PipelineError> {
        self.send(call, samples)?;
        let reply = self.receive();
        self.process.busy.store(false, Ordering::SeqCst);
        reply
    }

    fn unexpected(&self) -> PipelineError {
        PipelineError::Engine(format!("the {} engine sent the wrong reply", self.name))
    }
}

struct RemoteVad {
    connection: Connection,
    speaking: bool,
}

impl RemoteVad {
    fn segments(&mut self, reply: Reply) -> Result<Vec<SpeechSegment>, PipelineError> {
        let Reply::Segments { segments, speaking } = reply else {
            return Err(self.connection.unexpected());
        };
        self.speaking = speaking;
        Ok(segments
            .into_iter()
            .map(|(start, end)| SpeechSegment { start, end })
            .collect())
    }
}

impl Vad for RemoteVad {
    fn accept(&mut self, samples: &[f32]) -> Result<Vec<SpeechSegment>, PipelineError> {
        let reply = self.connection.call(&Call::Accept, samples)?;
        self.segments(reply)
    }

    fn is_speaking(&self) -> bool {
        self.speaking
    }

    fn finish(&mut self) -> Result<Vec<SpeechSegment>, PipelineError> {
        let reply = self.connection.call(&Call::Finish, &[])?;
        self.segments(reply)
    }
}

struct RemoteAsr(Connection);

impl Asr for RemoteAsr {
    fn transcribe(
        &mut self,
        clip: &AudioClip,
        options: &AsrOptions,
    ) -> Result<Transcript, PipelineError> {
        let call = Call::Transcribe {
            language: options.language.clone(),
            bias_words: options.bias_words.clone(),
        };
        match self.0.call(&call, clip.samples())? {
            Reply::Transcript { text, language } => Ok(Transcript { text, language }),
            _ => Err(self.0.unexpected()),
        }
    }
}

struct RemoteLlm(Connection);

impl Llm for RemoteLlm {
    fn generate(
        &mut self,
        prompt: &LlmPrompt,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<String, PipelineError> {
        let timeout_ms = prompt.deadline.map(|deadline| {
            deadline
                .saturating_duration_since(Instant::now())
                .as_millis() as u64
        });
        let call = Call::Generate {
            system: prompt.system.clone(),
            user: prompt.user.clone(),
            examples: prompt.examples.clone(),
            max_tokens: prompt.max_tokens,
            timeout_ms,
        };
        self.0.send(&call, &[])?;
        let result = loop {
            match self.0.receive() {
                Ok(Reply::Token(token)) => on_token(&token),
                Ok(Reply::Completion(text)) => break Ok(text),
                Ok(_) => break Err(self.0.unexpected()),
                Err(error) => break Err(error),
            }
        };
        self.0.process.busy.store(false, Ordering::SeqCst);
        result
    }
}

struct RemoteDiarizer(Connection);

impl Diarizer for RemoteDiarizer {
    fn diarize(&mut self, clip: &AudioClip) -> Result<Vec<SpeakerTurn>, PipelineError> {
        match self.0.call(&Call::Diarize, clip.samples())? {
            Reply::Turns(turns) => Ok(turns
                .into_iter()
                .map(|(start, end, speaker)| SpeakerTurn {
                    start: Duration::from_millis(start),
                    end: Duration::from_millis(end),
                    speaker,
                })
                .collect()),
            _ => Err(self.0.unexpected()),
        }
    }
}

/// [`Engines`] that run in hosts started from `executable`, the app's own binary.
pub struct ChildEngines {
    executable: PathBuf,
    running: Mutex<Vec<Weak<Process>>>,
}

impl ChildEngines {
    pub fn new(executable: PathBuf) -> Self {
        Self {
            executable,
            running: Mutex::default(),
        }
    }

    fn start(&self, spec: EngineSpec, name: &'static str) -> Result<Connection, PipelineError> {
        let mut child = Command::new(&self.executable)
            .arg(ENGINE_HOST_ARG)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|error| PipelineError::Engine(format!("can't start the engine: {error}")))?;
        let input = child.stdin.take().expect("stdin is piped");
        let output = child.stdout.take().expect("stdout is piped");
        let process = Arc::new(Process {
            child: Mutex::new(child),
            busy: AtomicBool::new(false),
        });
        let mut running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
        running.retain(|process| process.strong_count() > 0);
        running.push(Arc::downgrade(&process));
        drop(running);

        let mut connection = Connection {
            process,
            input: BufWriter::new(input),
            output: BufReader::new(output),
            name,
        };
        connection.call(&Call::Load(spec), &[])?;
        Ok(connection)
    }
}

impl Engines for ChildEngines {
    fn vad(&self, model: &Path) -> Result<Box<dyn Vad>, PipelineError> {
        let connection = self.start(EngineSpec::Silero(model.to_owned()), "voice detection")?;
        Ok(Box::new(RemoteVad {
            connection,
            speaking: false,
        }))
    }

    fn asr(&self, model: &AsrModel) -> Result<Box<dyn Asr>, PipelineError> {
        let spec = match model {
            AsrModel::Whisper(file) => EngineSpec::Whisper(file.clone()),
            AsrModel::Sherpa(dir) => EngineSpec::Sherpa(dir.clone()),
        };
        Ok(Box::new(RemoteAsr(self.start(spec, "speech")?)))
    }

    fn llm(&self, model: &LlmModel) -> Result<Box<dyn Llm>, PipelineError> {
        let spec = EngineSpec::Llama {
            file: model.file.clone(),
            in_ram: model.in_ram,
        };
        Ok(Box::new(RemoteLlm(self.start(spec, "cleanup")?)))
    }

    fn diarizer(&self, model: &DiarizerModel) -> Result<Box<dyn Diarizer>, PipelineError> {
        let spec = EngineSpec::Diarizer {
            segmentation: model.segmentation.clone(),
            embedding: model.embedding.clone(),
        };
        Ok(Box::new(RemoteDiarizer(self.start(spec, "speaker")?)))
    }

    fn interrupt(&self) {
        let running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
        running
            .iter()
            .filter_map(Weak::upgrade)
            .filter(|process| process.busy.load(Ordering::SeqCst))
            .for_each(|process| process.kill());
    }

    fn scope(&self) -> Arc<dyn Engines> {
        Arc::new(Self::new(self.executable.clone()))
    }
}

/// Loads the real engines. Runs in the host process.
pub fn load_engine(spec: &EngineSpec) -> Result<Hosted, PipelineError> {
    use wf_pipeline::diarization::{DiarizationConfig, SherpaDiarizer};
    use wf_pipeline::llama::{LlamaConfig, LlamaEngine};
    use wf_pipeline::sherpa::SherpaAsr;
    use wf_pipeline::silero::SileroVad;
    use wf_pipeline::whisper::WhisperAsr;

    Ok(match spec {
        EngineSpec::Silero(file) => Hosted::Vad(Box::new(SileroVad::new(file)?)),
        EngineSpec::Whisper(file) => Hosted::Asr(Box::new(WhisperAsr::new(file)?)),
        EngineSpec::Sherpa(dir) => Hosted::Asr(Box::new(SherpaAsr::new(dir)?)),
        EngineSpec::Llama { file, in_ram } => {
            Hosted::Llm(Box::new(LlamaEngine::load(&LlamaConfig {
                use_mmap: !in_ram,
                ..LlamaConfig::new(file)
            })?))
        }
        EngineSpec::Diarizer {
            segmentation,
            embedding,
        } => Hosted::Diarizer(Box::new(SherpaDiarizer::new(&DiarizationConfig::new(
            segmentation,
            embedding,
        ))?)),
    })
}

/// The whole life of a host process: `main` calls this when it's given [`ENGINE_HOST_ARG`].
pub fn run_engine_host() -> io::Result<()> {
    serve(io::stdin().lock(), io::stdout().lock(), &mut load_engine)
}

#[cfg(test)]
mod tests {
    use std::io::pipe;
    use std::thread;

    use super::*;

    struct ScriptedVad;

    impl Vad for ScriptedVad {
        fn accept(&mut self, samples: &[f32]) -> Result<Vec<SpeechSegment>, PipelineError> {
            Ok(vec![SpeechSegment {
                start: 0,
                end: samples.len(),
            }])
        }

        fn is_speaking(&self) -> bool {
            true
        }

        fn finish(&mut self) -> Result<Vec<SpeechSegment>, PipelineError> {
            Err(PipelineError::Timeout)
        }
    }

    struct EchoLengthAsr;

    impl Asr for EchoLengthAsr {
        fn transcribe(
            &mut self,
            clip: &AudioClip,
            options: &AsrOptions,
        ) -> Result<Transcript, PipelineError> {
            Ok(Transcript {
                text: format!(
                    "{} samples, last {}",
                    clip.samples().len(),
                    clip.samples()[1]
                ),
                language: options.language.clone(),
            })
        }
    }

    struct TwoTokenLlm;

    impl Llm for TwoTokenLlm {
        fn generate(
            &mut self,
            prompt: &LlmPrompt,
            on_token: &mut dyn FnMut(&str),
        ) -> Result<String, PipelineError> {
            on_token("he");
            on_token("llo");
            assert!(prompt.deadline.is_some());
            Ok(prompt.user.clone())
        }
    }

    /// One voice for the first half of the clip, another for the rest.
    struct TwoVoices;

    impl Diarizer for TwoVoices {
        fn diarize(&mut self, clip: &AudioClip) -> Result<Vec<SpeakerTurn>, PipelineError> {
            let half = clip.duration() / 2;
            Ok(vec![
                SpeakerTurn {
                    start: Duration::ZERO,
                    end: half,
                    speaker: 0,
                },
                SpeakerTurn {
                    start: half,
                    end: clip.duration(),
                    speaker: 1,
                },
            ])
        }
    }

    fn load(spec: &EngineSpec) -> Result<Hosted, PipelineError> {
        match spec {
            EngineSpec::Silero(_) => Ok(Hosted::Vad(Box::new(ScriptedVad))),
            EngineSpec::Whisper(_) => Ok(Hosted::Asr(Box::new(EchoLengthAsr))),
            EngineSpec::Llama { .. } => Ok(Hosted::Llm(Box::new(TwoTokenLlm))),
            EngineSpec::Diarizer { .. } => Ok(Hosted::Diarizer(Box::new(TwoVoices))),
            EngineSpec::Sherpa(dir) => Err(PipelineError::ModelMissing(dir.display().to_string())),
        }
    }

    /// A host running on a thread, with pipes in place of the child's stdin and stdout.
    struct Host {
        calls: io::PipeWriter,
        replies: io::PipeReader,
        thread: thread::JoinHandle<io::Result<()>>,
    }

    impl Host {
        fn start() -> Self {
            let (input, calls) = pipe().unwrap();
            let (replies, output) = pipe().unwrap();
            let thread = thread::spawn(move || serve(input, output, &mut load));
            Self {
                calls,
                replies,
                thread,
            }
        }

        fn load(&mut self, spec: EngineSpec) -> Reply {
            self.call(Call::Load(spec), &[])
        }

        fn call(&mut self, call: Call, samples: &[f32]) -> Reply {
            write_message(&mut self.calls, &call, samples).unwrap();
            self.reply()
        }

        fn reply(&mut self) -> Reply {
            read_message(&mut self.replies).unwrap().unwrap().0
        }
    }

    #[test]
    fn samples_reach_the_engine_exactly_and_results_come_back() {
        let mut host = Host::start();
        assert!(matches!(
            host.load(EngineSpec::Whisper("m".into())),
            Reply::Done
        ));

        let call = Call::Transcribe {
            language: Some("it".into()),
            bias_words: Vec::new(),
        };
        let reply = host.call(call, &[0.0, -0.25]);

        let Reply::Transcript { text, language } = reply else {
            panic!("{reply:?}");
        };
        assert_eq!(text, "2 samples, last -0.25");
        assert_eq!(language.as_deref(), Some("it"));
    }

    #[test]
    fn the_detector_reports_segments_and_whether_speech_is_open() {
        let mut host = Host::start();
        host.load(EngineSpec::Silero("m".into()));

        let reply = host.call(Call::Accept, &[0.1; 512]);

        let Reply::Segments { segments, speaking } = reply else {
            panic!("{reply:?}");
        };
        assert_eq!(segments, [(0, 512)]);
        assert!(speaking);
    }

    #[test]
    fn tokens_stream_before_the_completion() {
        let mut host = Host::start();
        host.load(EngineSpec::Llama {
            file: "m".into(),
            in_ram: false,
        });

        let call = Call::Generate {
            system: String::new(),
            user: "hello".into(),
            examples: Vec::new(),
            max_tokens: 8,
            timeout_ms: Some(1_000),
        };
        let first = host.call(call, &[]);

        assert!(matches!(first, Reply::Token(token) if token == "he"));
        assert!(matches!(host.reply(), Reply::Token(token) if token == "llo"));
        assert!(matches!(host.reply(), Reply::Completion(text) if text == "hello"));
    }

    #[test]
    fn who_speaks_when_comes_back_with_its_times() {
        let mut host = Host::start();
        host.load(EngineSpec::Diarizer {
            segmentation: "s".into(),
            embedding: "e".into(),
        });

        let second = vec![0.0; 16_000];
        let reply = host.call(Call::Diarize, &second);

        let Reply::Turns(turns) = reply else {
            panic!("{reply:?}");
        };
        assert_eq!(turns, [(0, 500, 0), (500, 1_000, 1)]);
    }

    #[test]
    fn engine_errors_keep_their_kind_across_the_process_wall() {
        let mut host = Host::start();

        let Reply::Failed(failure) = host.load(EngineSpec::Sherpa("gone".into())) else {
            panic!("the load must fail");
        };
        assert!(matches!(
            PipelineError::from(failure),
            PipelineError::ModelMissing(model) if model == "gone"
        ));

        host.load(EngineSpec::Silero("m".into()));
        let Reply::Failed(failure) = host.call(Call::Finish, &[]) else {
            panic!("the call must fail");
        };
        assert!(matches!(
            PipelineError::from(failure),
            PipelineError::Timeout
        ));
    }

    #[test]
    fn a_call_for_an_engine_that_is_not_loaded_is_an_error_not_a_crash() {
        let mut host = Host::start();

        assert!(matches!(host.call(Call::Finish, &[]), Reply::Failed(_)));
    }

    #[test]
    fn the_host_ends_when_the_app_closes_the_stream() {
        let Host { calls, thread, .. } = Host::start();

        drop(calls);

        assert!(thread.join().unwrap().is_ok());
    }
}
