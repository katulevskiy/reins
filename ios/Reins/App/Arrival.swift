/// Something that comes once, later (the model, once the core is open), or never (nil). Hooks run with it first, in
/// the order they were added; then whoever waits for it resumes, in the order they began to wait.
@MainActor
final class Arrival<Value: AnyObject> {
    private(set) var settled = false
    private(set) var value: Value?
    private var waiting: [CheckedContinuation<Value?, Never>] = []
    private var hooks: [(Value) -> Void] = []

    /// The value once it came; nil when it never will.
    func wait() async -> Value? {
        if settled { return value }
        return await withCheckedContinuation { waiting.append($0) }
    }

    /// Runs `body` with the value when it comes (now, if it came), before any waiter resumes.
    func whenSettled(_ body: @escaping (Value) -> Void) {
        if settled {
            if let value { body(value) }
        } else {
            hooks.append(body)
        }
    }

    /// Only the first call counts.
    func settle(_ value: Value?) {
        guard !settled else { return }
        settled = true
        self.value = value
        let hooks = hooks
        let waiting = waiting
        self.hooks = []
        self.waiting = []
        if let value { for hook in hooks { hook(value) } }
        for waiter in waiting { waiter.resume(returning: value) }
    }
}
