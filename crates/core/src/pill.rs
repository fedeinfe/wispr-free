//! The dictation pill's behaviour (spec §1), free of any window system: which state it shows,
//! where its window goes and whether it catches the mouse. The app feeds it one [`Tick`] per poll
//! plus press/release, and applies the resulting [`Frame`] to the real window.

pub mod dock;
pub mod waveform;

use std::time::Duration;

use dock::{Display, Edge, Point, Rect, Size, display_at, dock_origin, nearest_edge};

/// The transparent window. Larger than any capsule so tooltips and toasts fit, in points.
pub const WINDOW_SIZE: Size = Size {
    width: 320.0,
    height: 120.0,
};

/// Gap between the capsule and a side edge, in points.
const EDGE_MARGIN: f64 = 8.0;
/// Gap between the capsule and the bottom edge, in points. The card above it just fits.
const BOTTOM_MARGIN: f64 = 24.0;
/// Extra hover area around the capsule, in points. The resting capsule alone is a thin target.
const HOVER_SLOP: f64 = 6.0;
/// Cursor travel that turns a press into a drag, in points.
const DRAG_THRESHOLD: f64 = 4.0;
/// The suggestion card's size and its gap from the capsule, in points.
const CARD_SIZE: Size = Size {
    width: 236.0,
    height: 64.0,
};
const CARD_GAP: f64 = 8.0;
/// Bar height in the Meeting capsule relative to the Listening one.
const MEETING_BAR_SCALE: f32 = 0.85;

#[derive(Debug, Clone, PartialEq, Default)]
pub enum PillState {
    /// The user turned the pill off. Shortcuts still work.
    Hidden,
    #[default]
    Resting,
    Ready,
    ListeningPtt,
    ListeningHandsFree,
    Processing,
    Error {
        reason: String,
    },
    Meeting,
}

impl PillState {
    /// The capsule's size in points when docked to `edge`.
    pub fn capsule_size(&self, edge: Edge) -> Size {
        let (width, height) = match self {
            PillState::Hidden => (0.0, 0.0),
            PillState::Resting if edge == Edge::Bottom => (32.0, 4.0),
            PillState::Resting => (4.0, 32.0),
            PillState::Ready | PillState::Processing | PillState::Error { .. } => (42.0, 18.0),
            PillState::Meeting => (50.0, 18.0),
            PillState::ListeningPtt => (60.0, 20.0),
            PillState::ListeningHandsFree => (100.0, 24.0),
        };
        Size { width, height }
    }

    pub fn tooltip(&self) -> Option<&str> {
        match self {
            PillState::Error { reason } => Some(reason),
            _ => None,
        }
    }

    /// Full height of the waveform bars relative to the Listening capsule; `None` shows no bars.
    pub fn bar_scale(&self) -> Option<f32> {
        match self {
            PillState::ListeningPtt | PillState::ListeningHandsFree => Some(1.0),
            PillState::Meeting => Some(MEETING_BAR_SCALE),
            _ => None,
        }
    }

    fn is_listening(&self) -> bool {
        matches!(
            self,
            PillState::ListeningPtt | PillState::ListeningHandsFree
        )
    }

    fn capsule(&self, edge: Edge) -> Rect {
        let Size { width, height } = self.capsule_size(edge);
        let (x, y) = match edge {
            Edge::Bottom => (
                (WINDOW_SIZE.width - width) / 2.0,
                WINDOW_SIZE.height - BOTTOM_MARGIN - height,
            ),
            Edge::Left => (EDGE_MARGIN, (WINDOW_SIZE.height - height) / 2.0),
            Edge::Right => (
                WINDOW_SIZE.width - EDGE_MARGIN - width,
                (WINDOW_SIZE.height - height) / 2.0,
            ),
        };
        Rect {
            x,
            y,
            width,
            height,
        }
    }
}

/// The short cue to play on a state change, behind the user's "Sounds" toggle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sound {
    Start,
    Stop,
    Lock,
    Error,
}

/// What the pill asks of the app after a press and release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PillCommand {
    ToggleHandsFree,
    EdgeChanged(Edge),
}

