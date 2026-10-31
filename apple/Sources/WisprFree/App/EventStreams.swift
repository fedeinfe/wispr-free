import WfCore

/// Where `Shell`'s events come out. Two streams, so a flood of levels can't push out a state
/// change: states are all kept, and levels only ever the newest.
struct EventStreams: Sendable {
    let states: AsyncStream<ShellEvent>
    let levels: AsyncStream<Float>
    let sink: any EventSink

    init() {
        let (states, stateInput) = AsyncStream.makeStream(
            of: ShellEvent.self, bufferingPolicy: .unbounded)
        let (levels, levelInput) = AsyncStream.makeStream(
            of: Float.self, bufferingPolicy: .bufferingNewest(1))
        self.states = states
        self.levels = levels
        sink = StreamSink(states: stateInput, levels: levelInput)
    }
}

/// Runs on Rust's threads, which it holds up for as long as it takes, so it only yields.
private nonisolated final class StreamSink: EventSink {
    private let states: AsyncStream<ShellEvent>.Continuation
    private let levels: AsyncStream<Float>.Continuation

    init(
        states: AsyncStream<ShellEvent>.Continuation,
        levels: AsyncStream<Float>.Continuation
    ) {
        self.states = states
        self.levels = levels
    }

    func onEvent(event: ShellEvent) {
        if case .level(let value) = event {
            levels.yield(value)
        } else {
            states.yield(event)
        }
    }
}
