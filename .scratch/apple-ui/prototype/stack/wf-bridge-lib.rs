//! Throwaway: the real flow behind a UniFFI surface, to prove the stack and weigh it.
use std::path::PathBuf;
use std::sync::Arc;
use wf_core::pill::PillState;
use wf_core::settings::Settings;
use wf_dictation::{Dictation, Event, Parts};
use wf_models::Catalog;

uniffi::setup_scaffolding!("wfcore");

#[derive(uniffi::Enum, Clone, Debug)]
pub enum CoreEvent {
    Pill { state: String },
    Level { value: f32 },
    Toast { text: String },
}

#[uniffi::export(with_foreign)]
pub trait EventSink: Send + Sync {
    fn on_event(&self, event: CoreEvent);
}

#[derive(uniffi::Object)]
pub struct Core {
    dictation: Dictation,
    sink: Arc<dyn EventSink>,
}

#[uniffi::export]
impl Core {
    #[uniffi::constructor]
    pub fn start(data: String, helper: String, sink: Arc<dyn EventSink>) -> Arc<Self> {
        let data = PathBuf::from(data);
        let events = Arc::clone(&sink);
        let on_event: Arc<dyn Fn(Event) + Send + Sync> = Arc::new(move |event| match event {
            Event::Pill(state) => events.on_event(CoreEvent::Pill { state: describe(&state) }),
            Event::Level(level) => events.on_event(CoreEvent::Level { value: level }),
            Event::Toast(text) => events.on_event(CoreEvent::Toast { text }),
            _ => {}
        });
        let parts = Parts::system(
            Settings::default(),
            Arc::new(Catalog::bundled()),
            data.join("models"),
            data.join("history.sqlite3"),
            data.join("meetings"),
            PathBuf::from(helper),
            on_event,
        );
        Arc::new(Self { dictation: Dictation::start(parts), sink })
    }

    /// The whole settings value crosses as records.
    pub fn settings(&self) -> Settings {
        Settings::default()
    }

    pub fn echo(&self, settings: Settings) -> Settings {
        settings
    }

    /// A round trip through real Rust: reads the flow's meetings status on the caller's thread and
    /// answers through the event sink from another thread.
    pub fn pill_clicked(&self) {
        let dictation = self.dictation.clone();
        let sink = Arc::clone(&self.sink);
        std::thread::spawn(move || {
            drop(dictation);
            sink.on_event(CoreEvent::Toast { text: "clicked".to_owned() });
        });
    }
}

fn describe(state: &PillState) -> String {
    format!("{state:?}")
}