/// One poll of the outside world. `cursor` is `None` where the OS has no global cursor position
/// (Wayland).
#[derive(Debug, Clone, Copy)]
pub struct Tick<'a> {
    pub now: Duration,
    pub cursor: Option<Point>,
    pub displays: &'a [Display],
    /// The Esc key is down right now.
    pub escape: bool,
}

/// What to do with the window after a tick.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub shown: PillState,
    /// Capsule within the window, in points.
    pub capsule: Rect,
    /// The suggestion card within the window, when one is up.
    pub card: Option<Rect>,
    /// New window position, when it changed.
    pub move_to: Option<Point>,
    /// Whether the window catches the mouse. Otherwise clicks pass through to what is below.
    pub hit_test: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Gesture {
    Pressed,
    Dragging,
    Cancelled,
}

#[derive(Debug, Clone, Copy)]
struct Press {
    gesture: Gesture,
    cursor: Point,
    origin: Point,
}

#[derive(Debug)]
pub struct Pill {
    state: PillState,
    edge: Edge,
    hovered: bool,
    press: Option<Press>,
    /// The Show pill setting is off.
    turned_off: bool,
    card: bool,
    hidden_until: Option<Duration>,
    now: Duration,
    cursor: Option<Point>,
    display: Option<Display>,
    origin: Point,
    placed: bool,
}

impl Pill {
    pub fn new(edge: Edge) -> Self {
        Self {
            state: PillState::default(),
            edge,
            hovered: false,
            press: None,
            turned_off: false,
            card: false,
            hidden_until: None,
            now: Duration::ZERO,
            cursor: None,
            display: None,
            origin: Point { x: 0.0, y: 0.0 },
            placed: false,
        }
    }

    pub fn state(&self) -> &PillState {
        &self.state
    }

    pub fn edge(&self) -> Edge {
        self.edge
    }

    /// The window's current position.
    pub fn origin(&self) -> Point {
        self.origin
    }

    pub fn set_state(&mut self, state: PillState) -> Option<Sound> {
        let cue = cue(&self.state, &state);
        self.state = state;
        cue
    }

    /// The Show pill setting. Dictation carries on unseen while off, but an error still shows:
    /// the user has no other way to learn why nothing was typed.
    pub fn set_turned_off(&mut self, turned_off: bool) {
        self.turned_off = turned_off;
    }

    /// Shows or takes down the card that asks the user something. It catches the mouse like the
    /// capsule does, and it shows even with the pill turned off: it only appears on request.
    pub fn set_card(&mut self, shown: bool) {
        self.card = shown;
    }

    /// Hides the pill for a while, whatever its state. Dictation carries on unseen.
    pub fn hide_for(&mut self, duration: Duration) {
        self.hidden_until = Some(self.now + duration);
        self.press = None;
    }

    /// The left button went down on the capsule.
    pub fn press(&mut self) {
        if self.shown() == PillState::Hidden {
            return;
        }
        self.press = Some(Press {
            gesture: Gesture::Pressed,
            cursor: self.cursor.unwrap_or(self.origin),
            origin: self.origin,
        });
    }

    /// The left button came up after [`Pill::press`].
    pub fn release(&mut self) -> Option<PillCommand> {
        match self.press.take()?.gesture {
            Gesture::Pressed => {
                (self.shown() == PillState::Ready).then_some(PillCommand::ToggleHandsFree)
            }
            Gesture::Dragging => {
                let edge = nearest_edge(&self.display?, self.cursor?);
                let changed = edge != self.edge;
                self.edge = edge;
                changed.then_some(PillCommand::EdgeChanged(edge))
            }
            Gesture::Cancelled => None,
        }
    }

    pub fn tick(&mut self, tick: &Tick) -> Frame {
        self.now = tick.now;
        self.cursor = tick.cursor;
        if self.hidden_until.is_some_and(|until| tick.now >= until) {
            self.hidden_until = None;
        }
        let display = match tick.cursor {
            Some(cursor) => display_at(tick.displays, cursor),
            None => tick.displays.first(),
        };
        if let Some(display) = display {
            self.display = Some(*display);
        }

        let move_to = self.place(tick.escape);
        self.hovered = self.cursor_near_capsule(HOVER_SLOP);
        let over_capsule = self.cursor_near_capsule(0.0);
        let shown = self.shown();
        let dragging = self
            .press
            .is_some_and(|press| press.gesture == Gesture::Dragging);
        let card = self.card.then(|| self.card_rect());
        let over_card = card.is_some_and(|card| self.cursor_within(card));
        let hit_test = over_card
            || (card.is_some() && tick.cursor.is_none())
            || (shown != PillState::Hidden && (over_capsule || dragging || tick.cursor.is_none()));
        Frame {
            capsule: shown.capsule(self.edge),
            card,
            shown,
            move_to,
            hit_test,
        }
    }

