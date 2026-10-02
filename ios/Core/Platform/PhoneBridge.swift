import Contacts
import EventKit
import Foundation

/// This phone's calendar and contacts for the core (`DeviceBridge`), over EventKit and Contacts. Nothing here decides
/// what an AI may see: every call comes from the core after the user approved it, and each fails with "needs user
/// interaction" while the matching permission is not granted. iOS gives apps no access to text messages, so "sms" is
/// never offered.
///
/// The integration screens ask for access with `PhoneBridge.shared.requestAccess(service:)`.
final class PhoneBridge: DeviceBridge, @unchecked Sendable {
    static let shared = PhoneBridge()

    static let calendar = "device_calendar"
    static let contacts = "device_contacts"
    static let offered = [calendar, contacts]

    private let lock = NSLock()
    private var events = EKEventStore()
    private let people = CNContactStore()

    func services() -> [String] { Self.offered }

    func permitted(service: String) -> Bool {
        switch service {
        case Self.calendar: EKEventStore.authorizationStatus(for: .event) == .fullAccess
        case Self.contacts: Self.contactsAllowed(CNContactStore.authorizationStatus(for: .contacts))
        default: false
        }
    }

    /// Whether the system was asked already for `service` (the screens then send the user to Settings instead).
    func asked(service: String) -> Bool {
        switch service {
        case Self.calendar: EKEventStore.authorizationStatus(for: .event) != .notDetermined
        case Self.contacts: CNContactStore.authorizationStatus(for: .contacts) != .notDetermined
        default: true
        }
    }

    /// Shows the system's prompt for `service` (once; after that only Settings can change it) and says whether access
    /// is granted now.
    func requestAccess(service: String) async -> Bool {
        switch service {
        case Self.calendar:
            let store = lock.withLock { events }
            let granted = (try? await store.requestFullAccessToEvents()) ?? false
            // A store made before access was granted does not always see the calendars.
            if granted { lock.withLock { events = EKEventStore() } }
            return granted
        case Self.contacts:
            let granted = (try? await people.requestAccess(for: .contacts)) ?? false
            return granted || permitted(service: service)
        default:
            return false
        }
    }

    private static func contactsAllowed(_ status: CNAuthorizationStatus) -> Bool {
        status == .authorized || status == .limited
    }

    private func need(_ service: String) throws {
        if !permitted(service: service) { throw ForeignError.NeedsUserInteraction }
    }

    // MARK: Calendar

    func calendarEvents(from: Int64, to: Int64, query: String?, limit: UInt32) throws -> [DeviceEvent] {
        try need(Self.calendar)
        guard to > from, limit > 0 else { return [] }
        let store = lock.withLock { events }
        let start = Date(timeIntervalSince1970: TimeInterval(from))
        let end = Date(timeIntervalSince1970: TimeInterval(to))
        let needle = query?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() ?? ""
        var found: [EKEvent] = []
        // EventKit searches at most four years at once.
        var chunkStart = start
        while chunkStart < end {
            let chunkEnd = min(end, Calendar.current.date(byAdding: .year, value: 4, to: chunkStart) ?? end)
            let predicate = store.predicateForEvents(withStart: chunkStart, end: chunkEnd, calendars: nil)
            found += store.events(matching: predicate)
            chunkStart = chunkEnd
        }
        var seen = Set<String>()
        return found
            .filter { needle.isEmpty || Self.matches($0, needle) }
            .sorted { $0.startDate < $1.startDate }
            .filter { seen.insert("\($0.eventIdentifier ?? "")@\($0.startDate.timeIntervalSince1970)").inserted }
            .prefix(Int(limit))
            .map(Self.event)
    }

    private static func matches(_ e: EKEvent, _ needle: String) -> Bool {
        [e.title, e.location, e.notes].contains { ($0 ?? "").lowercased().contains(needle) }
    }

    private static func event(_ e: EKEvent) -> DeviceEvent {
        let (start, end) = e.isAllDay ? allDayBounds(start: e.startDate, end: e.endDate) : (unix(e.startDate), unix(e.endDate))
        return DeviceEvent(
            id: e.eventIdentifier ?? e.calendarItemIdentifier,
            calendarId: e.calendar?.calendarIdentifier ?? "",
            calendar: e.calendar?.title ?? "",
            title: e.title ?? "",
            start: start,
            end: end,
            allDay: e.isAllDay,
            location: e.location ?? "",
            description: e.notes ?? ""
        )
    }

    /// All-day events travel as UTC midnights, end exclusive (what the date means; the Android provider's form): an
    /// event on 1 October is 1 Oct 00:00 UTC to 2 Oct 00:00 UTC, whatever the phone's time zone.
    static func allDayBounds(start: Date, end: Date, calendar: Calendar = .current) -> (Int64, Int64) {
        var utc = Calendar(identifier: .gregorian)
        utc.timeZone = TimeZone(identifier: "UTC") ?? .gmt
        func utcMidnight(_ date: Date) -> Date {
            let d = calendar.dateComponents([.year, .month, .day], from: date)
            return utc.date(from: d) ?? date
        }
        let first = utcMidnight(start)
        // EventKit ends an all-day event at the last second of its last day.
        let last = utcMidnight(max(start, end.addingTimeInterval(-1)))
        let after = utc.date(byAdding: .day, value: 1, to: last) ?? last
        return (unix(first), unix(max(after, first.addingTimeInterval(86_400))))
    }

