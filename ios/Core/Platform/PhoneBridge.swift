import Foundation

/// This phone's calendar and contacts for the core (`DeviceBridge`). iOS gives apps no access to text messages, so
/// "sms" is never offered. Placeholder until the EventKit / Contacts implementation lands.
final class PhoneBridge: DeviceBridge, @unchecked Sendable {
    static let shared = PhoneBridge()

    static let offered = ["device_calendar", "device_contacts"]

    func services() -> [String] { Self.offered }

    func permitted(service: String) -> Bool { false }

    func calendarEvents(from: Int64, to: Int64, query: String?, limit: UInt32) throws -> [DeviceEvent] {
        throw ForeignError.NeedsUserInteraction
    }

    func calendarCreate(event: NewDeviceEvent) throws -> String { throw ForeignError.NeedsUserInteraction }

    func contactsSearch(query: String, limit: UInt32) throws -> [DeviceContact] { throw ForeignError.NeedsUserInteraction }

    func smsThreads(limit: UInt32) throws -> [SmsThread] { throw ForeignError.Failed(reason: "iOS does not let apps read text messages") }

    func smsMessages(thread: String, limit: UInt32) throws -> [SmsMessage] {
        throw ForeignError.Failed(reason: "iOS does not let apps read text messages")
    }

    func smsSend(to: String, text: String) throws { throw ForeignError.Failed(reason: "iOS does not let apps send text messages") }
}