    /// Moves the window to where the press or the dock wants it. Returns the move, if any.
    fn place(&mut self, escape: bool) -> Option<Point> {
        let units = self.display.map_or(1.0, |d| d.units_per_point);
        if let (Some(press), Some(cursor)) = (self.press.as_mut(), self.cursor) {
            let (dx, dy) = (cursor.x - press.cursor.x, cursor.y - press.cursor.y);
            if press.gesture == Gesture::Pressed && dx.hypot(dy) >= DRAG_THRESHOLD * units {
                press.gesture = Gesture::Dragging;
            }
            if press.gesture == Gesture::Dragging && escape {
                press.gesture = Gesture::Cancelled;
            }
        }
        let target = match (self.press, self.cursor) {
            (Some(press), Some(cursor)) if press.gesture == Gesture::Dragging => Point {
                x: press.origin.x + cursor.x - press.cursor.x,
                y: press.origin.y + cursor.y - press.cursor.y,
            },
            _ => dock_origin(&self.display?, self.edge, WINDOW_SIZE),
        };
        if self.placed && target == self.origin {
            return None;
        }
        self.placed = true;
        self.origin = target;
        Some(target)
    }

    /// Whether the cursor is within `slop` points of the capsule shown now.
    fn cursor_near_capsule(&self, slop: f64) -> bool {
        let capsule = self.shown().capsule(self.edge);
        self.cursor_within(Rect {
            x: capsule.x - slop,
            y: capsule.y - slop,
            width: capsule.width + 2.0 * slop,
            height: capsule.height + 2.0 * slop,
        })
    }

    /// Whether the cursor is inside `area`, a rectangle of the window.
    fn cursor_within(&self, area: Rect) -> bool {
        let Some(cursor) = self.cursor else {
            return false;
        };
        let units = self.display.map_or(1.0, |d| d.units_per_point);
        area.contains(Point {
            x: (cursor.x - self.origin.x) / units,
            y: (cursor.y - self.origin.y) / units,
        })
    }

    /// Beside the capsule, on the side away from the screen edge.
    fn card_rect(&self) -> Rect {
        let Rect { x, y, width, .. } = self.state.capsule(self.edge);
        let (card_x, card_y) = match self.edge {
            Edge::Bottom => (
                (WINDOW_SIZE.width - CARD_SIZE.width) / 2.0,
                y - CARD_GAP - CARD_SIZE.height,
            ),
            Edge::Left => (
                x + width + CARD_GAP,
                (WINDOW_SIZE.height - CARD_SIZE.height) / 2.0,
            ),
            Edge::Right => (
                x - CARD_GAP - CARD_SIZE.width,
                (WINDOW_SIZE.height - CARD_SIZE.height) / 2.0,
            ),
        };
        Rect {
            x: card_x,
            y: card_y,
            width: CARD_SIZE.width,
            height: CARD_SIZE.height,
        }
    }

    fn shown(&self) -> PillState {
        let is_error = matches!(self.state, PillState::Error { .. });
        if self.hidden_until.is_some() || (self.turned_off && !is_error) {
            return PillState::Hidden;
        }
        match &self.state {
            PillState::Resting if self.hovered => PillState::Ready,
            state => state.clone(),
        }
    }
}

fn cue(from: &PillState, to: &PillState) -> Option<Sound> {
    match (from, to) {
        (PillState::Error { .. }, PillState::Error { .. }) => None,
        (_, PillState::Error { .. }) => Some(Sound::Error),
        (PillState::ListeningPtt, PillState::ListeningHandsFree) => Some(Sound::Lock),
        (from, to) if !from.is_listening() && to.is_listening() => Some(Sound::Start),
        (from, to) if from.is_listening() && !to.is_listening() => Some(Sound::Stop),
        _ => None,
    }
}