    /// The local days of an all-day event that arrives as UTC midnights (end exclusive).
    static func localAllDay(start: Int64, end: Int64, calendar: Calendar = .current) -> (Date, Date) {
        var utc = Calendar(identifier: .gregorian)
        utc.timeZone = TimeZone(identifier: "UTC") ?? .gmt
        func local(_ seconds: Int64) -> Date {
            let d = utc.dateComponents([.year, .month, .day], from: Date(timeIntervalSince1970: TimeInterval(seconds)))
            return calendar.date(from: d) ?? Date(timeIntervalSince1970: TimeInterval(seconds))
        }
        let first = local(start)
        let last = local(max(start, end - 1))
        return (first, max(first, last))
    }

    private static func unix(_ date: Date) -> Int64 { Int64(date.timeIntervalSince1970.rounded(.down)) }

    func calendarCreate(event: NewDeviceEvent) throws -> String {
        try need(Self.calendar)
        let store = lock.withLock { events }
        guard let calendar = store.defaultCalendarForNewEvents ?? store.calendars(for: .event).first(where: \.allowsContentModifications) else {
            throw ForeignError.Failed(reason: "This phone has no calendar that can be written to")
        }
        let e = EKEvent(eventStore: store)
        e.calendar = calendar
        e.title = event.title
        e.isAllDay = event.allDay
        if event.allDay {
            let (start, end) = Self.localAllDay(start: event.start, end: event.end)
            e.startDate = start
            e.endDate = end
        } else {
            e.startDate = Date(timeIntervalSince1970: TimeInterval(event.start))
            e.endDate = Date(timeIntervalSince1970: TimeInterval(max(event.end, event.start)))
        }
        if !event.location.isEmpty { e.location = event.location }
        if !event.description.isEmpty { e.notes = event.description }
        do {
            try store.save(e, span: .thisEvent, commit: true)
        } catch {
            throw ForeignError.Failed(reason: "The calendar did not accept the event: \(error.localizedDescription)")
        }
        guard let id = e.eventIdentifier else { throw ForeignError.Failed(reason: "The calendar did not accept the event") }
        return id
    }

    // MARK: Contacts

    func contactsSearch(query: String, limit: UInt32) throws -> [DeviceContact] {
        try need(Self.contacts)
        let needle = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !needle.isEmpty, limit > 0 else { return [] }
        let keys: [CNKeyDescriptor] = [
            CNContactFormatter.descriptorForRequiredKeys(for: .fullName),
            CNContactOrganizationNameKey as CNKeyDescriptor,
            CNContactEmailAddressesKey as CNKeyDescriptor,
            CNContactPhoneNumbersKey as CNKeyDescriptor,
        ]
        // By name, then by address and number (as the Android provider's filters), each contact once.
        var predicates = [CNContact.predicateForContacts(matchingName: needle)]
        if needle.contains("@") { predicates.append(CNContact.predicateForContacts(matchingEmailAddress: needle)) }
        if needle.contains(where: \.isNumber) {
            predicates.append(CNContact.predicateForContacts(matching: CNPhoneNumber(stringValue: needle)))
        }
        var seen = Set<String>()
        var out: [DeviceContact] = []
        do {
            for predicate in predicates {
                for c in try people.unifiedContacts(matching: predicate, keysToFetch: keys) where seen.insert(c.identifier).inserted {
                    out.append(Self.contact(c))
                    if out.count >= Int(limit) { return out }
                }
            }
        } catch let error as CNError where error.code == .authorizationDenied {
            throw ForeignError.NeedsUserInteraction
        } catch {
            throw ForeignError.Failed(reason: "The phone could not search the contacts: \(error.localizedDescription)")
        }
        return out
    }

    private static func contact(_ c: CNContact) -> DeviceContact {
        var emails: [String] = []
        for e in c.emailAddresses where !emails.contains(e.value as String) { emails.append(e.value as String) }
        var phones: [String] = []
        for p in c.phoneNumbers where !phones.contains(p.value.stringValue) { phones.append(p.value.stringValue) }
        return DeviceContact(
            id: c.identifier,
            name: CNContactFormatter.string(from: c, style: .fullName) ?? "",
            organization: c.organizationName,
            emails: emails,
            phones: phones
        )
    }

    // MARK: Text messages (not on iOS)

    func smsThreads(limit: UInt32) throws -> [SmsThread] { throw ForeignError.Failed(reason: "iOS does not let apps read text messages") }

    func smsMessages(thread: String, limit: UInt32) throws -> [SmsMessage] {
        throw ForeignError.Failed(reason: "iOS does not let apps read text messages")
    }

    func smsSend(to: String, text: String) throws { throw ForeignError.Failed(reason: "iOS does not let apps send text messages") }
}
