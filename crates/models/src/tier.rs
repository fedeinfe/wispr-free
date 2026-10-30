/// The hardware-matched pipeline presets of spec §6. "Custom" is a settings concept (ticket 12),
/// not a catalog tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Tier {
    Lite,
    Balanced,
    High,
}
